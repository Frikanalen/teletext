//! ttxd: Frikanalen's teletext server.
//!
//! Keeps a page store, schedules teletext packets frame by frame (subtitles, flash
//! updates, carousel) and builds one OP-47 packet per frame for the SDI output.

mod api;
mod charset;
mod config;
#[cfg(feature = "decklink")]
mod decklink;
mod ham;
mod op47;
mod page;
mod service;

use anyhow::Context;
use service::{Service, StoredPage};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;

fn load_snapshot(path: &std::path::Path) -> BTreeMap<page::PageNo, StoredPage> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            tracing::warn!("ignoring unreadable snapshot {}: {e}", path.display());
            BTreeMap::new()
        }),
        Err(_) => BTreeMap::new(),
    }
}

fn save_snapshot(
    path: &std::path::Path,
    pages: &BTreeMap<page::PageNo, StoredPage>,
) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(pages)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/etc/ttxd/ttxd.toml".into());
    let cfg: config::Config = toml::from_str(
        &std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?,
    )?;

    let mut svc = Service::new(&cfg.service_name, load_snapshot(&cfg.snapshot));
    for p in &cfg.publishers {
        svc.set_flash_budget(&p.name, p.flash_per_sec);
    }
    tracing::info!(
        "{} pages loaded from {}",
        svc.pages.len(),
        cfg.snapshot.display()
    );
    let svc = Arc::new(Mutex::new(svc));

    // Frame clock: the DeckLink card when configured, else a 50 Hz timer.
    // Each frame's packets also go to the T42 debug stream.
    let (tx, _) = broadcast::channel::<Vec<page::Packet>>(250);
    match &cfg.decklink_device {
        #[cfg(feature = "decklink")]
        Some(dev) => {
            decklink::start(dev, cfg.vanc_line, cfg.op47_lines, svc.clone(), tx.clone())?;
            tracing::info!("OP-47 on {dev:?}, VANC line {}", cfg.vanc_line);
        }
        #[cfg(not(feature = "decklink"))]
        Some(_) => {
            anyhow::bail!("decklink_device is set but ttxd was built without the decklink feature")
        }
        None => {
            tracing::warn!("no decklink_device: using a 50 Hz timer, nothing goes to SDI");
            let svc = svc.clone();
            let tx = tx.clone();
            let lines = cfg.op47_lines;
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_millis(20));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut seq: u16 = 0;
                loop {
                    tick.tick().await;
                    let packets = svc.lock().unwrap().next_frame(SystemTime::now());
                    let _sdp = op47::sdp(&packets, &lines, seq);
                    seq = seq.wrapping_add(1);
                    let _ = tx.send(packets);
                }
            });
        }
    }

    // Snapshot writer.
    {
        let svc = svc.clone();
        let path = cfg.snapshot.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let pages = {
                    let mut s = svc.lock().unwrap();
                    if !s.dirty {
                        continue;
                    }
                    s.dirty = false;
                    s.pages.clone()
                };
                if let Err(e) = save_snapshot(&path, &pages) {
                    tracing::error!("saving snapshot: {e:#}");
                }
            }
        });
    }

    if let Some(addr) = &cfg.t42_listen {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!("T42 debug stream on {addr}");
        let tx = tx.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, peer)) = listener.accept().await else {
                    continue;
                };
                let mut rx = tx.subscribe();
                tokio::spawn(async move {
                    tracing::info!("T42 client {peer}");
                    while let Ok(packets) = rx.recv().await {
                        for p in packets {
                            if sock.write_all(&p).await.is_err() {
                                return;
                            }
                        }
                    }
                });
            }
        });
    }

    let state = api::AppState {
        svc,
        publishers: Arc::new(cfg.publishers),
    };
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    tracing::info!("API on {}", cfg.listen);
    axum::serve(listener, api::router(state)).await?;
    Ok(())
}
