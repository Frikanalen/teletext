//! HTTP API for publishers.
//!
//!   GET    /v1/status
//!   GET    /v1/pages                     list pages
//!   GET    /v1/pages/{page}              page with a plain-text preview
//!   PUT    /v1/pages/{page}              store a page   {subpages:[[row,...] | {rows, raw_rows, x26, packets},...], cycle_secs?, ttl_secs?, flash?, flags?}
//!   DELETE /v1/pages/{page}
//!   POST   /v1/pages/{page}/flash        replace rows now {rows:{"5":"text"}, subpage?}
//!   POST   /v1/subtitles                 show lines     {lines:["...",...], page?}; [] clears
//!
//! Writes need `Authorization: Bearer <token>`; each publisher may only touch its own pages.

use crate::config::Publisher;
use crate::page::{Content, PageNo, Triplet};
use crate::service::{PageFlags, Service, StoredPage};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
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

async fn get_page(State(st): State<AppState>, Path(page): Path<String>) -> ApiResult {
    let no = parse_page(&page)?;
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
    fn content(&self) -> Result<Content, String> {
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
    let subpages = body
        .subpages
        .iter()
        .map(SubpageIn::content)
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
    st.svc.lock().unwrap().put_page(no, stored, body.flash);
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
        Some(_) => Ok(Json(json!({ "ok": true }))),
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
    st.svc
        .lock()
        .unwrap()
        .flash_rows(no, body.subpage, &body.rows, SystemTime::now())
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct Subtitles {
    lines: Vec<String>,
    page: Option<String>,
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
    st.svc
        .lock()
        .unwrap()
        .subtitle(no, &body.lines)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
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
            pages: vec!["200-29F".into()],
            subtitles: false,
            flash_per_sec: 100.0,
        }];
        (
            router(AppState {
                svc: svc.clone(),
                publishers: Arc::new(publishers),
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
}
