//! Page store and the per-frame packet scheduler.
//!
//! Each video frame carries up to 5 teletext packets (one OP-47 SDP). They are filled in
//! three tiers: subtitles, then flash updates, then the carousel. Magazines are sent in
//! parallel mode, so a page ends at the next header of the same magazine: a page in
//! progress is never interleaved with another page of its own magazine. A higher tier
//! that needs a busy magazine preempts the lower tier's page (it is resent later).

use crate::assets::Assets;
use crate::charset::encode_char;
use crate::link::{Frame, Kind};
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
    /// Header control bits the publisher asked for.
    #[serde(default)]
    pub flags: PageFlags,
    /// Send this page at most once per this many seconds in the carousel. For data
    /// pages (DRCS, objects) that rarely change; None = every pass.
    #[serde(default)]
    pub repeat_secs: Option<u32>,
    /// Rows that scroll, and rows that step through frames (see `motion`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tickers: Vec<crate::motion::Ticker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<crate::motion::Animation>,
    #[serde(skip)]
    motion: crate::motion::MotionState,
    /// The DRCS page this page's own characters are on. ttxd lists it in the
    /// magazine's MOT, which is how a decoder finds it.
    #[serde(default)]
    pub drcs_page: Option<PageNo>,
    #[serde(skip)]
    last_sent: Option<SystemTime>,
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
            flags: PageFlags::default(),
            repeat_secs: None,
            tickers: vec![],
            animation: None,
            motion: Default::default(),
            drcs_page: None,
            last_sent: None,
            sub_idx: 0,
            sub_since: None,
        }
    }

    /// Header bits for a transmission of this page, on top of `base`.
    pub(crate) fn header_flags(&self, base: Flags) -> Flags {
        Flags {
            inhibit: base.inhibit || self.flags.inhibit_display,
            suppress_header: base.suppress_header || self.flags.suppress_header,
            ..base
        }
    }

    pub fn expired(&self, now: SystemTime) -> bool {
        self.expires.is_some_and(|e| now >= e)
    }

    pub(crate) fn subcode(&self, idx: usize) -> u16 {
        if self.subpages.len() > 1 {
            idx as u16 + 1
        } else {
            0
        }
    }
}

/// Header control bits a publisher may set on its page. DRCS and object pages, which
/// carry data rather than text, set `inhibit_display` (C10) so a decoder never shows
/// them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PageFlags {
    pub inhibit_display: bool,
    pub suppress_header: bool,
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
    pub service_data_packets: u64,
}

/// Header bits of a subtitle transmission.
pub const SUBTITLE_FLAGS: Flags = Flags {
    erase: true,
    newsflash: false,
    subtitle: true,
    suppress_header: true,
    update: false,
    interrupted: false,
    inhibit: false,
};

/// The page content for up to 3 subtitle lines: boxed, double height, centred, on
/// the bottom rows.
pub fn subtitle_content(lines: &[String]) -> Result<Content, String> {
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
    Ok(content)
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
    /// Pages sent but not yet ended by another header of their magazine.
    unterminated: Vec<PageNo>,
    /// Packet 8/30 contents; sent once a second when set.
    pub service_data: Option<crate::p830::ServiceData>,
    /// Set by the carousel when it wraps round to its first page.
    cycle_start: bool,
    /// Shared Level 2.5 assets, and the pages ttxd builds from them (owner "ttxd").
    pub assets: Assets,
    pub gpop_page: PageNo,
    pub gdrcs_page: PageNo,
    /// Pages whose rows motion changed since the last call to `take_motion_updates`.
    motion_updates: Vec<PageNo>,
}

