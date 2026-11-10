//! `/api` handlers for blocker rules (LIC-75, `docs/api/blockers.md`).
//!
//! | Route | Who |
//! |---|---|
//! | `GET /api/blocker-rules`, `GET /api/blocker-rules/{id}` | every principal of `/api` |
//! | `POST /api/blocker-rules`, `PATCH`/`DELETE /api/blocker-rules/{id}` | browser session or device |
//!
//! A rule writes into its target calendar whatever that calendar's
//! `mcp_write` says, so like openings only the app and browser sessions
//! may change rules ([`Principal::may_change_settings`]).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};

use super::extract::ApiJson;
use crate::app::App;
use crate::auth::Principal;
use crate::blockers::{BlockerRule, BlockerRulePatch, NewBlockerRule};
use crate::error::{AppError, AppResult};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/blocker-rules", get(list_rules).post(create_rule))
        .route(
            "/blocker-rules/{id}",
            get(get_rule).patch(update_rule).delete(delete_rule),
        )
}

fn require_manager(principal: &Principal) -> AppResult<()> {
    if principal.may_change_settings() {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Blocker-Regeln lassen sich nur mit einer Browser-Sitzung oder in der Lichtung-App \
             anlegen, ändern und löschen"
                .into(),
        ))
    }
}

async fn list_rules(State(app): State<App>) -> AppResult<Json<Vec<BlockerRule>>> {
    Ok(Json(app.list_blocker_rules().await?))
}

async fn get_rule(State(app): State<App>, Path(id): Path<String>) -> AppResult<Json<BlockerRule>> {
    Ok(Json(app.blocker_rule(&id).await?))
}

async fn create_rule(
    State(app): State<App>,
    principal: Principal,
    ApiJson(body): ApiJson<NewBlockerRule>,
) -> AppResult<(StatusCode, Json<BlockerRule>)> {
    require_manager(&principal)?;
    let rule = app.create_blocker_rule(body).await?;
    Ok((StatusCode::CREATED, Json(rule)))
}

async fn update_rule(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<BlockerRulePatch>,
) -> AppResult<Json<BlockerRule>> {
    require_manager(&principal)?;
    Ok(Json(app.update_blocker_rule(&id, body).await?))
}

async fn delete_rule(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    require_manager(&principal)?;
    app.delete_blocker_rule(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
