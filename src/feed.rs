//! Public read-only feed for the web teletext viewer.
//!
//!   GET /teletext/feed   Server-Sent Events
//!
//! The feed carries pages as the T42 packets a decoder would receive, so the browser
//! runs a real decoder (Hamming, parity, X/26-28, DRCS) on the same bytes a TV gets.
//! A packet is 42 bytes (magazine/row address, then 40 data bytes); each subpage or
//! subtitle is sent as the base64 of its packets one after the other.
//!
//! Events, each with a JSON `data` line:
//!
//! - `snapshot` `{t, pages: [Page]}`: every stored page. Sent first, and again if a
//!   client falls too far behind.
//! - `page` `Page`: a page was stored or had rows flashed.
//! - `delete` `{page}`
//! - `subtitle` `{page, t, packets}`: a subtitle cue (empty rows clear the screen).
//!
//! `Page` is `{page, cycle_secs, expires, subpages: [base64]}`. Times `t` and `expires`
//! are Unix milliseconds; `t` lets the player hold a cue until its delayed stream
//! reaches that moment.
//!
//! This listener has no write routes. It is meant to be exposed through the public
//! ingress, apart from the publisher API.

use crate::page::{Flags, Packet, PageNo, page_packets};
use crate::service::{SUBTITLE_FLAGS, Service, StoredPage, subtitle_content};
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::stream::{self, Stream};
use serde_json::{Value, json};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};

/// Events kept for clients that are slow to read; one that falls further behind gets a
/// fresh snapshot.
const BACKLOG: usize = 64;

#[derive(Clone)]
pub struct Feed {
    tx: broadcast::Sender<(&'static str, String)>,
    slots: Arc<Semaphore>,
}

impl Feed {
    pub fn new(max_clients: usize) -> Self {
        Feed {
            tx: broadcast::channel(BACKLOG).0,
            slots: Arc::new(Semaphore::new(max_clients)),
        }
    }

    fn send(&self, event: &'static str, data: Value) {
        // No receivers is normal: nobody is watching.
        let _ = self.tx.send((event, data.to_string()));
    }

    /// Call after a page was stored or flashed, with the service still locked.
    pub fn page(&self, svc: &Service, no: PageNo) {
        if self.tx.receiver_count() == 0 {
            return;
        }
        if let Some(p) = svc.pages.get(&no) {
            self.send("page", page_json(svc, no, p));
        }
    }

    pub fn delete(&self, no: PageNo) {
        self.send("delete", json!({ "page": no.to_string() }));
    }

    pub fn subtitle(&self, svc: &Service, no: PageNo, lines: &[String]) {
        if self.tx.receiver_count() == 0 {
            return;
        }
        let Ok(content) = subtitle_content(lines) else {
            return;
        };
        let header = svc.header_text(SystemTime::now().into());
        let packets = page_packets(no, 0, SUBTITLE_FLAGS, &header, &content, true);
        self.send(
            "subtitle",
            json!({ "page": no.to_string(), "t": millis(SystemTime::now()), "packets": b64(&packets) }),
        );
    }
}

fn millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn page_json(svc: &Service, no: PageNo, p: &StoredPage) -> Value {
    let header = svc.header_text(SystemTime::now().into());
    let subpages: Vec<String> = p
        .subpages
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let flags = p.header_flags(Flags {
                erase: true,
                ..Default::default()
            });
            b64(&page_packets(no, p.subcode(i), flags, &header, c, true))
        })
        .collect();
    json!({
        "page": no.to_string(),
        "cycle_secs": p.cycle_secs,
        "expires": p.expires.map(millis),
        "subpages": subpages,
    })
}

fn snapshot(svc: &Service) -> String {
    let now = SystemTime::now();
    let pages: Vec<Value> = svc
        .pages
        .iter()
        .filter(|(_, p)| !p.expired(now))
        .map(|(no, p)| page_json(svc, *no, p))
        .collect();
    json!({ "t": millis(now), "pages": pages }).to_string()
}

fn b64(packets: &[Packet]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = packets.iter().flatten().copied().collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for ch in bytes.chunks(3) {
        let n = (ch[0] as u32) << 16
            | (*ch.get(1).unwrap_or(&0) as u32) << 8
            | *ch.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= ch.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[derive(Clone)]
struct FeedState {
    svc: Arc<Mutex<Service>>,
    feed: Feed,
}

pub fn router(svc: Arc<Mutex<Service>>, feed: Feed) -> Router {
    Router::new()
        .route("/teletext/feed", get(sse))
        .with_state(FeedState { svc, feed })
}

struct Client {
    rx: broadcast::Receiver<(&'static str, String)>,
    svc: Arc<Mutex<Service>>,
    resync: bool,
    _slot: OwnedSemaphorePermit,
}

fn events(c: Client) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold(c, |mut c| async move {
        if c.resync {
            c.resync = false;
            let data = snapshot(&c.svc.lock().unwrap());
            return Some((Ok(Event::default().event("snapshot").data(data)), c));
        }
        match c.rx.recv().await {
            Ok((name, data)) => Some((Ok(Event::default().event(name).data(data)), c)),
            Err(broadcast::error::RecvError::Lagged(_)) => {
                c.rx = c.rx.resubscribe();
                let data = snapshot(&c.svc.lock().unwrap());
                Some((Ok(Event::default().event("snapshot").data(data)), c))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    })
}

async fn sse(State(st): State<FeedState>) -> Response {
    let Ok(slot) = st.feed.slots.clone().try_acquire_owned() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "too many viewers").into_response();
    };
    // Subscribe before the snapshot is taken, so no change falls in between.
    let client = Client {
        rx: st.feed.tx.subscribe(),
        svc: st.svc.clone(),
        resync: true,
        _slot: slot,
    };
    let mut res = Sse::new(events(client))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
        .into_response();
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // Stop nginx-style proxies from buffering the stream.
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn base64_matches_rfc4648() {
        let enc = |b: &[u8]| {
            // Pad to a whole packet and compare the prefix.
            let mut p = [0u8; 42];
            p[..b.len()].copy_from_slice(b);
            b64(&[p])
        };
        assert!(enc(b"Man").starts_with("TWFu"));
        assert_eq!(b64(&[[0xFF; 42]]).len(), 56);
        assert!(b64(&[[0xFF; 42]]).starts_with("////"));
    }

    #[test]
    fn snapshot_lists_pages_as_packets() {
        let mut svc = Service::new("FRIKANALEN", BTreeMap::new());
        let c = crate::page::Content::from_markup(&["Hei".to_string()]).unwrap();
        let no: PageNo = "100".parse().unwrap();
        svc.put_page(
            no,
            StoredPage::new("t", vec![c], 10, None, SystemTime::now()),
            false,
        );
        let v: Value = serde_json::from_str(&snapshot(&svc)).unwrap();
        let pages = v["pages"].as_array().unwrap();
        assert_eq!(pages[0]["page"], "100");
        // Header + one row = 2 packets = 84 bytes = 112 base64 chars.
        assert_eq!(pages[0]["subpages"][0].as_str().unwrap().len(), 112);
    }
}
