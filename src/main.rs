//! ttxd: Frikanalen's teletext server.
//!
//! Keeps a page store, schedules teletext packets frame by frame (subtitles, flash
//! updates, carousel) and hands each frame to ttx-inject, which puts it on SDI.

use anyhow::Context;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use ttxd::assets::Assets;
use ttxd::service::{Service, StoredPage, TTXD_OWNER};
use ttxd::{api, config, feed, link, p830, page};

fn load_snapshot(path: &std::path::Path) -> BTreeMap<page::PageNo, StoredPage> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            tracing::warn!("ignoring unreadable snapshot {}: {e}", path.display());
            BTreeMap::new()
        }),
        Err(_) => BTreeMap::new(),
    }
}

fn assets_path(snapshot: &std::path::Path) -> std::path::PathBuf {
    snapshot.with_file_name("assets.json")
}

fn load_assets(path: &std::path::Path) -> Assets {
    std::fs::read(path)
        .ok()
        .and_then(|b| {
            serde_json::from_slice(&b)
                .map_err(|e| tracing::warn!("ignoring unreadable {}: {e}", path.display()))
                .ok()
        })
        .unwrap_or_default()
}

fn write_atomic(path: &std::path::Path, bytes: Vec<u8>) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Save the publishers' pages (not the ones ttxd builds from the assets) and the assets.
fn save_snapshot(
    path: &std::path::Path,
    pages: &BTreeMap<page::PageNo, StoredPage>,
    assets: &Assets,
) -> anyhow::Result<()> {
    let own: BTreeMap<_, _> = pages
        .iter()
        .filter(|(_, p)| p.owner != TTXD_OWNER)
        .collect();
    write_atomic(path, serde_json::to_vec(&own)?)?;
    write_atomic(&assets_path(path), serde_json::to_vec(assets)?)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/etc/ttxd/ttxd.toml".into());
    let mut cfg: config::Config = toml::from_str(
        &std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?,
    )?;

    cfg.resolve_tokens()?;
    let mut svc = Service::new(&cfg.service_name, load_snapshot(&cfg.snapshot));
    svc.gpop_page = cfg.gpop_page;
    svc.gdrcs_page = cfg.gdrcs_page;
    let assets = load_assets(&assets_path(&cfg.snapshot));
    if let Err(e) = assets.validate() {
        tracing::warn!("ignoring stored assets: {e}");
    } else {
        svc.set_assets(assets);
    }
    svc.service_data = Some(p830::ServiceData {
        initial_page: cfg.initial_page,
        network_id: cfg.network_id,
        status: cfg.service_name.clone(),
    });
    for p in &cfg.publishers {
        svc.set_flash_budget(&p.name, p.flash_per_sec);
    }
    tracing::info!(
        "{} pages loaded from {}",
        svc.pages.len(),
        cfg.snapshot.display()
    );
    let svc = Arc::new(Mutex::new(svc));

    // Frame clock: ttx-inject asks for each frame, clocked by the DeckLink card. While no
    // injector is connected a 50 Hz timer keeps the scheduler (and the T42 stream) going.
    // Each frame's packets also go to the T42 debug stream.
    let (tx, _) = broadcast::channel::<Vec<page::Packet>>(250);
    let injector = Arc::new(AtomicU64::new(0));
    {
        let svc = svc.clone();
        let tx = tx.clone();
        let injector = injector.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                if injector.load(Ordering::Relaxed) != 0 {
                    continue;
                }
                let packets = svc.lock().unwrap().next_frame(SystemTime::now());
                let _ = tx.send(packets);
            }
        });
    }
    if let Some(addr) = &cfg.inject_listen {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!("injector link on {addr}");
        let (svc, tx, injector) = (svc.clone(), tx.clone(), injector.clone());
        tokio::spawn(async move {
            let mut next_id = 0u64;
            loop {
                let Ok((sock, peer)) = listener.accept().await else {
                    continue;
                };
                next_id += 1;
                let id = next_id;
                let (svc, tx, injector) = (svc.clone(), tx.clone(), injector.clone());
                tokio::spawn(async move {
                    let r = serve_injector(sock, id, &svc, &tx, &injector).await;
                    if injector
                        .compare_exchange(id, 0, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                    {
                        let why = r.err().map(|e| format!("{e:#}")).unwrap_or_default();
                        tracing::warn!("injector {peer} gone ({why}); timer takes over");
                    }
                });
            }
        });
    }

    // Snapshot writer.
    {
        let svc = svc.clone();
        let path = cfg.snapshot.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let (pages, assets) = {
                    let mut s = svc.lock().unwrap();
                    if !s.dirty {
                        continue;
                    }
                    s.dirty = false;
                    (s.pages.clone(), s.assets.clone())
                };
                if let Err(e) = save_snapshot(&path, &pages, &assets) {
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

    let feed = feed::Feed::new(cfg.public_max_clients);
    // Rows that tickers and animations change reach the web viewer as page events.
    {
        let (svc, feed) = (svc.clone(), feed.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            loop {
                tick.tick().await;
                let mut s = svc.lock().unwrap();
                for no in s.take_motion_updates() {
                    feed.page(&s, no);
                }
            }
        });
    }
    if let Some(addr) = &cfg.public_listen {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!("public feed on {addr}");
        let app = feed::router(svc.clone(), feed.clone());
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("public feed: {e}");
            }
        });
    }

    let state = api::AppState {
        svc,
        publishers: Arc::new(cfg.publishers),
        feed,
    };
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    tracing::info!("API on {}", cfg.listen);
    axum::serve(listener, api::router(state)).await?;
    Ok(())
}

/// Serve frames to one ttx-inject: a frame for every credit byte it sends.
/// The newest connection owns the frame clock; older ones are closed.
async fn serve_injector(
    mut sock: tokio::net::TcpStream,
    id: u64,
    svc: &Mutex<Service>,
    tx: &broadcast::Sender<Vec<page::Packet>>,
    injector: &AtomicU64,
) -> anyhow::Result<()> {
    sock.set_nodelay(true)?;
    let mut hello = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), sock.read_exact(&mut hello)).await??;
    anyhow::ensure!(&hello == link::HELLO, "not a ttx-inject hello");
    sock.write_all(link::HELLO).await?;
    injector.store(id, Ordering::Relaxed);
    tracing::info!("injector connected: {:?}", sock.peer_addr()?);
    let mut credits = [0u8; 64];
    loop {
        let n = sock.read(&mut credits).await?;
        anyhow::ensure!(n > 0, "closed");
        anyhow::ensure!(
            injector.load(Ordering::Relaxed) == id,
            "replaced by a newer injector"
        );
        let mut out = Vec::new();
        for _ in credits[..n].iter().filter(|&&b| b == link::CREDIT) {
            let frame = svc.lock().unwrap().next_link_frame(SystemTime::now());
            out.extend(frame.encode());
            let _ = tx.send(frame.packets());
        }
        sock.write_all(&out).await?;
    }
}