/// Owner of the pages ttxd builds itself (MOT, GPOP, GDRCS).
pub const TTXD_OWNER: &str = "ttxd";
/// How often the MOT (with the magazine palette) and the global pages go round.
const MOT_REPEAT_SECS: u32 = 5;
const GLOBAL_REPEAT_SECS: u32 = 10;

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
            unterminated: vec![],
            service_data: None,
            cycle_start: false,
            assets: Assets::default(),
            gpop_page: PageNo::new(8, 0xE8),
            gdrcs_page: PageNo::new(8, 0xE9),
            motion_updates: vec![],
        }
    }

    /// Pages publishers may not write: magazine inventory, MOT and filler pages of
    /// every magazine, and the global object and DRCS pages.
    pub fn reserved(&self, no: PageNo) -> bool {
        no.num >= 0xFD || no == self.gpop_page || no == self.gdrcs_page
    }

    /// Pages whose rows tickers or animations changed since the last call.
    pub fn take_motion_updates(&mut self) -> Vec<PageNo> {
        std::mem::take(&mut self.motion_updates)
    }

    /// Replace the shared assets and rebuild the pages made from them. Returns the
    /// pages that changed (true) or went away (false).
    pub fn set_assets(&mut self, assets: Assets) -> Vec<(PageNo, bool)> {
        self.assets = assets;
        self.dirty = true;
        self.rebuild_assets()
    }

    /// What the MOTs are built from: per magazine with pages, each page's link to a
    /// DRCS page of its own (page number, DRCS page, that page's subpage count).
    fn mot_inputs(&self) -> BTreeMap<u8, Vec<(u8, PageNo, u8)>> {
        let mut mags: BTreeMap<u8, Vec<(u8, PageNo, u8)>> = BTreeMap::new();
        for (no, p) in self.pages.iter().filter(|(_, p)| p.owner != TTXD_OWNER) {
            let links = mags.entry(no.mag).or_default();
            if let Some(drcs) = p.drcs_page {
                let subpages = self.pages.get(&drcs).map_or(0, |d| d.subpages.len());
                links.push((no.num, drcs, subpages.min(15) as u8));
            }
        }
        mags
    }

    fn rebuild_assets(&mut self) -> Vec<(PageNo, bool)> {
        let old: Vec<PageNo> = self
            .pages
            .iter()
            .filter(|(_, p)| p.owner == TTXD_OWNER)
            .map(|(no, _)| *no)
            .collect();
        for no in &old {
            self.pages.remove(no);
        }
        let mut new: Vec<(PageNo, Content, u32)> = vec![];
        // A magazine gets a MOT when there are shared assets to point at, or when one
        // of its pages has a DRCS page of its own.
        for (m, links) in self.mot_inputs() {
            if !self.assets.is_empty() || !links.is_empty() {
                let mot = self.assets.mot(self.gpop_page, self.gdrcs_page, &links);
                new.push((PageNo::new(m, 0xFE), mot, MOT_REPEAT_SECS));
            }
        }
        if !self.assets.is_empty() {
            if let Some(c) = self.assets.gpop() {
                new.push((self.gpop_page, c, GLOBAL_REPEAT_SECS));
            }
            if let Some(c) = self.assets.gdrcs() {
                new.push((self.gdrcs_page, c, GLOBAL_REPEAT_SECS));
            }
        }
        let now = SystemTime::now();
        let mut changed: Vec<(PageNo, bool)> = old
            .iter()
            .filter(|no| !new.iter().any(|(n, _, _)| n == *no))
            .map(|no| (*no, false))
            .collect();
        for (no, content, repeat) in new {
            let mut p = StoredPage::new(TTXD_OWNER, vec![content], 10, None, now);
            p.flags.inhibit_display = true;
            p.repeat_secs = Some(repeat);
            self.pages.insert(no, p);
            changed.push((no, true));
        }
        changed
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
        let before = self.mot_inputs();
        self.put_page_only(no, page, flash);
        if self.mot_inputs() != before {
            self.rebuild_assets();
        }
    }

    fn put_page_only(&mut self, no: PageNo, page: StoredPage, flash: bool) {
        if flash {
            let unit = Unit::new(
                Tier::Flash,
                no,
                page.subcode(0),
                page.header_flags(Flags {
                    erase: true,
                    ..Default::default()
                }),
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
        let before = self.mot_inputs();
        let gone = self.pages.remove(&no);
        if self.mot_inputs() != before {
            self.rebuild_assets();
        }
        gone
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
        let changed: Vec<u8> = rows.keys().copied().collect();
        self.flash_changed(no, sub, &changed);
        self.dirty = true;
        Ok(())
    }

    /// Queue a row update (no erase) of `rows` of subpage `sub`, as they are stored.
    fn flash_changed(&mut self, no: PageNo, sub: usize, rows: &[u8]) {
        let Some(page) = self.pages.get(&no) else {
            return;
        };
        let Some(content) = page.subpages.get(sub) else {
            return;
        };
        let mut partial = Content {
            rows: vec![vec![b' '; COLS]; content.rows.len()],
            ..content.clone()
        };
        for &r in rows {
            partial.rows[r as usize - 1] = content.rows[r as usize - 1].clone();
        }
        let mut unit = Unit::new(
            Tier::Flash,
            no,
            page.subcode(sub),
            page.header_flags(Flags {
                update: true,
                ..Default::default()
            }),
            &partial,
            false,
            Some(page.owner.clone()),
        );
        // Only the changed rows (enhancement packets X/26-28 are kept whole).
        unit.rest.retain(|p| {
            let row = crate::ham::unham84(p[0]).unwrap_or(0) >> 3
                | crate::ham::unham84(p[1]).unwrap_or(0) << 1;
            (26..=28).contains(&row) || rows.contains(&row)
        });
        self.queue_flash(unit);
    }

    /// Step tickers and animations that are due, and flash the rows they change.
    fn tick_motion(&mut self, now: SystemTime) {
        let mut flashes = vec![];
        for (no, page) in self.pages.iter_mut() {
            if (page.tickers.is_empty() && page.animation.is_none()) || page.expired(now) {
                continue;
            }
            let rows = crate::motion::advance(
                &page.tickers,
                page.animation.as_ref(),
                &mut page.motion,
                now,
            );
            if rows.is_empty() {
                continue;
            }
            let sub = page.sub_idx % page.subpages.len().max(1);
            let Some(content) = page.subpages.get_mut(sub) else {
                continue;
            };
            for (&r, (codes, g2)) in &rows {
                content.rows[r as usize - 1] = codes.clone();
                content.g2.retain(|&(gr, _, _)| gr != r);
                content.g2.extend(g2.iter().map(|&(c, g)| (r, c, g)));
            }
            content.g2.sort();
            flashes.push((*no, sub, rows.keys().copied().collect::<Vec<u8>>()));
        }
        for (no, sub, rows) in flashes {
            self.flash_changed(no, sub, &rows);
            if !self.motion_updates.contains(&no) {
                self.motion_updates.push(no);
            }
        }
    }

    fn queue_flash(&mut self, unit: Unit) {
        // A newer flash of the same page replaces a queued one.
        self.flashes
            .retain(|u| u.page != unit.page || u.subcode != unit.subcode);
        self.flashes.push_back(unit);
    }

    /// Show subtitle lines (bottom-aligned, double height, boxed). An empty list clears.
    pub fn subtitle(&mut self, no: PageNo, lines: &[String]) -> Result<(), String> {
        let content = subtitle_content(lines)?;
        let unit = Unit::new(Tier::Subtitle, no, 0, SUBTITLE_FLAGS, &content, true, None);
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
            if let (Some(r), Some(last)) = (page.repeat_secs, page.last_sent)
                && now.duration_since(last).unwrap_or_default() < Duration::from_secs(r as u64)
            {
                continue;
            }
            page.last_sent = Some(now);
            let since = *page.sub_since.get_or_insert(now);
            if now.duration_since(since).unwrap_or_default().as_secs() >= page.cycle_secs as u64 {
                page.sub_idx = (page.sub_idx + 1) % page.subpages.len();
                page.sub_since = Some(now);
            }
            page.sub_idx %= page.subpages.len();
            let idx = page.sub_idx;
            if self.cursor.is_none_or(|c| no <= c) {
                self.cycle_start = true;
            }
            self.cursor = Some(no);
            return Some(Unit::new(
                Tier::Carousel,
                no,
                page.subcode(idx),
                page.header_flags(Flags::default()),
                &page.subpages[idx],
                false,
                None,
            ));
        }
        None
    }

    pub(crate) fn header_text(&self, now: DateTime<Local>) -> [u8; 32] {
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
        self.next_link_frame(now).packets()
    }

    /// Fill one frame, with each packet's kind and whether the carousel wrapped.
    pub fn next_link_frame(&mut self, now: SystemTime) -> Frame {
        self.tick_motion(now);
        self.cycle_start = false;
        self.stats.frames += 1;
        for b in self.buckets.values_mut() {
            b.tokens = (b.tokens + b.rate / FRAMES_PER_SEC).min(b.burst);
        }
        for u in &mut self.inflight {
            u.held = false;
        }
        let header = self.header_text(now.into());
        let mut out = vec![];
        if let Some(sd) = &self.service_data
            && self.stats.frames % FRAMES_PER_SEC as u64 == 1
        {
            let local: DateTime<Local> = now.into();
            out.push((
                Kind::ServiceData,
                crate::p830::packet(sd, now.into(), local.offset().local_minus_utc()),
            ));
            self.stats.service_data_packets += 1;
        }
        while out.len() < PACKETS_PER_FRAME {
            match self.pick(now, &header) {
                Some((tier, p)) => {
                    let kind = match tier {
                        Tier::Subtitle => {
                            self.stats.subtitle_packets += 1;
                            Kind::Subtitle
                        }
                        Tier::Flash => {
                            self.stats.flash_packets += 1;
                            Kind::Flash
                        }
                        Tier::Carousel => {
                            self.stats.carousel_packets += 1;
                            Kind::Carousel
                        }
                    };
                    out.push((kind, p));
                }
                None => break,
            }
        }
        if out.is_empty() {
            // Idle: a time-filling header (page xFF) keeps decoders' clocks running.
            let mag = self.filler_mag;
            self.filler_mag = self.filler_mag % 8 + 1;
            if !self.inflight.iter().any(|u| u.page.mag == mag) {
                out.push((
                    Kind::Filler,
                    crate::page::header_packet(
                        PageNo::new(mag, 0xFF),
                        0x3F7F,
                        Flags::default(),
                        &header,
                    ),
                ));
                self.stats.filler_packets += 1;
            }
        }
        Frame {
            cycle_start: self.cycle_start,
            packets: out,
        }
    }

    fn budget_ok(&self, owner: &Option<String>) -> bool {
        match owner.as_ref().and_then(|o| self.buckets.get(o)) {
            Some(b) => b.tokens >= 1.0,
            None => true,
        }
    }

    fn pick(&mut self, now: SystemTime, header: &[u8; 32]) -> Option<(Tier, Packet)> {
        for tier in [Tier::Subtitle, Tier::Flash, Tier::Carousel] {
            // End finished pages ahead of carousel packets.
            if tier == Tier::Carousel
                && let Some(p) = self.terminator(header)
            {
                self.stats.filler_packets += 1;
                return Some((Tier::Carousel, p));
            }
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
                    let page = self.inflight.remove(i).page;
                    self.unterminated.push(page);
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
            self.unterminated.retain(|p| p.mag != unit.page.mag);
            let p = crate::page::header_packet(unit.page, unit.subcode, unit.flags, header);
            unit.header_sent = true;
            unit.held = unit.flags.erase;
            self.spend(&unit.owner);
            if unit.rest.is_empty() {
                self.unterminated.push(unit.page);
            } else {
                self.inflight.push(unit);
            }
            return Some((tier, p));
        }
        None
    }

    /// A page is only complete for a decoder when the next header of its magazine
    /// arrives. If nothing else is about to start in that magazine (a lone subtitle page,
    /// or a magazine holding one page), send a time-filling header (page xFF) to end it.
    /// Skipped when the carousel starts a different page of that magazine right away
    /// (a repeat of the same page number does not end it, at least not for libzvbi).
    fn terminator(&mut self, header: &[u8; 32]) -> Option<Packet> {
        let inflight = &self.inflight;
        self.unterminated
            .retain(|p| !inflight.iter().any(|u| u.page.mag == p.mag));
        let carousel_idle = !self.inflight.iter().any(|u| u.tier == Tier::Carousel);
        let next = if carousel_idle {
            self.peek_carousel()
        } else {
            None
        };
        let i = self
            .unterminated
            .iter()
            .position(|p| !next.is_some_and(|n| n.mag == p.mag && n != *p))?;
        let mag = self.unterminated.remove(i).mag;
        Some(crate::page::header_packet(
            PageNo::new(mag, 0xFF),
            0x3F7F,
            Flags::default(),
            header,
        ))
    }

    /// The page the carousel would start next.
    fn peek_carousel(&self) -> Option<PageNo> {
        let now = SystemTime::now();
        let live = |(k, p): (&PageNo, &StoredPage)| {
            (!p.expired(now) && !p.subpages.is_empty()).then_some(*k)
        };
        match self.cursor {
            Some(c) => self
                .pages
                .range(c..)
                .skip(1)
                .chain(self.pages.range(..=c))
                .find_map(live),
            None => self.pages.iter().find_map(live),
        }
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
    fn link_frames_mark_kinds_and_cycle_start() {
        let mut s = svc_with(&["100", "101"]);
        let now = SystemTime::now();
        let frames: Vec<Frame> = (0..40).map(|_| s.next_link_frame(now)).collect();
        assert!(frames[0].cycle_start);
        let starts = frames.iter().filter(|f| f.cycle_start).count();
        assert!(starts >= 2, "carousel of two pages wraps within 40 frames");
        assert!(
            frames
                .iter()
                .flat_map(|f| &f.packets)
                .all(|(k, _)| matches!(k, Kind::Carousel | Kind::Filler | Kind::ServiceData))
        );
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
    fn a_page_with_its_own_drcs_page_gets_a_mot() {
        let now = SystemTime::now();
        let mut s = Service::new("X", BTreeMap::new());
        let page = |drcs: Option<&str>| {
            let mut p = StoredPage::new("t", vec![Content::default()], 10, None, now);
            p.drcs_page = drcs.map(|d| d.parse().unwrap());
            p
        };
        let mot: PageNo = "1FE".parse().unwrap();
        s.put_page("100".parse().unwrap(), page(None), false);
        assert!(!s.pages.contains_key(&mot));
        s.put_page("100".parse().unwrap(), page(Some("10F")), false);
        let link = |s: &Service| s.pages[&mot].subpages[0].coded[&21][4..8].to_vec();
        let first = link(&s);
        // The DRCS page arriving changes the link's subpage count.
        s.put_page("10F".parse().unwrap(), page(None), false);
        assert_ne!(link(&s), first);
        s.delete_page("100".parse().unwrap());
        assert!(!s.pages.contains_key(&mot));
    }

    #[test]
    fn repeat_secs_limits_how_often_a_data_page_goes_out() {
        let now = SystemTime::now();
        let mut s = svc_with(&["100"]);
        let mut drcs = StoredPage::new("t", vec![Content::default()], 10, None, now);
        drcs.repeat_secs = Some(30);
        s.put_page("10F".parse().unwrap(), drcs, false);
        let out = run(&mut s, 100); // 2 s
        let sent = |p: u8| {
            out.iter()
                .filter(|x| row_of(x) == 0 && page_of(x) == p)
                .count()
        };
        assert_eq!(sent(0x0F), 1);
        assert!(sent(0x00) > 5);
    }

    #[test]
    fn service_data_goes_out_once_a_second() {
        let mut s = svc_with(&["100"]);
        s.service_data = Some(crate::p830::ServiceData {
            initial_page: "100".parse().unwrap(),
            network_id: 0,
            status: "Frikanalen".into(),
        });
        let out = run(&mut s, 150); // 3 s
        assert_eq!(
            out.iter()
                .filter(|p| mag_of(p) == 8 && row_of(p) == 30)
                .count(),
            3
        );
        assert_eq!(s.stats.service_data_packets, 3);
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
    fn lone_page_in_magazine_is_terminated() {
        // 100 and 800 are alone in their magazines: each must be followed by an xFF header
        // of its magazine before it is sent again.
        let mut s = svc_with(&["100", "800"]);
        let out = run(&mut s, 30);
        assert!(s.stats.filler_packets >= 2);
        let headers: Vec<(u8, u8)> = out
            .iter()
            .filter(|p| row_of(p) == 0)
            .map(|p| (mag_of(p), page_of(p)))
            .collect();
        assert_eq!(&headers[..4], &[(1, 0x00), (1, 0xFF), (8, 0x00), (8, 0xFF)]);
    }

    #[test]
    fn single_page_carousel_is_terminated() {
        let mut s = svc_with(&["100"]);
        let out = run(&mut s, 12);
        let headers: Vec<u8> = out.iter().filter(|p| row_of(p) == 0).map(page_of).collect();
        assert_eq!(&headers[..3], &[0x00, 0xFF, 0x00]);
    }

    #[test]
    fn same_magazine_pages_need_no_terminator() {
        let mut s = svc_with(&["100", "101"]);
        run(&mut s, 30);
        assert_eq!(s.stats.filler_packets, 0);
    }

    #[test]
    fn subtitle_is_terminated_promptly() {
        let mut s = svc_with(&["100", "101"]);
        s.subtitle("888".parse().unwrap(), &["Hei".into()]).unwrap();
        let out = run(&mut s, 3);
        let m8: Vec<(u8, u8)> = out
            .iter()
            .filter(|p| mag_of(p) == 8)
            .map(|p| (row_of(p), if row_of(p) == 0 { page_of(p) } else { 0 }))
            .collect();
        // Header 888, its row(s), then the 8FF terminator.
        assert_eq!(m8.first(), Some(&(0, 0x88)));
        assert_eq!(m8.last(), Some(&(0, 0xFF)));
    }

    #[test]
    fn header_text_has_clock() {
        let s = svc_with(&[]);
        let t = s.header_text(Local::now());
        let text = String::from_utf8_lossy(&t);
        assert!(text.starts_with("FRIKANALEN"));
        assert_eq!(&text[26..27], ":");
    }

    #[test]
    fn tickers_flash_their_row_as_they_scroll() {
        let mut s = svc_with(&["200"]);
        let no: PageNo = "200".parse().unwrap();
        s.set_flash_budget("t", 100.0);
        let page = s.pages.get_mut(&no).unwrap();
        page.owner = "t".into();
        page.tickers = vec![crate::motion::Ticker {
            row: 24,
            text: "Siste nytt fra Frikanalen: ".repeat(3),
            prefix: String::new(),
            cells_per_sec: 10.0,
        }];
        let start = SystemTime::now();
        let mut rows24 = std::collections::BTreeSet::new();
        for f in 0..100u64 {
            let now = start + Duration::from_millis(20 * f);
            for p in s.next_frame(now) {
                if row_of(&p) == 24 && mag_of(&p) == 2 {
                    rows24.insert(p[2..].to_vec());
                }
            }
        }
        // About 20 steps in 2 s, each a different row.
        assert!(rows24.len() >= 15, "{} distinct ticker rows", rows24.len());
        assert_eq!(s.take_motion_updates(), vec![no]);
    }
}
