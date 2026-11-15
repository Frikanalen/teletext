//! ttx-inject: puts ttxd's teletext on SDI as OP-47.
//!
//! The DeckLink card is the clock. On every frame it takes the next frame from a small
//! buffer and asks ttxd for one more (see `ttxd::link`). If ttxd stops answering, it
//! replays the last complete carousel cycle it saw, with the header clock kept current,
//! so the output never goes stale.

use anyhow::Context;
use serde::Deserialize;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ttxd::link::{self, Frame, Kind};
use ttxd::page::Packet;

const FRAME: Duration = Duration::from_millis(20);

#[derive(Debug, Deserialize)]
struct Config {
    /// ttxd's injector link, e.g. "192.168.3.94:4243".
    ttxd: String,
    /// DeckLink device, e.g. "DeckLink Duo (2)". Without it a 50 Hz timer stands in (for
    /// testing; nothing goes to SDI).
    decklink_device: Option<String>,
    #[serde(default = "default_vanc_line")]
    #[cfg_attr(not(feature = "decklink"), allow(dead_code))]
    vanc_line: u32,
    /// OP-47 packets per frame, 1-3 (5 teletext lines each). The encoding appliance
    /// must be in multi-packet mode for more than 1.
    #[serde(default = "default_op47_packets")]
    op47_packets: usize,
    /// Wrap each OP-47 packet in an ANC Multipacket (DID 0x43, SDID 0x03), for an
    /// encoder in its "multi-packet" OP-47 mode.
    #[serde(default)]
    #[cfg_attr(not(feature = "decklink"), allow(dead_code))]
    op47_multipacket: bool,
    /// Line numbers put in the OP-47 descriptors, 5 per packet. Defaults to 7 upwards.
    #[serde(default)]
    #[cfg_attr(not(feature = "decklink"), allow(dead_code))]
    op47_lines: Option<Vec<u8>>,
    /// Frames asked for ahead of the card. Each is 20 ms of delay.
    #[serde(default = "default_buffer")]
    buffer_frames: usize,
    /// Start replaying after this long without frames from ttxd.
    #[serde(default = "default_replay_after")]
    replay_after_ms: u64,
    /// Longest carousel cycle kept for replay.
    #[serde(default = "default_max_cycle")]
    max_cycle_secs: u64,
}

fn default_vanc_line() -> u32 {
    9
}
fn default_op47_packets() -> usize {
    1
}
fn default_buffer() -> usize {
    2
}
fn default_replay_after() -> u64 {
    500
}
fn default_max_cycle() -> u64 {
    120
}

/// Frames received from ttxd, waiting for the card.
type Buffer = Arc<Mutex<VecDeque<Frame>>>;

/// Runs on the card's thread, once per frame.
struct Output {
    buf: Buffer,
    credits: Sender<()>,
    replay_after: u32,
    max_cycle: usize,
    starved: u32,
    replaying: bool,
    /// The cycle being recorded (carousel packets only), and the last complete one.
    rec: Option<Vec<Frame>>,
    cycle: Vec<Frame>,
    pos: usize,
    stats: Stats,
}

#[derive(Default)]
struct Stats {
    live: u64,
    replayed: u64,
    empty: u64,
    since: Option<Instant>,
}

impl Output {
    fn next(&mut self) -> Vec<Packet> {
        self.report();
        let frame = self.buf.lock().unwrap().pop_front();
        match frame {
            Some(f) => {
                let _ = self.credits.send(());
                if self.replaying {
                    tracing::info!("ttxd is back; live again");
                    self.replaying = false;
                }
                self.starved = 0;
                self.record(&f);
                self.stats.live += 1;
                f.packets()
            }
            None => {
                self.starved = self.starved.saturating_add(1);
                if !self.replaying && self.starved >= self.replay_after && !self.cycle.is_empty() {
                    tracing::warn!(
                        "no frames from ttxd for {} ms; replaying the last cycle ({} frames)",
                        self.starved * 20,
                        self.cycle.len()
                    );
                    self.replaying = true;
                    self.pos = 0;
                }
                if self.replaying {
                    self.stats.replayed += 1;
                    let f = &self.cycle[self.pos % self.cycle.len()];
                    self.pos = (self.pos + 1) % self.cycle.len();
                    let clock = chrono::Local::now().format("%d.%m %H:%M:%S").to_string();
                    f.packets()
                        .into_iter()
                        .map(|p| set_clock(p, &clock))
                        .collect()
                } else {
                    self.stats.empty += 1;
                    vec![]
                }
            }
        }
    }

