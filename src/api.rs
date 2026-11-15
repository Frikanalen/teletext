//! HTTP API for publishers.
//!
//!   GET    /v1/status
//!   GET    /v1/pages                     list pages
//!   GET    /v1/pages/{page}              page with a plain-text preview
//!   GET    /v1/pages/{page}.png          drawn preview ?level=1|1.5|2.5&subpage=N&scale=1-4
//!   PUT    /v1/pages/{page}              store a page   {subpages:[[row,...] | {rows, raw_rows, x26, packets, links, palette, cells, place},...], cycle_secs?, ttl_secs?, flash?, flags?, tickers?, animation?}
//!   DELETE /v1/pages/{page}
//!   POST   /v1/pages/{page}/flash        replace rows now {rows:{"5":"text"}, subpage?}
//!   GET    /v1/assets                    shared Level 2.5 assets and where they ended up
//!   PUT    /v1/assets                    {palette?, glyphs?:[{name, rows}], objects?:[{name, type, x26}]}
//!   PUT    /v1/status_display            8/30 status line {text}; "" resets to the service name
//!   POST   /v1/subtitles                 show lines     {lines:["...",...], page?}; [] clears
//!
//! Writes need `Authorization: Bearer <token>`; each publisher may only touch its own pages.

use crate::assets::{Assets, Layout, Palette};
use crate::config::Publisher;
use crate::enhance::{Cell, Enhancements, Links, Place};
use crate::page::{Content, PageNo, Triplet};
use crate::render::Level;
use crate::service::{PageFlags, Service, StoredPage};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct AppState {
    pub svc: Arc<Mutex<Service>>,
    pub publishers: Arc<Vec<Publisher>>,
    pub feed: crate::feed::Feed,
}

type ApiError = (StatusCode, Json<Value>);
type ApiResult = Result<Json<Value>, ApiError>;

fn err(code: StatusCode, msg: impl ToString) -> ApiError {
    (code, Json(json!({ "error": msg.to_string() })))
}

fn parse_page(s: &str) -> Result<PageNo, ApiError> {
    s.parse().map_err(|e| err(StatusCode::BAD_REQUEST, e))
}

fn auth<'a>(st: &'a AppState, h: &HeaderMap) -> Result<&'a Publisher, ApiError> {
    let token = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    st.publishers
        .iter()
        .find(|p| p.token == token)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "unknown token"))
}

fn auth_page<'a>(st: &'a AppState, h: &HeaderMap, page: PageNo) -> Result<&'a Publisher, ApiError> {
    let p = auth(st, h)?;
    if st.svc.lock().unwrap().reserved(page) {
        return Err(err(
            StatusCode::FORBIDDEN,
            format!("page {page} is reserved for ttxd"),
        ));
    }
    if !p.may_write(page) {
        return Err(err(
            StatusCode::FORBIDDEN,
            format!("{} may not write page {page}", p.name),
        ));
    }
    Ok(p)
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/status", get(status))
        .route("/v1/pages", get(list_pages))
        .route(
            "/v1/pages/{page}",
            get(get_page).put(put_page).delete(delete_page),
        )
        .route("/v1/pages/{page}/flash", post(flash))
        .route("/v1/subtitles", post(subtitles))
        .route("/v1/status_display", put(status_display))
        .route("/v1/assets", get(get_assets).put(put_assets))
        .with_state(state)
}

async fn status(State(st): State<AppState>) -> Json<Value> {
    let svc = st.svc.lock().unwrap();
    Json(json!({ "pages": svc.pages.len(), "stats": svc.stats }))
}

async fn list_pages(State(st): State<AppState>) -> Json<Value> {
    let svc = st.svc.lock().unwrap();
    let now = SystemTime::now();
    let pages: Vec<Value> = svc
        .pages
        .iter()
        .map(|(no, p)| {
            json!({
                "page": no.to_string(), "owner": p.owner, "subpages": p.subpages.len(),
                "updated": secs(p.updated), "expires": p.expires.map(secs), "expired": p.expired(now),
            })
        })
        .collect();
    Json(json!({ "pages": pages }))
}

#[derive(Deserialize)]
struct PreviewQuery {
    level: Option<String>,
    #[serde(default)]
    subpage: usize,
    scale: Option<usize>,
}

