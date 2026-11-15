//! Page store and the per-frame packet scheduler.
//!
//! Each video frame carries up to 5 teletext packets (one OP-47 SDP). They are filled in
//! three tiers: subtitles, then flash updates, then the carousel. Magazines are sent in
//! parallel mode, so a page ends at the next header of the same magazine: a page in
//! progress is never interleaved with another page of its own magazine. A higher tier
//! that needs a busy magazine preempts the lower tier's page (it is resent later).

use crate::charset::encode_char;
use crate::page::{COLS, Content, Flags, Packet, PageNo, page_packets, parse_cells};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, SystemTime};

pub const PACKETS_PER_FRAME: usize = 5;
pub const FRAMES_PER_SEC: f64 = 50.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredPage {
    pub owner: String,
    pub subpages: Vec<Content>,
    /// Seconds each subpage stays before the carousel moves to the next one.
    pub cycle_secs: u32,
    /// After this the page leaves the carousel.
    pub expires: Option<SystemTime>,
    pub updated: SystemTime,
    #[serde(skip)]
    sub_idx: usize,
    #[serde(skip)]
    sub_since: Option<SystemTime>,
}

impl StoredPage {
    pub fn new(
        owner: &str,
        subpages: Vec<Content>,
        cycle_secs: u32,
        ttl: Option<Duration>,
        now: SystemTime,
    ) -> Self {
        StoredPage {
            owner: owner.into(),
            subpages,
            cycle_secs: cycle_secs.max(1),
            expires: ttl.map(|t| now + t),
            updated: now,
            sub_idx: 0,
            sub_since: None,
        }
    }

    pub fn expired(&self, now: SystemTime) -> bool {
        self.expires.is_some_and(|e| now >= e)
    }