    /// Keep the carousel's packets from one cycle start to the next. Subtitles, flashes
    /// and 8/30 are left out: replaying them would show stale cues and a stale clock.
    fn record(&mut self, f: &Frame) {
        if f.cycle_start {
            if let Some(rec) = self.rec.take()
                && !rec.is_empty()
            {
                self.cycle = rec;
            }
            self.rec = Some(vec![]);
        }
        let Some(rec) = &mut self.rec else { return };
        let packets: Vec<_> = f
            .packets
            .iter()
            .filter(|(k, _)| matches!(k, Kind::Carousel | Kind::Filler))
            .copied()
            .collect();
        if !packets.is_empty() {
            rec.push(Frame {
                cycle_start: false,
                packets,
            });
        }
        if rec.len() > self.max_cycle {
            tracing::warn!("carousel cycle longer than max_cycle_secs; not recording it");
            self.rec = None;
        }
    }

    fn report(&mut self) {
        let since = *self.stats.since.get_or_insert_with(Instant::now);
        if since.elapsed() >= Duration::from_secs(60) {
            let s = &self.stats;
            tracing::info!(
                "last minute: {} live, {} replayed, {} empty frames; replay cycle {} frames",
                s.live,
                s.replayed,
                s.empty,
                self.cycle.len()
            );
            self.stats = Stats::default();
        }
    }
}

/// Put the current date and time in a page header's clock field (its last 14 characters).
fn set_clock(mut p: Packet, clock: &str) -> Packet {
    let row =
        ttxd::ham::unham84(p[0]).unwrap_or(0xF) >> 3 | ttxd::ham::unham84(p[1]).unwrap_or(0xF) << 1;
    if row == 0 && clock.len() == 14 {
        for (d, c) in p[28..42].iter_mut().zip(clock.bytes()) {
            *d = ttxd::ham::parity(c);
        }
    }
    p
}

