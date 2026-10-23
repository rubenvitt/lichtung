//! `/api` handlers for events.
//!
//! | Route | Who |
//! |---|---|
//! | `GET /api/occurrences?from&to&calendar_ids&time_zone&include_cancelled` | every principal of `/api` |
//!
//! Pull, push and SSE follow with LIC-10/LIC-11.

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};

use crate::app::App;
use crate::error::{AppError, AppResult};
use crate::events::occurrences::{OccurrenceQuery, Occurrences};

pub fn routes() -> Router<App> {
    Router::new().route("/occurrences", get(occurrences))
}

async fn occurrences(
    State(app): State<App>,
    query: Result<Query<OccurrenceQuery>, QueryRejection>,
) -> AppResult<Json<Occurrences>> {
    let Query(query) = query
        .map_err(|e| AppError::bad_request(format!("Ungültige Abfrage: {}", e.body_text())))?;
    Ok(Json(app.occurrences(query).await?))
}