    fn subcode(&self, idx: usize) -> u16 {
        if self.subpages.len() > 1 {
            idx as u16 + 1
        } else {
            0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Subtitle,
    Flash,
    Carousel,
}

/// One page transmission: a header (built when sent, so the clock is current) and its packets.
struct Unit {
    tier: Tier,
    page: PageNo,
    subcode: u16,
    flags: Flags,
    header_sent: bool,
    rest: VecDeque<Packet>,
    owner: Option<String>,
    /// Rows wait for the next frame after an erasing header.
    held: bool,
}

impl Unit {
    fn new(
        tier: Tier,
        page: PageNo,
        subcode: u16,
        flags: Flags,
        content: &Content,
        skip_blank: bool,
        owner: Option<String>,
    ) -> Unit {
        // Header placeholder is dropped; the real header is built when sent.
        let mut packets: VecDeque<Packet> =
            page_packets(page, subcode, flags, &[b' '; 32], content, skip_blank).into();
        packets.pop_front();
        Unit {
            tier,
            page,
            subcode,
            flags,
            header_sent: false,
            rest: packets,
            owner,
            held: false,
        }
    }
}

struct Bucket {
    tokens: f64,
    rate: f64,
    burst: f64,
}

#[derive(Default, Clone, Serialize)]
pub struct Stats {
    pub frames: u64,
    pub subtitle_packets: u64,
    pub flash_packets: u64,
    pub carousel_packets: u64,
    pub filler_packets: u64,
}

pub struct Service {
    pub pages: BTreeMap<PageNo, StoredPage>,
    pub service_name: String,
    pub stats: Stats,
    pub dirty: bool,
    subtitles: VecDeque<Unit>,
    flashes: VecDeque<Unit>,
    inflight: Vec<Unit>,
    cursor: Option<PageNo>,
    buckets: BTreeMap<String, Bucket>,
    filler_mag: u8,
}

impl Service {
    pub fn new(service_name: &str, pages: BTreeMap<PageNo, StoredPage>) -> Self {
        Service {
            pages,
            service_name: service_name.into(),
            stats: Stats::default(),
            dirty: false,
            subtitles: VecDeque::new(),
            flashes: VecDeque::new(),
            inflight: vec![],
            cursor: None,
            buckets: BTreeMap::new(),
            filler_mag: 1,
        }
    }

    /// Set a publisher's flash budget in packets per second.
    pub fn set_flash_budget(&mut self, owner: &str, per_sec: f64) {
        let burst = (per_sec * 0.5).max(30.0);
        self.buckets.insert(
            owner.into(),
            Bucket {
                tokens: burst,
                rate: per_sec,
                burst,
            },
        );
    }

    /// Store a page (replacing it). With `flash`, also send it ahead of the carousel.
    pub fn put_page(&mut self, no: PageNo, page: StoredPage, flash: bool) {
        if flash {
            let unit = Unit::new(
                Tier::Flash,
                no,
                page.subcode(0),
                Flags {
                    erase: true,
                    ..Default::default()
                },
                &page.subpages[0],
                true,
                Some(page.owner.clone()),
            );
            self.queue_flash(unit);
        }
        self.pages.insert(no, page);
        self.dirty = true;
    }

    pub fn delete_page(&mut self, no: PageNo) -> Option<StoredPage> {
        self.dirty = true;
        self.flashes.retain(|u| u.page != no);
        self.pages.remove(&no)
    }

    /// Replace some rows of a stored page and flash just those rows (no erase).
    pub fn flash_rows(
        &mut self,
        no: PageNo,
        sub: usize,
        rows: &BTreeMap<u8, String>,
        now: SystemTime,
    ) -> Result<(), String> {
        let page = self.pages.get_mut(&no).ok_or("no such page")?;
        let content = page.subpages.get_mut(sub).ok_or("no such subpage")?;
        for (&r, m) in rows {
            content.set_row(r, m)?;
        }
        page.updated = now;
        let mut partial = Content {
            rows: vec![vec![b' '; COLS]; content.rows.len()],
            g2: content.g2.clone(),
        };
        for &r in rows.keys() {
            partial.rows[r as usize - 1] = content.rows[r as usize - 1].clone();
        }
        let mut unit = Unit::new(
            Tier::Flash,
            no,
            page.subcode(sub),
            Flags {
                update: true,
                ..Default::default()
            },
            &partial,
            false,
            Some(page.owner.clone()),
        );
        // Only the changed rows (X/26 packets are kept whole).
        unit.rest.retain(|p| {
            let row = crate::ham::unham84(p[0]).unwrap_or(0) >> 3
                | crate::ham::unham84(p[1]).unwrap_or(0) << 1;
            row == 26 || rows.contains_key(&row)
        });
        self.queue_flash(unit);
        self.dirty = true;
        Ok(())
    }

    fn queue_flash(&mut self, unit: Unit) {
        // A newer flash of the same page replaces a queued one.
        self.flashes
            .retain(|u| u.page != unit.page || u.subcode != unit.subcode);
        self.flashes.push_back(unit);
    }

    /// Show subtitle lines (bottom-aligned, double height, boxed). An empty list clears.
    pub fn subtitle(&mut self, no: PageNo, lines: &[String]) -> Result<(), String> {
        if lines.len() > 3 {
            return Err("at most 3 subtitle lines".into());
        }
        let mut content = Content::default();
        let first_row = 23 - 2 * lines.len() as u8;
        for (i, text) in lines.iter().enumerate() {
            let (cells, _) = parse_cells(text)?;
            let width = cells.len() + 5; // double height, 2x start box, 2x end box
            if width > COLS {
                return Err(format!("subtitle line too long: {text:?}"));
            }
            let pad = " ".repeat((COLS - width) / 2);
            content.set_row(
                first_row + 2 * i as u8,
                &format!("{pad}{{double}}{{box}}{{box}}{text}{{endbox}}{{endbox}}"),
            )?;
        }
        let flags = Flags {
            erase: true,
            subtitle: true,
            suppress_header: true,
            ..Default::default()
        };
        let unit = Unit::new(Tier::Subtitle, no, 0, flags, &content, true, None);
        // Only the newest cue matters.
        self.subtitles.clear();
        self.subtitles.push_back(unit);
        Ok(())
    }

    /// The next carousel transmission, or None if nothing is due.
    fn next_carousel(&mut self, now: SystemTime) -> Option<Unit> {
        let keys: Vec<PageNo> = match self.cursor {
            Some(c) => self
                .pages
                .range(c..)
                .skip(1)
                .chain(self.pages.range(..=c))
                .map(|(k, _)| *k)
                .collect(),
            None => self.pages.keys().copied().collect(),
        };
        for no in keys {
            let page = self.pages.get_mut(&no)?;
            if page.expired(now) || page.subpages.is_empty() {
                continue;
            }
            let since = *page.sub_since.get_or_insert(now);
            if now.duration_since(since).unwrap_or_default().as_secs() >= page.cycle_secs as u64 {
                page.sub_idx = (page.sub_idx + 1) % page.subpages.len();
                page.sub_since = Some(now);
            }
            page.sub_idx %= page.subpages.len();
            let idx = page.sub_idx;
            self.cursor = Some(no);
            return Some(Unit::new(
                Tier::Carousel,
                no,
                page.subcode(idx),
                Flags::default(),
                &page.subpages[idx],
                false,
                None,
            ));
        }
        None
    }

    fn header_text(&self, now: DateTime<Local>) -> [u8; 32] {
        let clock = now.format("%d.%m %H:%M:%S").to_string();
        let mut t = [b' '; 32];
        for (d, c) in t
            .iter_mut()
            .zip(self.service_name.chars().take(32 - clock.len() - 1))
        {
            *d = encode_char(c).0;
        }
        t[32 - clock.len()..].copy_from_slice(clock.as_bytes());
        t
    }

    /// Fill one frame's worth of packets.
    pub fn next_frame(&mut self, now: SystemTime) -> Vec<Packet> {
        self.stats.frames += 1;
        for b in self.buckets.values_mut() {
            b.tokens = (b.tokens + b.rate / FRAMES_PER_SEC).min(b.burst);
        }
        for u in &mut self.inflight {
            u.held = false;
        }
        let header = self.header_text(now.into());
        let mut out = vec![];
        while out.len() < PACKETS_PER_FRAME {
            match self.pick(now, &header) {
                Some((tier, p)) => {
                    match tier {
                        Tier::Subtitle => self.stats.subtitle_packets += 1,
                        Tier::Flash => self.stats.flash_packets += 1,
                        Tier::Carousel => self.stats.carousel_packets += 1,
                    }
                    out.push(p);
                }
                None => break,
            }
        }
        if out.is_empty() {
            // Idle: a time-filling header (page xFF) keeps decoders' clocks running.
            let mag = self.filler_mag;
            self.filler_mag = self.filler_mag % 8 + 1;
            if !self.inflight.iter().any(|u| u.page.mag == mag) {
                out.push(crate::page::header_packet(
                    PageNo::new(mag, 0xFF),
                    0x3F7F,
                    Flags::default(),
                    &header,
                ));
                self.stats.filler_packets += 1;
            }
        }
        out
    }

    fn budget_ok(&self, owner: &Option<String>) -> bool {
        match owner.as_ref().and_then(|o| self.buckets.get(o)) {
            Some(b) => b.tokens >= 1.0,
            None => true,
        }
    }

    fn pick(&mut self, now: SystemTime, header: &[u8; 32]) -> Option<(Tier, Packet)> {
        for tier in [Tier::Subtitle, Tier::Flash, Tier::Carousel] {
            // Continue a page in progress.
            if let Some(i) = self.inflight.iter().position(|u| u.tier == tier) {
                let u = &self.inflight[i];
                if u.held || (tier == Tier::Flash && !self.budget_ok(&u.owner)) {
                    continue;
                }
                let u = &mut self.inflight[i];
                let p = u.rest.pop_front().expect("inflight units are never empty");
                let owner = u.owner.clone();
                if u.rest.is_empty() {
                    self.inflight.remove(i);
                }
                self.spend(&owner);
                return Some((tier, p));
            }
            // Or start a new one.
            let unit = match tier {
                Tier::Subtitle => self.subtitles.pop_front(),
                Tier::Flash => {
                    let pos = self.flashes.iter().position(|u| {
                        self.budget_ok(&u.owner) && !self.busy_above(u.page.mag, tier)
                    });
                    pos.and_then(|i| self.flashes.remove(i))
                }
                Tier::Carousel => {
                    let mut u = self.next_carousel(now);
                    if u.as_ref()
                        .is_some_and(|u| self.busy_above(u.page.mag, tier))
                    {
                        // Its magazine is busy with a higher tier; try again next slot.
                        self.cursor = self
                            .cursor
                            .and_then(|c| self.pages.range(..c).next_back().map(|(k, _)| *k));
                        u = None;
                    }
                    u
                }
            };
            let Some(mut unit) = unit else { continue };
            // Preempt a lower tier's page in the same magazine.
            self.inflight.retain(|u| u.page.mag != unit.page.mag);
            let p = crate::page::header_packet(unit.page, unit.subcode, unit.flags, header);
            unit.header_sent = true;
            unit.held = unit.flags.erase;
            self.spend(&unit.owner);
            if !unit.rest.is_empty() {
                self.inflight.push(unit);
            }
            return Some((tier, p));
        }
        None
    }

    /// Is `mag` taken by a page of a higher tier than `tier`?
    fn busy_above(&self, mag: u8, tier: Tier) -> bool {
        let rank = |t: Tier| t as u8;
        self.inflight
            .iter()
            .any(|u| u.page.mag == mag && rank(u.tier) < rank(tier))
    }

    fn spend(&mut self, owner: &Option<String>) {
        if let Some(b) = owner.as_ref().and_then(|o| self.buckets.get_mut(o)) {
            b.tokens -= 1.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ham::unham84;

    fn row_of(p: &Packet) -> u8 {
        unham84(p[0]).unwrap() >> 3 | unham84(p[1]).unwrap() << 1
    }
    fn mag_of(p: &Packet) -> u8 {
        match unham84(p[0]).unwrap() & 7 {
            0 => 8,
            m => m,
        }
    }
    fn page_of(p: &Packet) -> u8 {
        unham84(p[2]).unwrap() | unham84(p[3]).unwrap() << 4
    }

    fn svc_with(pages: &[&str]) -> Service {
        let now = SystemTime::now();
        let mut s = Service::new("FRIKANALEN", BTreeMap::new());
        for p in pages {
            let c = Content::from_markup(&[format!("page {p}")]).unwrap();
            s.put_page(
                p.parse().unwrap(),
                StoredPage::new("test", vec![c], 10, None, now),
                false,
            );
        }
        s
    }

    fn run(s: &mut Service, frames: usize) -> Vec<Packet> {
        let now = SystemTime::now();
        (0..frames).flat_map(|_| s.next_frame(now)).collect()
    }

    #[test]
    fn idle_sends_filler_headers() {
        let mut s = svc_with(&[]);
        let out = run(&mut s, 3);
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|p| row_of(p) == 0 && page_of(p) == 0xFF));
    }

    #[test]
    fn carousel_cycles_whole_pages() {
        let mut s = svc_with(&["100", "101"]);
        let out = run(&mut s, 20);
        assert_eq!(out.len(), 100);
        let headers: Vec<u8> = out.iter().filter(|p| row_of(p) == 0).map(page_of).collect();
        assert_eq!(&headers[..4], &[0x00, 0x01, 0x00, 0x01]);
        // Each page is a header plus 24 rows.
        assert_eq!(row_of(&out[25]), 0);
    }

    #[test]
    fn subtitles_preempt_and_hold_rows_until_next_frame() {
        let mut s = svc_with(&["800", "100"]);
        run(&mut s, 1); // carousel page 800 in progress
        s.subtitle("888".parse().unwrap(), &["Hei på deg".into()])
            .unwrap();
        let f1 = s.next_frame(SystemTime::now());
        assert_eq!(mag_of(&f1[0]), 8);
        assert_eq!(row_of(&f1[0]), 0);
        assert_eq!(page_of(&f1[0]), 0x88);
        // No other magazine-8 packet may follow in this frame (the 800 page was dropped, the
        // subtitle rows are held), so the rest is carousel page 100.
        assert!(f1[1..].iter().all(|p| mag_of(p) == 1));
        let f2 = s.next_frame(SystemTime::now());
        assert_eq!(mag_of(&f2[0]), 8);
        assert_eq!(row_of(&f2[0]), 21);
    }

    #[test]
    fn flash_jumps_the_queue() {
        let mut s = svc_with(&["100", "101", "102"]);
        run(&mut s, 2);
        let now = SystemTime::now();
        let rows = BTreeMap::from([(5u8, "Nå på lufta".to_string())]);
        s.flash_rows("102".parse().unwrap(), 0, &rows, now).unwrap();
        let f = s.next_frame(now);
        // Page 102's header and changed row come before the carousel resumes on magazine 1.
        assert_eq!(page_of(&f[0]), 0x02);
        assert!(f.iter().any(|p| row_of(p) == 5));
        assert_eq!(
            s.pages[&"102".parse().unwrap()].subpages[0].preview()[4],
            "Nå på lufta"
        );
    }

    #[test]
    fn flash_budget_limits_a_chatty_publisher() {
        let mut s = svc_with(&["100"]);
        s.set_flash_budget("test", 50.0); // 1 packet per frame, burst 30
        for _ in 0..200 {
            let rows = BTreeMap::from([(1u8, "x".to_string())]);
            s.flash_rows("100".parse().unwrap(), 0, &rows, SystemTime::now())
                .unwrap();
            run(&mut s, 1);
        }
        assert!(s.stats.carousel_packets > 0);
        assert!(s.stats.flash_packets <= 30 + 200);
    }

    #[test]
    fn expired_pages_leave_the_carousel() {
        let now = SystemTime::now();
        let mut s = Service::new("X", BTreeMap::new());
        let page = StoredPage::new(
            "t",
            vec![Content::default()],
            10,
            Some(Duration::from_secs(0)),
            now,
        );
        s.put_page("100".parse().unwrap(), page, false);
        let out = run(&mut s, 2);
        assert!(out.iter().all(|p| page_of(p) == 0xFF));
    }

    #[test]
    fn header_text_has_clock() {
        let s = svc_with(&[]);
        let t = s.header_text(Local::now());
        let text = String::from_utf8_lossy(&t);
        assert!(text.starts_with("FRIKANALEN"));
        assert_eq!(&text[26..27], ":");
    }
}
