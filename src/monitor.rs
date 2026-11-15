//! Return-path monitor: reads the broadcast transport stream (frikanalen.ts) and checks
//! what actually went out. It finds the teletext PID from the PMT, decodes the EN 300 472
//! data units back into WST packets and reports, per minute:
//!
//! - whether the packets arrive intact (continuity, Hamming and parity errors),
//! - which pages were seen,
//! - how evenly the header clock ticks, measured by the stream's own timestamps (PTS),
//!   which is what a player that honours PTS will show,
//! - how far the clock a viewer sees is behind real time (the downstream delay),
//! - how early the encoder muxes teletext and video ahead of their PTS. A player that
//!   shows teletext on arrival instead of at its PTS runs ahead of the picture by the
//!   difference.
//!
//! The header clock is stamped when each frame leaves on SDI, so it follows the output
//! and carries the delay; this module only measures.

use crate::ham::unham84;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitorConfig {
    /// Plain HTTP URL of the transport stream, e.g. "http://simula.frikanalen.tv:9094/frikanalen.ts".
    pub url: String,
    /// Teletext PID; found from the PMT when left out.
    pub pid: Option<u16>,
}

/// The last complete minute, as shown by `GET /v1/status`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    /// Unix time the minute ended.
    pub at: u64,
    pub teletext_pid: Option<u16>,
    pub ts_packets: u64,
    pub teletext_packets: u64,
    pub cc_errors: u64,
    pub hamming_errors: u64,
    pub parity_errors: u64,
    pub headers: u64,
    pub pages_seen: BTreeSet<String>,
    /// Header clock seconds seen change.
    pub ticks: u64,
    /// Shortest and longest time between ticks, in milliseconds of PTS.
    pub tick_min_ms: Option<u64>,
    pub tick_max_ms: Option<u64>,
    /// Ticks more than 250 ms off a second, and ticks that skipped or repeated a second.
    pub uneven_ticks: u64,
    pub skipped_ticks: u64,
    /// When a frame is due on screen (by its PTS) minus the clock it shows: how far the
    /// viewer is behind real time, before the player's own buffer.
    pub delay_ms: Option<i64>,
    /// PTS minus PCR on arrival: how far ahead of its display time each kind is muxed.
    pub teletext_lead_ms: Option<i64>,
    pub video_lead_ms: Option<i64>,
    pub last_clock: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub url: String,
    pub connected: bool,
    pub error: Option<String>,
    pub last_minute: Option<Report>,
}

pub type Shared = Arc<Mutex<Status>>;

const WRAP: i64 = 1 << 33;

