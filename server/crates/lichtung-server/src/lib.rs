//! Lichtung server: login, API and device tokens, OAuth for `/mcp` and the
//! apps, calendar accounts, CalDAV event sync, persistence. The sync API
//! (pull, push, SSE) and MCP tools build on this (later tasks).
//!
//! - [`config`]: configuration from environment variables (bootstrap only)
//! - [`db`]: SQLite schema, migrations and queries
//! - [`app`]: application service (the only way to read or change data)
//! - [`accounts`]: calendar accounts, calendars, calendar sets
//! - [`events`]: stored events (iCalendar original), event JSON, occurrences
//! - [`sync`]: CalDAV sync engine and its scheduler
//! - [`crypto`]: encryption of provider credentials (AES-256-GCM)
//! - [`writes`]: writing events to the provider (create, update, delete,
//!   respond) with conflict detection
//! - [`conflicts`]: dropped own changes (provider wins), visible until dismissed
//! - [`changes`]: change log (`seq`, tombstones, cursor notifications)
//! - [`auth`]: principals, API and device tokens, sessions, CSRF rule, OIDC
//! - [`oauth`]: built-in OAuth 2.1 authorization server (`/mcp`, devices)
//! - [`http`]: router and handlers
//! - [`backup`]: nightly `VACUUM INTO` backups
//! - [`healthcheck`]: client for the `healthcheck` subcommand

pub mod accounts;
pub mod app;
pub mod auth;
pub mod backup;
pub mod changes;
pub mod config;
pub mod conflicts;
pub mod crypto;
pub mod db;
pub mod error;
pub mod events;
pub mod healthcheck;
pub mod http;
pub mod oauth;
pub mod sync;
pub mod writes;

use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Router;

pub use app::App;
pub use config::Config;
pub use error::{AppError, AppResult};

use auth::oidc::OidcClient;

/// The default HTTP client for outgoing requests (OIDC provider).
pub fn default_http_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("lichtung/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("HTTP-Client lässt sich nicht bauen")
}

/// Opens the app and builds the router.
pub async fn build(config: Config) -> anyhow::Result<(App, Router)> {
    build_with_http_client(config, default_http_client()?).await
}

/// Like [`build`], with a custom HTTP client for outgoing requests.
pub async fn build_with_http_client(
    config: Config,
    http: reqwest::Client,
) -> anyhow::Result<(App, Router)> {
    let oidc = config.oidc.clone().map(|c| {
        Arc::new(OidcClient::new(
            c,
            config.oidc_redirect_uri(),
            &config.session_secret,
            http,
        ))
    });
    let app = App::open(config).await?;
    let router = http::router(app.clone(), oidc);
    Ok((app, router))
}

fn log_startup_warnings(config: &Config) {
    if config.dev_auth {
        for _ in 0..3 {
            tracing::warn!(
                "!!! LICHTUNG_DEV_AUTH=true: Anmeldung OHNE OIDC aktiv. Jede Anfrage gilt als angemeldet. Nur lokal verwenden !!!"
            );
        }
    } else if config.oidc.is_none() {
        tracing::warn!(
            "no OIDC provider configured: browser login and OAuth consent are disabled, only API tokens work"
        );
    }
    if config.uses_dev_secret_key() {
        tracing::warn!(
            "LICHTUNG_SECRET_KEY not set: provider credentials use the fixed development key. Never use this database in production"
        );
    }
    if config
        .bootstrap_token
        .as_ref()
        .is_some_and(|t| t.len() < 32)
    {
        tracing::warn!("LICHTUNG_BOOTSTRAP_TOKEN is shorter than 32 characters");
    }
}

/// Runs the server until SIGTERM/SIGINT, then shuts down gracefully.
pub async fn run(config: Config) -> anyhow::Result<()> {
    log_startup_warnings(&config);
    let bind = config.bind;
    let (app, router) = build(config).await?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("{bind} lässt sich nicht binden"))?;
    tracing::info!(addr = %listener.local_addr()?, "lichtung-server listening");
    serve(listener, app, router, shutdown_signal(), DRAIN_TIMEOUT).await
}

/// How long running requests may take to finish after the shutdown signal.
/// Below Docker's default stop timeout of 10 s, so the process still exits
/// on its own (and closes SQLite) before it would be killed.
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(8);

/// Serves `router` on `listener` with the nightly backup and the sync
/// scheduler until `signal`
/// completes, then lets running requests finish for at most
/// `drain_timeout` and closes the remaining connections.
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: App,
    router: Router,
    signal: impl std::future::Future<Output = ()> + Send + 'static,
    drain_timeout: Duration,
) -> anyhow::Result<()> {
    let backup = tokio::spawn(backup::run_nightly(app.clone()));
    let scheduler = tokio::spawn(sync::run_scheduler(app.clone()));
    let shutdown = app.shutdown_token();
    let service = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
    let server = axum::serve(listener, service).with_graceful_shutdown({
        let shutdown = shutdown.clone();
        async move {
            signal.await;
            tracing::info!("shutting down");
            // Ends long-running streams and background tasks so connections
            // drain.
            shutdown.cancel();
        }
    });
    let deadline = async {
        shutdown.cancelled().await;
        tokio::time::sleep(drain_timeout).await;
    };
    tokio::select! {
        result = server.into_future() => result?,
        () = deadline => {
            // Dropping the server stops accepting; the remaining connection
            // tasks end with the runtime.
            tracing::warn!(timeout = ?drain_timeout, "requests still running at the drain deadline, closing them");
        }
    }
    match tokio::time::timeout(drain_timeout, backup).await {
        Ok(joined) => joined?,
        Err(_) => tracing::warn!("backup still running at shutdown"),
    }
    match tokio::time::timeout(drain_timeout, scheduler).await {
        Ok(joined) => joined?,
        Err(_) => tracing::warn!("sync scheduler still running at shutdown"),
    }
    tracing::info!("stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "cannot listen for SIGINT");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
