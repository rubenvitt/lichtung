//! `/api` handlers for openings (booking links, LIC-31,
//! `docs/api/openings.md`).
//!
//! | Route | Who |
//! |---|---|
//! | `GET /api/openings`, `GET /api/openings/{id}` | every principal of `/api` |
//! | `POST /api/openings`, `PATCH`/`DELETE /api/openings/{id}` | browser session or device |
//!
//! Creating a link writes placeholders into its target calendar whatever
//! that calendar's `mcp_write` says, so like calendar settings only the app
//! and browser sessions may change links
//! ([`Principal::may_change_settings`]); API tokens also open `/mcp`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};

use super::extract::ApiJson;
use crate::app::App;
use crate::auth::Principal;
use crate::error::{AppError, AppResult};
use crate::openings::{NewOpening, Opening, OpeningPatch};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/openings", get(list_openings).post(create_opening))
        .route(
            "/openings/{id}",
            get(get_opening)
                .patch(update_opening)
                .delete(delete_opening),
        )
}

fn require_manager(principal: &Principal) -> AppResult<()> {
    if principal.may_change_settings() {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Buchungslinks lassen sich nur mit einer Browser-Sitzung oder in der Lichtung-App \
             anlegen, ändern und löschen"
                .into(),
        ))
    }
}

async fn list_openings(State(app): State<App>) -> AppResult<Json<Vec<Opening>>> {
    Ok(Json(app.list_openings().await?))
}

async fn get_opening(State(app): State<App>, Path(id): Path<String>) -> AppResult<Json<Opening>> {
    Ok(Json(app.opening(&id).await?))
}

async fn create_opening(
    State(app): State<App>,
    principal: Principal,
    ApiJson(body): ApiJson<NewOpening>,
) -> AppResult<(StatusCode, Json<Opening>)> {
    require_manager(&principal)?;
    let opening = app.create_opening(body).await?;
    Ok((StatusCode::CREATED, Json(opening)))
}

async fn update_opening(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<OpeningPatch>,
) -> AppResult<Json<Opening>> {
    require_manager(&principal)?;
    Ok(Json(app.update_opening(&id, body).await?))
}

async fn delete_opening(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    require_manager(&principal)?;
    app.delete_opening(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