/// PTS/PCR difference `a - b` in 90 kHz ticks, across the 33-bit wrap.
fn diff90(a: u64, b: u64) -> i64 {
    let d = (a as i64 - b as i64).rem_euclid(WRAP);
    if d >= WRAP / 2 { d - WRAP } else { d }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[derive(Default)]
struct Acc {
    report: Report,
    tt_lead: (i64, i64),
    video_lead: (i64, i64),
    delays: Vec<i64>,
}

/// The analysis, fed TS packets with their arrival time. Kept free of I/O for tests.
pub struct Analyzer {
    fixed_pid: Option<u16>,
    pmt_pid: Option<u16>,
    tt_pid: Option<u16>,
    video_pid: Option<u16>,
    pcr_pid: Option<u16>,
    cc: Option<u8>,
    pes: Vec<u8>,
    /// Latest PCR (90 kHz base) and arrival times minus PCR over the last 10 s; their
    /// minimum is the least-delayed packet, which maps PCR to wall time.
    pcr: Option<u64>,
    offsets: VecDeque<(f64, f64)>,
    pcr_ext: f64,
    /// Last header clock (seconds of day) and the PTS it changed at.
    last_clock: Option<(u32, Option<u64>)>,
    acc: Acc,
    minute_start: Option<f64>,
    /// Local UTC offset in seconds, for turning the header clock into wall time.
    utc_offset: i64,
}

impl Analyzer {
    pub fn new(fixed_pid: Option<u16>) -> Self {
        Analyzer {
            fixed_pid,
            pmt_pid: None,
            tt_pid: fixed_pid,
            video_pid: None,
            pcr_pid: None,
            cc: None,
            pes: vec![],
            pcr: None,
            offsets: VecDeque::new(),
            pcr_ext: 0.0,
            last_clock: None,
            acc: Acc::default(),
            minute_start: None,
            utc_offset: chrono::Local::now().offset().local_minus_utc() as i64,
        }
    }

    /// Feed one 188-byte packet that arrived at `arrival` (Unix seconds). Returns a report
    /// when a minute is complete.
    pub fn packet(&mut self, p: &[u8], arrival: f64) -> Option<Report> {
        let start = *self.minute_start.get_or_insert(arrival);
        let mut done = None;
        if arrival - start >= 60.0 {
            done = Some(self.finish(arrival));
            self.minute_start = Some(arrival);
        }
        if p.len() != 188 || p[0] != 0x47 {
            return done;
        }
        self.acc.report.ts_packets += 1;
        let pusi = p[1] & 0x40 != 0;
        let pid = u16::from(p[1] & 0x1F) << 8 | u16::from(p[2]);
        let afc = p[3] >> 4 & 3;
        let cc = p[3] & 15;
        let mut payload = 4;
        if afc & 2 != 0 {
            let len = p[4] as usize;
            if Some(pid) == self.pcr_pid && len >= 7 && p[5] & 0x10 != 0 {
                let base = u64::from(p[6]) << 25
                    | u64::from(p[7]) << 17
                    | u64::from(p[8]) << 9
                    | u64::from(p[9]) << 1
                    | u64::from(p[10]) >> 7;
                self.on_pcr(base, arrival);
            }
            payload = 5 + len;
        }
        if afc & 1 == 0 || payload >= 188 {
            return done;
        }
        let data = &p[payload..];
        if pid == 0 && pusi {
            self.on_pat(data);
        } else if Some(pid) == self.pmt_pid && pusi {
            self.on_pmt(data);
        } else if Some(pid) == self.tt_pid {
            if let Some(prev) = self.cc
                && (prev + 1) & 15 != cc
            {
                self.acc.report.cc_errors += 1;
                // The PES in progress lost a piece; drop it.
                if !pusi {
                    self.pes.clear();
                }
            }
            self.cc = Some(cc);
            if pusi {
                if let (Some(pts), Some(pcr)) = (pes_pts(data), self.pcr) {
                    let l = &mut self.acc.tt_lead;
                    l.0 += diff90(pts, pcr);
                    l.1 += 1;
                }
                let pes = std::mem::take(&mut self.pes);
                self.on_teletext_pes(&pes);
            }
            if pusi || !self.pes.is_empty() {
                self.pes.extend_from_slice(data);
            }
        } else if Some(pid) == self.video_pid
            && pusi
            && let (Some(pts), Some(pcr)) = (pes_pts(data), self.pcr)
        {
            let l = &mut self.acc.video_lead;
            l.0 += diff90(pts, pcr);
            l.1 += 1;
        }
        done
    }

    fn on_pcr(&mut self, base: u64, arrival: f64) {
        if let Some(prev) = self.pcr {
            self.pcr_ext += diff90(base, prev) as f64 / 90_000.0;
        }
        self.pcr = Some(base);
        self.offsets.push_back((arrival, arrival - self.pcr_ext));
        while self
            .offsets
            .front()
            .is_some_and(|&(t, _)| arrival - t > 10.0)
        {
            self.offsets.pop_front();
        }
    }

    /// Wall time (Unix seconds) at which a PTS is due, if PCR has been seen.
    fn due(&self, pts: u64) -> Option<f64> {
        let pcr = self.pcr?;
        let base = self.offsets.iter().map(|&(_, o)| o).reduce(f64::min)?;
        Some(base + self.pcr_ext + diff90(pts, pcr) as f64 / 90_000.0)
    }

    fn on_pat(&mut self, d: &[u8]) {
        let Some(s) = section(d) else { return };
        for e in s.as_chunks::<4>().0 {
            let program = u16::from(e[0]) << 8 | u16::from(e[1]);
            if program != 0 {
                self.pmt_pid = Some(u16::from(e[2] & 0x1F) << 8 | u16::from(e[3]));
                return;
            }
        }
    }

    fn on_pmt(&mut self, d: &[u8]) {
        let Some(s) = section(d) else { return };
        if s.len() < 4 {
            return;
        }
        self.pcr_pid = Some(u16::from(s[0] & 0x1F) << 8 | u16::from(s[1]));
        let info = (usize::from(s[2] & 0x0F) << 8 | usize::from(s[3])) + 4;
        let mut i = info;
        while i + 5 <= s.len() {
            let ty = s[i];
            let pid = u16::from(s[i + 1] & 0x1F) << 8 | u16::from(s[i + 2]);
            let len = usize::from(s[i + 3] & 0x0F) << 8 | usize::from(s[i + 4]);
            let desc = s.get(i + 5..i + 5 + len).unwrap_or(&[]);
            if matches!(ty, 0x01 | 0x02 | 0x10 | 0x1B | 0x24) && self.video_pid.is_none() {
                self.video_pid = Some(pid);
            }
            if ty == 0x06 && self.fixed_pid.is_none() && has_descriptor(desc, 0x56) {
                if self.tt_pid != Some(pid) {
                    tracing::info!("monitor: teletext on PID {pid:#x}");
                }
                self.tt_pid = Some(pid);
            }
            i += 5 + len;
        }
    }

    fn on_teletext_pes(&mut self, pes: &[u8]) {
        if pes.len() < 9 || pes[..3] != [0, 0, 1] {
            return;
        }
        let pts = pes_pts(pes);
        let mut i = 9 + pes[8] as usize + 1; // header, then data_identifier
        while i + 2 <= pes.len() {
            let (id, len) = (pes[i], pes[i + 1] as usize);
            let unit = pes.get(i + 2..i + 2 + len);
            i += 2 + len;
            let Some(u) = unit else { break };
            if matches!(id, 0x02 | 0x03) && len == 0x2C && u[1] == 0xE4 {
                let mut t = [0u8; 42];
                for (d, s) in t.iter_mut().zip(&u[2..]) {
                    *d = s.reverse_bits();
                }
                self.on_t42(&t, pts);
            }
        }
    }

    fn on_t42(&mut self, t: &[u8; 42], pts: Option<u64>) {
        let r = &mut self.acc.report;
        r.teletext_packets += 1;
        let (Some(a), Some(b)) = (unham84(t[0]), unham84(t[1])) else {
            r.hamming_errors += 1;
            return;
        };
        let mag = match a & 7 {
            0 => 8,
            m => m,
        };
        if (a >> 3 | b << 1) != 0 {
            return;
        }
        r.headers += 1;
        let (Some(units), Some(tens)) = (unham84(t[2]), unham84(t[3])) else {
            r.hamming_errors += 1;
            return;
        };
        let text = &t[10..42];
        let bad = text.iter().filter(|c| c.count_ones() % 2 == 0).count();
        r.parity_errors += bad as u64;
        if tens << 4 | units != 0xFF {
            r.pages_seen.insert(format!("{mag}{tens:X}{units:X}"));
        }
        if bad > 0 {
            return;
        }
        let clock: Vec<u8> = text[24..].iter().map(|c| c & 0x7F).collect();
        let Some(sod) = parse_hms(&clock) else { return };
        r.last_clock = Some(String::from_utf8_lossy(&clock).into_owned());
        match self.last_clock {
            Some((prev, _)) if prev == sod => {}
            Some((prev, prev_pts)) => {
                r.ticks += 1;
                if (sod + 86_400 - prev) % 86_400 != 1 {
                    r.skipped_ticks += 1;
                }
                if let (Some(a), Some(b)) = (pts, prev_pts) {
                    let ms = diff90(a, b) / 90;
                    if ms >= 0 {
                        let ms = ms as u64;
                        r.tick_min_ms = Some(r.tick_min_ms.map_or(ms, |m| m.min(ms)));
                        r.tick_max_ms = Some(r.tick_max_ms.map_or(ms, |m| m.max(ms)));
                        let secs = u64::from((sod + 86_400 - prev) % 86_400).max(1);
                        if ms.abs_diff(secs * 1000) > 250 {
                            r.uneven_ticks += 1;
                        }
                    }
                }
                // The clock changed between the previous header and this one, so this
                // header is due at most one header spacing after the second began.
                if let Some(due) = pts.and_then(|p| self.due(p)) {
                    let local = due + self.utc_offset as f64;
                    let day = (local / 86_400.0).floor() * 86_400.0;
                    let mut d = local - (day + f64::from(sod));
                    // Around midnight the clock may belong to the other day.
                    if d > 43_200.0 {
                        d -= 86_400.0;
                    } else if d < -43_200.0 {
                        d += 86_400.0;
                    }
                    self.acc.delays.push((d * 1000.0) as i64);
                }
                self.last_clock = Some((sod, pts));
                return;
            }
            None => {}
        }
        if self.last_clock.is_none() {
            self.last_clock = Some((sod, None));
        }
    }

    fn finish(&mut self, now: f64) -> Report {
        let acc = std::mem::take(&mut self.acc);
        let mut r = acc.report;
        r.at = now as u64;
        r.teletext_pid = self.tt_pid;
        let avg = |(s, n): (i64, i64)| (n > 0).then(|| s / n / 90);
        r.teletext_lead_ms = avg(acc.tt_lead);
        r.video_lead_ms = avg(acc.video_lead);
        // The smallest is the one seen closest to the start of its second.
        r.delay_ms = acc.delays.iter().copied().min();
        r
    }
}

fn parse_hms(c: &[u8]) -> Option<u32> {
    let s = std::str::from_utf8(c).ok()?;
    let mut it = s.split(':').map(|x| x.parse::<u32>().ok());
    let (h, m, sec) = (it.next()??, it.next()??, it.next()??);
    (s.len() == 8 && h < 24 && m < 60 && sec < 60).then_some(h * 3600 + m * 60 + sec)
}

/// The body of a PSI section that starts in this payload (after pointer_field, header
/// and before the CRC).
fn section(d: &[u8]) -> Option<&[u8]> {
    let p = *d.first()? as usize;
    let s = d.get(1 + p..)?;
    let len = usize::from(*s.get(1)? & 0x0F) << 8 | usize::from(*s.get(2)?);
    s.get(8..(3 + len).checked_sub(4)?)
}

fn has_descriptor(mut d: &[u8], tag: u8) -> bool {
    while d.len() >= 2 {
        if d[0] == tag {
            return true;
        }
        d = d.get(2 + d[1] as usize..).unwrap_or(&[]);
    }
    false
}

/// The PTS of a PES that starts here.
fn pes_pts(p: &[u8]) -> Option<u64> {
    if p.len() < 14 || p[..3] != [0, 0, 1] || p[7] & 0x80 == 0 {
        return None;
    }
    let b = &p[9..14];
    Some(
        u64::from(b[0] >> 1 & 7) << 30
            | u64::from(b[1]) << 22
            | u64::from(b[2] >> 1) << 15
            | u64::from(b[3]) << 7
            | u64::from(b[4] >> 1),
    )
}

/// Read the stream for ever, reconnecting with backoff, and publish each minute's report.
pub fn run(cfg: MonitorConfig, status: Shared) {
    status.lock().unwrap().url = cfg.url.clone();
    let mut backoff = Duration::from_secs(1);
    loop {
        let mut a = Analyzer::new(cfg.pid);
        let r = stream(&cfg.url, |p, t| {
            if let Some(rep) = a.packet(p, t) {
                tracing::info!(
                    "monitor: {} teletext packets, {} CC / {} Hamming / {} parity errors, ticks {}-{} ms ({} uneven, {} skipped), delay {:?} ms, lead teletext {:?} / video {:?} ms",
                    rep.teletext_packets,
                    rep.cc_errors,
                    rep.hamming_errors,
                    rep.parity_errors,
                    rep.tick_min_ms.unwrap_or(0),
                    rep.tick_max_ms.unwrap_or(0),
                    rep.uneven_ticks,
                    rep.skipped_ticks,
                    rep.delay_ms,
                    rep.teletext_lead_ms,
                    rep.video_lead_ms,
                );
                let mut s = status.lock().unwrap();
                s.connected = true;
                s.error = None;
                s.last_minute = Some(rep);
            }
        });
        let why = match r {
            Ok(()) => "stream ended".to_string(),
            Err(e) => format!("{e:#}"),
        };
        tracing::warn!("monitor: {why}; retrying in {backoff:?}");
        {
            let mut s = status.lock().unwrap();
            s.connected = false;
            s.error = Some(why);
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

/// GET a plain-HTTP URL and call `f` with each TS packet and its arrival time.
pub fn stream(url: &str, mut f: impl FnMut(&[u8], f64)) -> anyhow::Result<()> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow::anyhow!("only http:// URLs"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let addr = if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:80")
    };
    let sa = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("{host} has no address"))?;
    let mut sock = TcpStream::connect_timeout(&sa, Duration::from_secs(10))?;
    sock.set_read_timeout(Some(Duration::from_secs(10)))?;
    write!(
        sock,
        "GET /{path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: ttxd-monitor\r\nConnection: close\r\n\r\n"
    )?;
    let mut r = BufReader::with_capacity(64 * 1024, sock);
    let mut line = String::new();
    r.read_line(&mut line)?;
    if line.split_whitespace().nth(1) != Some("200") {
        anyhow::bail!("HTTP said {}", line.trim());
    }
    let mut chunked = false;
    loop {
        line.clear();
        r.read_line(&mut line)?;
        let l = line.trim().to_ascii_lowercase();
        if l.is_empty() {
            break;
        }
        if l.starts_with("transfer-encoding:") && l.contains("chunked") {
            chunked = true;
        }
    }
    tracing::info!("monitor: reading {url}");
    let mut body: Box<dyn Read> = if chunked {
        Box::new(Chunked { r, left: 0 })
    } else {
        Box::new(r)
    };
    let mut buf = vec![0u8; 188 * 64];
    let mut have = 0;
    loop {
        let n = body.read(&mut buf[have..])?;
        if n == 0 {
            return Ok(());
        }
        let t = now_secs();
        have += n;
        let mut i = 0;
        while i + 188 <= have {
            if buf[i] != 0x47 {
                i += 1; // resync
                continue;
            }
            f(&buf[i..i + 188], t);
            i += 188;
        }
        buf.copy_within(i..have, 0);
        have -= i;
    }
}

struct Chunked<R> {
    r: R,
    left: usize,
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            let mut line = String::new();
            self.r.read_line(&mut line)?;
            if line.trim().is_empty() {
                line.clear();
                self.r.read_line(&mut line)?;
            }
            let size = line.trim().split(';').next().unwrap_or("");
            self.left = usize::from_str_radix(size, 16)
                .map_err(|_| std::io::Error::other("bad chunk size"))?;
            if self.left == 0 {
                return Ok(0);
            }
        }
        let want = out.len().min(self.left);
        let n = self.r.read(&mut out[..want])?;
        self.left -= n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ham::{ham84, parity};

    /// A tiny muxer: PAT, PMT (video 0x100, teletext 0x96 with descriptor 0x56, PCR on
    /// video), a teletext PES per frame and PCR packets.
    struct Mux {
        cc: [u8; 3],
        out: Vec<(Vec<u8>, f64)>,
    }

    impl Mux {
        fn ts(&mut self, pid: u16, pusi: bool, pcr: Option<u64>, payload: &[u8], t: f64) {
            let k = match pid {
                0x96 => 0,
                0x100 => 1,
                _ => 2,
            };
            let mut af = vec![];
            if let Some(b) = pcr {
                af.extend([
                    0x10,
                    (b >> 25) as u8,
                    (b >> 17) as u8,
                    (b >> 9) as u8,
                    (b >> 1) as u8,
                    ((b & 1) << 7) as u8 | 0x7E,
                    0,
                ]);
            }
            let room = 184 - payload.len();
            let has_af = pcr.is_some() || room > 0;
            let mut p = vec![
                0x47,
                (pusi as u8) << 6 | (pid >> 8) as u8,
                pid as u8,
                if has_af { 0x30 } else { 0x10 } | self.cc[k],
            ];
            self.cc[k] = (self.cc[k] + 1) & 15;
            if has_af {
                assert!(room > af.len());
                if room >= 2 && af.is_empty() {
                    af.push(0);
                }
                af.resize(room - 1, 0xFF);
                p.push((room - 1) as u8);
                p.extend(af);
            }
            p.extend_from_slice(payload);
            assert_eq!(p.len(), 188);
            self.out.push((p, t));
        }
    }

    fn psi(table: u8, body: &[u8]) -> Vec<u8> {
        let len = body.len() + 5 + 4;
        let mut s = vec![
            0,
            table,
            0xB0 | (len >> 8) as u8,
            len as u8,
            0,
            1,
            0xC1,
            0,
            0,
        ];
        s.extend_from_slice(body);
        s.extend([0, 0, 0, 0]);
        s
    }

    fn pts_bytes(pts: u64) -> [u8; 5] {
        [
            0x21 | ((pts >> 29) & 0xE) as u8,
            (pts >> 22) as u8,
            ((pts >> 14) as u8) | 1,
            (pts >> 7) as u8,
            ((pts << 1) as u8) | 1,
        ]
    }

    fn header(page: &str, clock: &str) -> [u8; 42] {
        let p: crate::page::PageNo = page.parse().unwrap();
        let mut t = [parity(b' '); 42];
        t[0] = ham84(p.mag & 7);
        t[1] = ham84(0);
        t[2] = ham84(p.num & 15);
        t[3] = ham84(p.num >> 4);
        for b in &mut t[4..10] {
            *b = ham84(0);
        }
        for (d, c) in t[34..].iter_mut().zip(clock.bytes()) {
            *d = parity(c);
        }
        t
    }

    fn pes(pts: u64, units: &[[u8; 42]]) -> Vec<u8> {
        let mut v = vec![0, 0, 1, 0xBD, 0, 0, 0x84, 0x80, 0x24];
        v.extend(pts_bytes(pts));
        v.resize(9 + 0x24, 0xFF);
        v.push(0x10);
        for u in units {
            v.extend([0x02, 0x2C, 0xE7, 0xE4]);
            v.extend(u.iter().map(|b| b.reverse_bits()));
        }
        let len = v.len() - 6;
        v[4] = (len >> 8) as u8;
        v[5] = len as u8;
        v
    }

    /// `secs` of 50 Hz frames starting at local 12:00:00 today; a header on every
    /// frame, with the clock stamped `delay` before the frame is due on screen.
    fn run(secs: u32, delay: f64, lose_one: bool) -> (Analyzer, Vec<Report>) {
        let mut m = Mux {
            cc: [0; 3],
            out: vec![],
        };
        let off = chrono::Local::now().offset().local_minus_utc() as f64;
        let day = ((now_secs() + off) / 86_400.0).floor() * 86_400.0 - off;
        let t0 = day + 12.0 * 3600.0; // wall time of the first frame's clock
        let pat = psi(0, &[0, 1, 0xE0, 0x20]);
        let pmt = psi(
            2,
            &[
                0xE1, 0x00, 0xF0, 0x00, // PCR PID 0x100, no program info
                0x1B, 0xE1, 0x00, 0xF0, 0x00, // H.264 video
                0x06, 0xE0, 0x96, 0xF0, 0x07, 0x56, 0x05, b'n', b'o', b'r', 0x09, 0x00,
            ],
        );
        for f in 0..secs * 50 {
            let clock_t = t0 + f64::from(f) * 0.02;
            let due = clock_t + delay;
            let arrival = due - 2.0; // muxed 2 s ahead
            let pcr = (f as u64 * 1800 + 1_000_000) % (1 << 33);
            let pts = pcr + 180_000;
            if f % 25 == 0 {
                m.ts(0, true, None, &pat, arrival);
                m.ts(0x20, true, None, &pmt, arrival);
            }
            m.ts(0x100, false, Some(pcr), &[], arrival);
            let s = (clock_t - day + 0.000_001) as u32;
            let clock = format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60);
            let data = pes(pts, &[header("100", &clock)]);
            for (i, c) in data.chunks(184).enumerate() {
                if lose_one && f == 100 && i == 0 {
                    m.cc[0] = (m.cc[0] + 1) & 15;
                    continue;
                }
                m.ts(0x96, i == 0, None, c, arrival);
            }
        }
        // A final PES start flushes the last one.
        let pts = (u64::from(secs) * 50 * 1800 + 1_000_000 + 180_000) % (1 << 33);
        m.ts(0x96, true, None, &pes(pts, &[]), m.out.last().unwrap().1);
        let mut a = Analyzer::new(None);
        let mut reps = vec![];
        for (p, t) in &m.out {
            reps.extend(a.packet(p, *t));
        }
        let last = m.out.last().unwrap().1 + 60.0;
        reps.push(a.finish(last));
        (a, reps)
    }

    #[test]
    fn even_clock_and_delay_are_measured() {
        let (a, reps) = run(10, 3.5, false);
        assert_eq!(a.tt_pid, Some(0x96));
        let r = reps.last().unwrap();
        assert_eq!(r.hamming_errors + r.parity_errors + r.cc_errors, 0);
        assert_eq!(r.headers, 500);
        assert_eq!(r.ticks, 9);
        assert_eq!((r.tick_min_ms, r.tick_max_ms), (Some(1000), Some(1000)));
        assert_eq!(r.uneven_ticks + r.skipped_ticks, 0);
        let d = r.delay_ms.unwrap();
        assert!((3450..=3550).contains(&d), "delay {d}");
        assert_eq!(r.teletext_lead_ms, Some(2000));
        assert!(r.pages_seen.contains("100"));
    }

    #[test]
    fn lost_packets_are_counted() {
        let (_, reps) = run(4, 1.0, true);
        assert_eq!(reps.last().unwrap().cc_errors, 1);
    }

    #[test]
    fn clock_parse() {
        assert_eq!(parse_hms(b"12:34:56"), Some(45_296));
        assert_eq!(parse_hms(b"24:00:00"), None);
        assert_eq!(parse_hms(b"1:2:3"), None);
    }
}
