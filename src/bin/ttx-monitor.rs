//! ttx-monitor: the return-path monitor on its own, for a quick look at a stream.
//!
//!   ttx-monitor http://simula.frikanalen.tv:9094/frikanalen.ts [teletext PID]
//!
//! Prints a JSON report per minute (see `ttxd::monitor::Report`).

use ttxd::monitor::{Analyzer, stream};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: ttx-monitor <http url> [pid]"))?;
    let pid = args.next().map(|p| {
        let p = p.trim_start_matches("0x");
        u16::from_str_radix(p, 16).expect("PID in hex")
    });
    let mut a = Analyzer::new(pid);
    stream(&url, |p, t| {
        if let Some(r) = a.packet(p, t) {
            println!("{}", serde_json::to_string_pretty(&r).unwrap());
        }
    })
}
