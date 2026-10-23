//! HTTP layer: router, middleware stack and handlers.
//!
//! | Route | Auth |
//! |---|---|
//! | `GET /` | none (plain text) |
//! | `GET /healthz` | none |
//! | `/auth/login`, `/auth/callback`, `/auth/logout` | OIDC login ([`auth_routes`]) |
//! | `/.well-known/oauth-*`, `/oauth/*` | built-in authorization server ([`crate::oauth::routes`]) |
//! | `/api/*` | session cookie, API/bootstrap token or device token ([`api`], [`accounts`], [`events`]) |
//!
//! `/mcp` follows in LIC-12. Protected routes use
//! [`crate::auth::middleware::require_auth`] and the
//! [`crate::auth::Principal`] extractor and call into [`App`].

pub mod accounts;
pub mod api;
pub mod auth_routes;
pub mod events;
pub mod extract;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_TYPE, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{middleware, Json, Router};
use serde_json::json;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::RequestBodyTimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::app::App;
use crate::auth::middleware::require_auth;
use crate::auth::oidc::OidcClient;
use crate::error::AppError;

/// A request body that sends nothing for this long fails, so a stalled
/// upload does not hold its connection (and a graceful shutdown) open.
pub const REQUEST_BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(15);

/// Builds the complete router.
pub fn router(app: App, oidc: Option<Arc<OidcClient>>) -> Router {
    let api = api::routes()
        .fallback(api_not_found)
        .route_layer(middleware::from_fn_with_state(app.clone(), require_auth));

    Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .nest("/api", api)
        .merge(crate::oauth::routes::routes(
            crate::oauth::routes::OAuthState::new(app.clone()),
        ))
        .merge(auth_routes::routes(auth_routes::AuthState {
            app: app.clone(),
            oidc,
        }))
        .fallback(not_found)
        .with_state(app)
        .layer(CompressionLayer::new().gzip(true).br(true))
        .layer(RequestBodyTimeoutLayer::new(REQUEST_BODY_IDLE_TIMEOUT))
        .layer(SetResponseHeaderLayer::if_not_present(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(TraceLayer::new_for_http())
}

/// Lichtung has no web UI: `/` only says what runs here.
async fn index() -> Response {
    let body = format!(
        "Lichtung-Server {}\n\n\
         Hier läuft der Server der Kalender-App Lichtung. Eine Weboberfläche gibt es nicht;\n\
         Kalender, Konten und Tokens werden in der App gepflegt.\n",
        env!("CARGO_PKG_VERSION")
    );
    let mut response = (StatusCode::OK, body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

async fn healthz(State(app): State<App>) -> Response {
    let version = env!("CARGO_PKG_VERSION");
    match app.health().await {
        Ok(()) => Json(json!({"status": "ok", "version": version})).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "health check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"status": "error", "version": version})),
            )
                .into_response()
        }
    }
}

async fn api_not_found() -> AppError {
    AppError::NotFound("Unbekannter API-Pfad".into())
}

async fn not_found() -> AppError {
    AppError::NotFound("Unbekannter Pfad".into())
}
