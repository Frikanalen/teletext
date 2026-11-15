//! HTTP API for publishers.
//!
//!   GET    /v1/status
//!   GET    /v1/pages                     list pages
//!   GET    /v1/pages/{page}              page with a plain-text preview
//!   PUT    /v1/pages/{page}              store a page   {subpages:[[row,...],...], cycle_secs?, ttl_secs?, flash?}
//!   DELETE /v1/pages/{page}
//!   POST   /v1/pages/{page}/flash        replace rows now {rows:{"5":"text"}, subpage?}
//!   POST   /v1/subtitles                 show lines     {lines:["...",...], page?}; [] clears
//!
//! Writes need `Authorization: Bearer <token>`; each publisher may only touch its own pages.

use crate::config::Publisher;
use crate::page::{Content, PageNo};
use crate::service::{Service, StoredPage};
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
    /// Each subpage is a list of markup rows, first entry = row 1.
    subpages: Vec<Vec<String>>,
    #[serde(default = "default_cycle")]
    cycle_secs: u32,
    ttl_secs: Option<u64>,
    #[serde(default)]
    flash: bool,
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
        .map(|rows| Content::from_markup(rows))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let now = SystemTime::now();
    let stored = StoredPage::new(
        &publisher.name,
        subpages,
        body.cycle_secs,
        body.ttl_secs.map(Duration::from_secs),
        now,
    );
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
}