/// `GET /v1/pages/{page}.png`: the page drawn as a decoder would show it.
fn preview_png(st: &AppState, page: &str, q: &PreviewQuery) -> Result<Response, ApiError> {
    let no = parse_page(page)?;
    let level: Level = q
        .level
        .as_deref()
        .unwrap_or("2.5")
        .parse()
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let scale = q.scale.unwrap_or(1).clamp(1, 4);
    let svc = st.svc.lock().unwrap();
    let row0 = crate::render::header_row(no, &svc.header_text(chrono::Local::now()));
    let rgb = crate::render::render(&svc, no, q.subpage, level, &row0)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "no such page or subpage"))?;
    drop(svc);
    let body = crate::render::png(&rgb, scale);
    Ok(([(header::CONTENT_TYPE, "image/png")], body).into_response())
}

async fn get_page(
    State(st): State<AppState>,
    Path(page): Path<String>,
    Query(q): Query<PreviewQuery>,
) -> Result<Response, ApiError> {
    if let Some(p) = page.strip_suffix(".png") {
        return preview_png(&st, p, &q);
    }
    get_page_json(&st, &page).map(IntoResponse::into_response)
}

fn get_page_json(st: &AppState, page: &str) -> ApiResult {
    let no = parse_page(page)?;
    let svc = st.svc.lock().unwrap();
    let p = svc
        .pages
        .get(&no)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "no such page"))?;
    Ok(Json(json!({
        "page": no.to_string(), "owner": p.owner, "cycle_secs": p.cycle_secs,
        "updated": secs(p.updated), "expires": p.expires.map(secs),
        "preview": p.subpages.iter().map(|c| c.preview()).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct PutPage {
    subpages: Vec<SubpageIn>,
    #[serde(default = "default_cycle")]
    cycle_secs: u32,
    ttl_secs: Option<u64>,
    #[serde(default)]
    flash: bool,
    #[serde(default)]
    flags: PageFlags,
    /// Send at most once per this many seconds (for data pages such as DRCS).
    repeat_secs: Option<u32>,
    /// Rows that scroll right to left, driven by ttxd.
    #[serde(default)]
    tickers: Vec<crate::motion::Ticker>,
    /// Rows that step through frames, driven by ttxd.
    animation: Option<crate::motion::Animation>,
    /// The DRCS page with this page's own characters, for the magazine's MOT.
    drcs_page: Option<PageNo>,
}

/// A subpage: a list of markup rows (first entry = row 1), or an object that can also
/// carry raw rows and Level 2.5 enhancement data.
#[derive(Deserialize)]
#[serde(try_from = "Value")]
enum SubpageIn {
    Rows(Vec<String>),
    Full(FullSubpage),
}

// Not `#[serde(untagged)]`: that hides the real error behind "did not match any variant".
impl TryFrom<Value> for SubpageIn {
    type Error = String;
    fn try_from(v: Value) -> Result<Self, String> {
        match v {
            Value::Array(_) => serde_json::from_value(v)
                .map(SubpageIn::Rows)
                .map_err(|e| format!("subpage rows: {e}")),
            Value::Object(_) => serde_json::from_value(v)
                .map(SubpageIn::Full)
                .map_err(|e| format!("subpage: {e}")),
            _ => Err("a subpage is a list of rows or an object".into()),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FullSubpage {
    #[serde(default)]
    rows: Vec<String>,
    /// Row number ("1".."24") to 40 raw 7-bit codes, replacing that row's markup.
    /// (String keys: numeric map keys do not survive serde's untagged enums.)
    #[serde(default)]
    raw_rows: BTreeMap<String, Vec<u8>>,
    /// X/26 enhancement triplets [address, mode, data], in active-position order.
    #[serde(default)]
    x26: Vec<Triplet>,
    /// X/27 and X/28 packets.
    #[serde(default)]
    packets: Vec<PacketIn>,
    /// Declarative enhancements, compiled by ttxd (see `enhance`).
    #[serde(default)]
    links: Option<Links>,
    #[serde(default)]
    palette: Option<Palette>,
    #[serde(default)]
    cells: Vec<Cell>,
    #[serde(default)]
    place: Vec<Place>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PacketIn {
    packet: u8,
    designation: u8,
    triplets: Option<Vec<Triplet>>,
    raw: Option<Vec<u8>>,
}

impl SubpageIn {
    fn content(&self, page: PageNo, layout: &Layout) -> Result<Content, String> {
        let full = match self {
            SubpageIn::Rows(rows) => return Content::from_markup(rows),
            SubpageIn::Full(f) => f,
        };
        let mut c = Content::from_markup(&full.rows)?;
        for (row, codes) in &full.raw_rows {
            let row = row.parse().map_err(|_| format!("bad row number {row:?}"))?;
            c.set_raw_row(row, codes)?;
        }
        for p in &full.packets {
            c.add_packet(
                p.packet,
                p.designation,
                p.triplets.as_deref(),
                p.raw.as_deref(),
            )?;
        }
        c.set_x26(full.x26.clone())?;
        let e = Enhancements {
            links: full.links.clone(),
            palette: full.palette.clone(),
            cells: full.cells.clone(),
            place: full.place.clone(),
        };
        e.apply(page, layout, &mut c, !full.x26.is_empty())?;
        Ok(c)
    }
}

fn default_cycle() -> u32 {
    10
}

async fn put_page(
    State(st): State<AppState>,
    Path(page): Path<String>,
    h: HeaderMap,
    Json(body): Json<PutPage>,
) -> ApiResult {
    let no = parse_page(&page)?;
    let publisher = auth_page(&st, &h, no)?;
    if body.subpages.is_empty() || body.subpages.len() > 99 {
        return Err(err(StatusCode::BAD_REQUEST, "need 1..=99 subpages"));
    }
    let layout = st.svc.lock().unwrap().assets.validate().unwrap_or_default();
    let subpages = body
        .subpages
        .iter()
        .map(|s| s.content(no, &layout))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let now = SystemTime::now();
    let mut stored = StoredPage::new(
        &publisher.name,
        subpages,
        body.cycle_secs,
        body.ttl_secs.map(Duration::from_secs),
        now,
    );
    stored.flags = body.flags;
    stored.repeat_secs = body.repeat_secs;
    for t in &body.tickers {
        t.validate().map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    }
    if let Some(a) = &body.animation {
        a.validate().map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    }
    stored.tickers = body.tickers;
    stored.animation = body.animation;
    stored.drcs_page = body.drcs_page;
    let mut svc = st.svc.lock().unwrap();
    svc.put_page(no, stored, body.flash);
    st.feed.page(&svc, no);
    Ok(Json(json!({ "page": no.to_string(), "ok": true })))
}

async fn delete_page(
    State(st): State<AppState>,
    Path(page): Path<String>,
    h: HeaderMap,
) -> ApiResult {
    let no = parse_page(&page)?;
    auth_page(&st, &h, no)?;
    match st.svc.lock().unwrap().delete_page(no) {
        Some(_) => {
            st.feed.delete(no);
            Ok(Json(json!({ "ok": true })))
        }
        None => Err(err(StatusCode::NOT_FOUND, "no such page")),
    }
}

#[derive(Deserialize)]
struct Flash {
    rows: BTreeMap<u8, String>,
    #[serde(default)]
    subpage: usize,
}

async fn flash(
    State(st): State<AppState>,
    Path(page): Path<String>,
    h: HeaderMap,
    Json(body): Json<Flash>,
) -> ApiResult {
    let no = parse_page(&page)?;
    auth_page(&st, &h, no)?;
    let mut svc = st.svc.lock().unwrap();
    svc.flash_rows(no, body.subpage, &body.rows, SystemTime::now())
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    st.feed.page(&svc, no);
    Ok(Json(json!({ "ok": true })))
}

fn assets_json(svc: &Service) -> Value {
    let layout = svc.assets.validate().unwrap_or_default();
    json!({
        "assets": svc.assets,
        "glyphs": layout.glyphs,
        "objects": layout.objects,
        "pages": { "gpop": svc.gpop_page.to_string(), "gdrcs": svc.gdrcs_page.to_string() },
    })
}

async fn get_assets(State(st): State<AppState>) -> Json<Value> {
    Json(assets_json(&st.svc.lock().unwrap()))
}

async fn put_assets(
    State(st): State<AppState>,
    h: HeaderMap,
    Json(body): Json<Assets>,
) -> ApiResult {
    let p = auth(&st, &h)?;
    if !p.assets {
        return Err(err(
            StatusCode::FORBIDDEN,
            format!("{} may not set the shared assets", p.name),
        ));
    }
    body.validate()
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let mut svc = st.svc.lock().unwrap();
    for (no, present) in svc.set_assets(body) {
        if present {
            st.feed.page(&svc, no);
        } else {
            st.feed.delete(no);
        }
    }
    Ok(Json(assets_json(&svc)))
}

#[derive(Deserialize)]
struct Subtitles {
    lines: Vec<String>,
    page: Option<String>,
}

#[derive(Deserialize)]
struct StatusDisplay {
    text: String,
}

async fn status_display(
    State(st): State<AppState>,
    h: HeaderMap,
    Json(body): Json<StatusDisplay>,
) -> ApiResult {
    let p = auth(&st, &h)?;
    if !p.status_display {
        return Err(err(
            StatusCode::FORBIDDEN,
            format!("{} may not set the status display", p.name),
        ));
    }
    if body.text.chars().count() > 20 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "status display is at most 20 characters",
        ));
    }
    let mut svc = st.svc.lock().unwrap();
    let text = if body.text.is_empty() {
        svc.service_name.clone()
    } else {
        body.text
    };
    if let Some(sd) = svc.service_data.as_mut() {
        sd.status = text;
    }
    Ok(Json(json!({ "ok": true })))
}

async fn subtitles(
    State(st): State<AppState>,
    h: HeaderMap,
    Json(body): Json<Subtitles>,
) -> ApiResult {
    let p = auth(&st, &h)?;
    if !p.subtitles {
        return Err(err(
            StatusCode::FORBIDDEN,
            format!("{} may not send subtitles", p.name),
        ));
    }
    let no = parse_page(body.page.as_deref().unwrap_or("888"))?;
    let mut svc = st.svc.lock().unwrap();
    svc.subtitle(no, &body.lines)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    st.feed.subtitle(&svc, no, &body.lines);
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn app() -> (Router, Arc<Mutex<Service>>) {
        let svc = Arc::new(Mutex::new(Service::new("FRIKANALEN", BTreeMap::new())));
        let publishers = vec![Publisher {
            name: "schedule".into(),
            token: "secret".into(),
            token_env: None,
            pages: vec!["200-29F".into()],
            subtitles: false,
            status_display: true,
            assets: true,
            flash_per_sec: 100.0,
        }];
        (
            router(AppState {
                svc: svc.clone(),
                publishers: Arc::new(publishers),
                feed: crate::feed::Feed::new(10),
            }),
            svc,
        )
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let resp = app
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn put_flash_and_preview() {
        let (app, _svc) = app();
        let body = json!({ "subpages": [["{yellow}Sendeplan", "18:00 Nyheter"]] });
        assert_eq!(
            call(&app, "PUT", "/v1/pages/200", Some("secret"), body.clone())
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            call(&app, "PUT", "/v1/pages/100", Some("secret"), body.clone())
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&app, "PUT", "/v1/pages/200", None, body).await.0,
            StatusCode::UNAUTHORIZED
        );
        let (s, _) = call(
            &app,
            "POST",
            "/v1/pages/200/flash",
            Some("secret"),
            json!({ "rows": { "2": "18:05 Nå: Været" } }),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let (_, v) = call(&app, "GET", "/v1/pages/200", None, Value::Null).await;
        assert_eq!(v["preview"][0][0], " Sendeplan");
        assert_eq!(v["preview"][0][1], "18:05 Nå: Været");
        let (s, _) = call(
            &app,
            "POST",
            "/v1/subtitles",
            Some("secret"),
            json!({ "lines": ["x"] }),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn status_display() {
        let (app, svc) = app();
        svc.lock().unwrap().service_data = Some(crate::p830::ServiceData {
            initial_page: "100".parse().unwrap(),
            network_id: 0,
            status: "FRIKANALEN".into(),
        });
        let (s, _) = call(
            &app,
            "PUT",
            "/v1/status_display",
            Some("secret"),
            json!({ "text": "Nyheter kl 18" }),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(
            svc.lock().unwrap().service_data.as_ref().unwrap().status,
            "Nyheter kl 18"
        );
        let (s, _) = call(
            &app,
            "PUT",
            "/v1/status_display",
            Some("secret"),
            json!({ "text": "x".repeat(21) }),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        call(
            &app,
            "PUT",
            "/v1/status_display",
            Some("secret"),
            json!({ "text": "" }),
        )
        .await;
        assert_eq!(
            svc.lock().unwrap().service_data.as_ref().unwrap().status,
            "FRIKANALEN"
        );
    }

    #[tokio::test]
    async fn level_25_subpage() {
        let (app, svc) = app();
        let body = json!({
            "subpages": [{
                "rows": ["{yellow}Logo"],
                "raw_rows": { "2": vec![0x40; 40] },
                "x26": [[41, 4, 0], [2, 13, 64]],
                "packets": [{ "packet": 28, "designation": 0, "triplets": [[0, 0, 0]] }]
            }],
            "flags": { "inhibit_display": true }
        });
        let (s, v) = call(&app, "PUT", "/v1/pages/20F", Some("secret"), body).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        {
            let svc = svc.lock().unwrap();
            let p = &svc.pages[&"20F".parse().unwrap()];
            assert!(p.flags.inhibit_display);
            assert_eq!(p.subpages[0].rows[1], vec![0x40; 40]);
            assert_eq!(p.subpages[0].x26.len(), 2);
            assert_eq!(p.subpages[0].extra.len(), 1);
        }
        let bad = json!({ "subpages": [{ "packets": [{ "packet": 26, "designation": 0, "triplets": [] }] }] });
        let (s, _) = call(&app, "PUT", "/v1/pages/20F", Some("secret"), bad).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let bad = json!({ "subpages": [{ "raw_rows": { "1": [1, 2] } }] });
        let (s, _) = call(&app, "PUT", "/v1/pages/20F", Some("secret"), bad).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        // A typo in a field name says which field.
        let bad = json!({ "subpages": [{ "rowz": [] }] });
        let (s, v) = call(&app, "PUT", "/v1/pages/20F", Some("secret"), bad).await;
        assert_ne!(s, StatusCode::OK);
        assert!(v.to_string().contains("rowz") || s == StatusCode::UNPROCESSABLE_ENTITY);
        let bad = json!({ "subpages": [{ "packets": [{ "packet": 28, "designation": 0, "triplets": [[64, 0, 0]] }] }] });
        let (s, _) = call(&app, "PUT", "/v1/pages/20F", Some("secret"), bad).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let ok = json!({ "subpages": [["x"]], "repeat_secs": 30 });
        assert_eq!(
            call(&app, "PUT", "/v1/pages/20F", Some("secret"), ok)
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            svc.lock().unwrap().pages[&"20F".parse().unwrap()].repeat_secs,
            Some(30)
        );
    }

    #[tokio::test]
    async fn shared_assets() {
        let (app, svc) = app();
        call(
            &app,
            "PUT",
            "/v1/pages/200",
            Some("secret"),
            json!({ "subpages": [["x"]] }),
        )
        .await;
        let glyph = vec!["#".repeat(12); 10];
        let body = json!({
            "palette": { "colours": ["#0f1926", "#f15645"], "screen": 16, "row": 16, "remap": 6 },
            "glyphs": [{ "name": "dot", "rows": glyph }],
            "objects": [{ "name": "logo", "type": "passive", "x26": [[0, 13, 0]] }]
        });
        let (s, v) = call(&app, "PUT", "/v1/assets", Some("secret"), body).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["glyphs"]["dot"], 0);
        assert_eq!(v["objects"]["logo"], json!([56, 19, 0]));
        {
            let svc = svc.lock().unwrap();
            for p in ["2FE", "8E8", "8E9"] {
                assert_eq!(svc.pages[&p.parse().unwrap()].owner, "ttxd", "{p}");
            }
            assert_eq!(
                svc.pages[&"2FE".parse().unwrap()].subpages[0].extra[0].0,
                29
            );
        }
        // A page in a new magazine gets its own MOT; ttxd's pages are off limits.
        call(
            &app,
            "PUT",
            "/v1/pages/250",
            Some("secret"),
            json!({ "subpages": [["y"]] }),
        )
        .await;
        let (s, _) = call(
            &app,
            "PUT",
            "/v1/pages/2FE",
            Some("secret"),
            json!({ "subpages": [["z"]] }),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (_, v) = call(&app, "GET", "/v1/assets", None, Value::Null).await;
        assert_eq!(v["pages"]["gdrcs"], "8E9");
        let bad = json!({ "glyphs": [{ "name": "short", "rows": ["#"] }] });
        let (s, _) = call(&app, "PUT", "/v1/assets", Some("secret"), bad).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        // Clearing the assets removes ttxd's pages.
        let (s, _) = call(&app, "PUT", "/v1/assets", Some("secret"), json!({})).await;
        assert_eq!(s, StatusCode::OK);
        assert!(
            svc.lock()
                .unwrap()
                .pages
                .values()
                .all(|p| p.owner != "ttxd")
        );
    }
}
