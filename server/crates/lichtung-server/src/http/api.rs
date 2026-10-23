//! `/api` handlers (all behind [`crate::auth::middleware::require_auth`]).
//!
//! | Route | Who |
//! |---|---|
//! | `GET /api/me` | every principal of `/api` |
//! | `GET, POST /api/tokens`, `DELETE /api/tokens/{id}` | browser session or device |
//! | `GET /api/devices`, `DELETE /api/devices/{id}` | browser session or device |
//! | `GET /api/oauth/clients`, `DELETE /api/oauth/clients/{id}` | browser session or device |
//!
//! Credentials are managed by a browser session or a device of the app
//! ([`Principal::may_manage_credentials`]); API, bootstrap and OAuth tokens
//! cannot mint or revoke credentials.

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use super::extract::{parse_uuid, ApiJson};
use crate::app::{App, CreatedToken, TokenInfo};
use crate::auth::{Principal, PrincipalKind};
use crate::error::{AppError, AppResult};
use crate::oauth;

/// Body limit of `/api` requests.
pub const API_BODY_LIMIT: usize = 64 * 1024;

/// The `/api` routes (without auth layer).
pub fn routes() -> Router<App> {
    Router::new()
        .route("/me", get(me))
        .route("/tokens", get(list_tokens).post(create_token))
        .route("/tokens/{id}", delete(revoke_token))
        .route("/devices", get(list_devices))
        .route("/devices/{id}", delete(revoke_device))
        .route("/oauth/clients", get(list_oauth_clients))
        .route("/oauth/clients/{id}", delete(revoke_oauth_client))
        .layer(DefaultBodyLimit::max(API_BODY_LIMIT))
}

async fn me(State(app): State<App>, principal: Principal) -> AppResult<Json<serde_json::Value>> {
    let user = app
        .user(principal.user_id)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let device = match &principal.kind {
        PrincipalKind::Device { id, name } => Some(json!({"id": id, "name": name})),
        _ => None,
    };
    Ok(Json(json!({
        "user_id": user.id,
        "name": user.name,
        "email": user.email,
        "auth": principal.label(),
        "device": device,
    })))
}

fn require_credential_manager(principal: &Principal) -> AppResult<()> {
    if principal.may_manage_credentials() {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Tokens, Geräte und verbundene Apps lassen sich nur mit einer Browser-Sitzung \
             oder in der Lichtung-App verwalten"
                .into(),
        ))
    }
}

async fn list_tokens(
    State(app): State<App>,
    principal: Principal,
) -> AppResult<Json<Vec<TokenInfo>>> {
    require_credential_manager(&principal)?;
    Ok(Json(app.list_tokens(principal.user_id).await?))
}

#[derive(Deserialize)]
struct CreateTokenBody {
    name: String,
}

async fn create_token(
    State(app): State<App>,
    principal: Principal,
    ApiJson(body): ApiJson<CreateTokenBody>,
) -> AppResult<(StatusCode, Json<CreatedToken>)> {
    require_credential_manager(&principal)?;
    let token = app.create_token(principal.user_id, &body.name).await?;
    Ok((StatusCode::CREATED, Json(token)))
}

async fn revoke_token(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    require_credential_manager(&principal)?;
    let id = parse_uuid(&id)?;
    app.revoke_token(principal.user_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_devices(
    State(app): State<App>,
    principal: Principal,
) -> AppResult<Json<Vec<oauth::DeviceInfo>>> {
    require_credential_manager(&principal)?;
    let current = match &principal.kind {
        PrincipalKind::Device { id, .. } => Some(*id),
        _ => None,
    };
    Ok(Json(
        oauth::devices(&app, principal.user_id, current).await?,
    ))
}

async fn revoke_device(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    require_credential_manager(&principal)?;
    let id = parse_uuid(&id)?;
    oauth::revoke_device(&app, principal.user_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_oauth_clients(
    State(app): State<App>,
    principal: Principal,
) -> AppResult<Json<Vec<oauth::ClientInfo>>> {
    require_credential_manager(&principal)?;
    Ok(Json(
        oauth::connected_clients(&app, principal.user_id).await?,
    ))
}

async fn revoke_oauth_client(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    require_credential_manager(&principal)?;
    oauth::revoke_client(&app, principal.user_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
