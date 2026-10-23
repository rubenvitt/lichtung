//! `/api` handlers of the sync API (contract sections 4–6) and conflicts.
//!
//! | Route | Who |
//! |---|---|
//! | `GET /api/sync/pull?since&limit` | every principal of `/api` |
//! | `GET /api/sync/events` (SSE) | every principal of `/api` |
//! | `POST /api/sync/push` | every principal of `/api` (origin `app:<device>`, `web`, `api:<token>`) |
//! | `GET /api/conflicts`, `POST /api/conflicts/{id}/dismiss` | every principal of `/api` |
//!
//! Writes through API tokens are allowed: what a calendar accepts is
//! decided per calendar (`writable`); the MCP guard `mcp_write` only
//! applies to writes from `/mcp` (LIC-12).

use std::convert::Infallible;

use axum::extract::rejection::QueryRejection;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{stream, Stream, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};

use super::extract::ApiJson;
use crate::app::App;
use crate::auth::Principal;
use crate::conflicts::Conflict;
use crate::error::{AppError, AppResult};
use crate::sync_api::{PullPage, DEFAULT_PULL_LIMIT};
use crate::writes::Origin;

/// Body limit of `POST /api/sync/push` (up to 100 mutations with notes).
pub const PUSH_BODY_LIMIT: usize = 4 * 1024 * 1024;

pub fn routes() -> Router<App> {
    Router::new()
        .route("/sync/pull", get(pull))
        .route("/sync/events", get(events))
        .route(
            "/sync/push",
            post(push).layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)),
        )
        .route("/conflicts", get(list_conflicts))
        .route("/conflicts/{id}/dismiss", post(dismiss_conflict))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PullQuery {
    #[serde(default)]
    since: Option<i64>,
    #[serde(default)]
    limit: Option<u32>,
}

async fn pull(
    State(app): State<App>,
    query: Result<Query<PullQuery>, QueryRejection>,
) -> AppResult<Json<PullPage>> {
    let Query(query) = query
        .map_err(|e| AppError::bad_request(format!("Ungültige Abfrage: {}", e.body_text())))?;
    Ok(Json(
        app.pull(
            query.since.unwrap_or(0),
            query.limit.unwrap_or(DEFAULT_PULL_LIMIT),
        )
        .await?,
    ))
}

/// Server-Sent Events: `event: cursor` with the current cursor right away
/// and after every change (several changes may collapse into one). Ends
/// on shutdown.
async fn events(State(app): State<App>) -> Response {
    // Subscribe before reading the cursor, so no change is missed.
    let mut rx = app.subscribe_changes();
    let initial = *rx.borrow_and_update();
    let changes = stream::unfold(rx, |mut rx| async move {
        rx.changed().await.ok()?;
        let cursor = *rx.borrow_and_update();
        Some((cursor, rx))
    });
    let events: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> = Box::pin(
        stream::once(async move { initial })
            .chain(changes)
            .map(|cursor| Ok(Event::default().event("cursor").data(cursor.to_string())))
            .take_until(app.shutdown_token().cancelled_owned()),
    );
    let mut response = Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response();
    // Ask reverse proxies not to buffer the stream.
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

#[derive(Debug, Deserialize)]
struct PushBody {
    mutations: Vec<Value>,
}

async fn push(
    State(app): State<App>,
    principal: Principal,
    ApiJson(body): ApiJson<PushBody>,
) -> AppResult<Json<Value>> {
    let origin = Origin::of(&principal);
    let results = app.push(&origin, body.mutations).await?;
    Ok(Json(json!({ "results": results })))
}

async fn list_conflicts(State(app): State<App>) -> AppResult<Json<Vec<Conflict>>> {
    Ok(Json(app.list_conflicts().await?))
}

async fn dismiss_conflict(State(app): State<App>, Path(id): Path<String>) -> AppResult<StatusCode> {
    app.dismiss_conflict(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