/// Keep a connection to ttxd, filling `buf`. Credits from the card go out as requests;
/// `credit` is the credit byte (the number of OP-47 packets per frame).
fn network(addr: String, buf: Buffer, credits: Receiver<()>, ahead: usize, credit: u8) {
    let mut last_err = String::new();
    loop {
        match connect(&addr) {
            Ok(sock) => {
                last_err.clear();
                tracing::info!("connected to ttxd at {addr}");
                if let Err(e) = serve(sock, &buf, &credits, ahead, credit) {
                    tracing::warn!("lost ttxd: {e:#}");
                }
            }
            Err(e) => {
                let e = format!("{e:#}");
                if e != last_err {
                    tracing::warn!("cannot reach ttxd at {addr}: {e}");
                    last_err = e;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn connect(addr: &str) -> anyhow::Result<TcpStream> {
    let sa = addr
        .to_socket_addrs()?
        .next()
        .context("no address for ttxd")?;
    let mut sock = TcpStream::connect_timeout(&sa, Duration::from_secs(2))?;
    sock.set_nodelay(true)?;
    sock.set_read_timeout(Some(Duration::from_secs(2)))?;
    sock.write_all(link::HELLO)?;
    let mut hello = [0u8; 4];
    sock.read_exact(&mut hello)?;
    anyhow::ensure!(&hello == link::HELLO, "not ttxd");
    Ok(sock)
}

fn serve(
    sock: TcpStream,
    buf: &Buffer,
    credits: &Receiver<()>,
    ahead: usize,
    credit: u8,
) -> anyhow::Result<()> {
    buf.lock().unwrap().clear();
    while credits.try_recv().is_ok() {}
    let alive = Arc::new(AtomicBool::new(true));
    let reader = {
        let mut sock = sock.try_clone()?;
        let (buf, alive) = (buf.clone(), alive.clone());
        std::thread::spawn(move || {
            let r = loop {
                match Frame::read_from(&mut sock) {
                    Ok(f) => buf.lock().unwrap().push_back(f),
                    Err(e) => break e,
                }
            };
            alive.store(false, Ordering::Relaxed);
            r
        })
    };
    let mut w = sock;
    let r = (|| -> anyhow::Result<()> {
        w.write_all(&vec![credit; ahead])?;
        while alive.load(Ordering::Relaxed) {
            match credits.recv_timeout(Duration::from_millis(200)) {
                Ok(()) => w.write_all(&[credit])?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => anyhow::bail!("output stopped"),
            }
        }
        Ok(())
    })();
    let _ = w.shutdown(std::net::Shutdown::Both);
    let read_err = reader.join();
    r?;
    match read_err {
        Ok(e) => Err(e.into()),
        Err(_) => anyhow::bail!("reader thread panicked"),
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/etc/ttx-inject/ttx-inject.toml".into());
    let cfg: Config = toml::from_str(
        &std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?,
    )?;
    anyhow::ensure!(cfg.buffer_frames >= 1, "buffer_frames must be at least 1");
    anyhow::ensure!(
        (1..=link::MAX_SDPS).contains(&cfg.op47_packets),
        "op47_packets is 1-{}",
        link::MAX_SDPS
    );
    let op47_lines: Vec<u8> = cfg
        .op47_lines
        .clone()
        .unwrap_or_else(|| (7..7 + 5 * cfg.op47_packets as u8).collect());
    anyhow::ensure!(
        op47_lines.len() == 5 * cfg.op47_packets,
        "op47_lines needs 5 lines per OP-47 packet"
    );
    let credit = cfg.op47_packets as u8;

    let buf: Buffer = Arc::default();
    let (credit_tx, credit_rx) = mpsc::channel();
    {
        let (addr, buf, ahead) = (cfg.ttxd.clone(), buf.clone(), cfg.buffer_frames);
        std::thread::spawn(move || network(addr, buf, credit_rx, ahead, credit));
    }
    let mut out = Output {
        buf,
        credits: credit_tx,
        replay_after: (cfg.replay_after_ms / 20).max(1) as u32,
        max_cycle: (cfg.max_cycle_secs * 50) as usize,
        starved: 0,
        replaying: false,
        rec: None,
        cycle: vec![],
        pos: 0,
        stats: Stats::default(),
    };

    match &cfg.decklink_device {
        #[cfg(feature = "decklink")]
        Some(dev) => {
            ttxd::decklink::start(
                dev,
                cfg.vanc_line,
                op47_lines,
                cfg.op47_multipacket,
                Box::new(move || out.next()),
            )?;
            tracing::info!(
                "OP-47 on {dev:?}, VANC line {}, {} packet(s) per frame{}",
                cfg.vanc_line,
                cfg.op47_packets,
                if cfg.op47_multipacket {
                    ", as multipackets"
                } else {
                    ""
                }
            );
            loop {
                std::thread::park();
            }
        }
        #[cfg(not(feature = "decklink"))]
        Some(_) => anyhow::bail!(
            "decklink_device is set but ttx-inject was built without the decklink feature"
        ),
        None => {
            tracing::warn!("no decklink_device: using a 50 Hz timer, nothing goes to SDI");
            let mut next = Instant::now();
            loop {
                out.next();
                next += FRAME;
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ttxd::page::{Flags, PageNo, header_packet};

    fn output() -> (Output, Buffer, Receiver<()>) {
        let buf: Buffer = Arc::default();
        let (tx, rx) = mpsc::channel();
        let out = Output {
            buf: buf.clone(),
            credits: tx,
            replay_after: 3,
            max_cycle: 100,
            starved: 0,
            replaying: false,
            rec: None,
            cycle: vec![],
            pos: 0,
            stats: Stats::default(),
        };
        (out, buf, rx)
    }

    fn frame(start: bool, kind: Kind, b: u8) -> Frame {
        Frame {
            cycle_start: start,
            packets: vec![(kind, [b; 42])],
        }
    }

    #[test]
    fn replays_the_last_cycle_without_subtitles() {
        let (mut out, buf, rx) = output();
        buf.lock().unwrap().extend([
            frame(true, Kind::Carousel, 1),
            frame(false, Kind::Subtitle, 9),
            frame(false, Kind::Carousel, 2),
            frame(true, Kind::Carousel, 3),
        ]);
        for _ in 0..4 {
            out.next();
        }
        assert_eq!(rx.try_iter().count(), 4, "one credit per frame taken");
        // Starved: empty frames until replay_after, then the cycle 1, 2 again and again.
        let got: Vec<Vec<Packet>> = (0..7).map(|_| out.next()).collect();
        assert!(got[0].is_empty() && got[1].is_empty());
        assert_eq!(got[2], vec![[1; 42]]);
        assert_eq!(got[3], vec![[2; 42]]);
        assert_eq!(got[4], vec![[1; 42]]);
        assert_eq!(rx.try_iter().count(), 0, "no credits while starved");
        // Live again as soon as a frame arrives.
        buf.lock().unwrap().push_back(frame(false, Kind::Flash, 7));
        assert_eq!(out.next(), vec![[7; 42]]);
        assert!(!out.replaying);
    }

    #[test]
    fn replayed_headers_get_the_current_clock() {
        let h = header_packet(PageNo::new(1, 0), 0, Flags::default(), &[b'x'; 32]);
        let p = set_clock(h, "15.11 18:30:00");
        let text: String = p[28..42].iter().map(|b| (b & 0x7F) as char).collect();
        assert_eq!(text, "15.11 18:30:00");
        // Rows are left alone.
        let row = ttxd::page::row_packet(1, 5, &[b'a'; 40]);
        assert_eq!(set_clock(row, "15.11 18:30:00"), row);
    }
}
