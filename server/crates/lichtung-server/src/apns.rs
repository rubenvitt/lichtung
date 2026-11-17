//! Silent push notifications through APNs (LIC-27).
//!
//! Reminders are local notifications of the app (LIC-68). An event the app
//! has never seen (created in the morning through MCP, say) would not
//! remind until the app is opened. So after events change, the server
//! wakes the app of every registered device with a background push
//! (`content-available: 1`); the app pulls and schedules its reminders.
//!
//! - Devices register their APNs token with `PUT /api/devices/current/push`
//!   ([`crate::http::api`]), stored at the device ([`crate::db::set_device_push`]).
//! - [`run_notifier`] follows the change cursor. Only changes of events
//!   count ([`crate::db::event_change_seq`]): every sync run of an account
//!   moves the cursor, which must not wake the phone. A burst becomes one
//!   push ([`ApnsConfig::debounce`]), and push rounds keep a distance
//!   ([`ApnsConfig::min_interval`]), as APNs throttles background pushes.
//! - Every registered device gets the push, also one whose app is open:
//!   an SSE stream may look open on the server long after the phone lost
//!   its network, and a redundant pull costs little.
//! - Authentication with the `.p8` key (ES256 JWT, renewed after 50
//!   minutes). A token APNs does not know in its environment is tried in
//!   the other one once (a build from Xcode reports `sandbox`, TestFlight
//!   `production`); a token APNs rejects for good is removed.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Context;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use uuid::Uuid;

use crate::app::{now_ms, App};
use crate::config::ApnsConfig;
use crate::db::{self, PushTarget};
use crate::error::AppResult;

/// The two APNs environments a token can belong to.
pub const SANDBOX: &str = "sandbox";
pub const PRODUCTION: &str = "production";

/// A provider token is reused this long (APNs: at most an hour, at least
/// 20 minutes).
const PROVIDER_TOKEN_LIFETIME: Duration = Duration::from_secs(50 * 60);

/// A push not delivered within a day (phone off) is dropped; until then
/// APNs keeps the newest one (`apns-collapse-id`).
const EXPIRATION_SECS: i64 = 24 * 60 * 60;

/// Collapses pending pushes on the device: only the newest counts.
const COLLAPSE_ID: &str = "lichtung-sync";

/// Body of the silent push: no alert, no sound, only the wake-up.
const PAYLOAD: &str = r#"{"aps":{"content-available":1}}"#;

/// Checks that the key signs (at startup, so a wrong key stops the server
/// instead of failing at the first push).
pub fn check_key(config: &ApnsConfig) -> Result<(), jsonwebtoken::errors::Error> {
    sign(config, 0).map(|_| ())
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    iat: i64,
}

fn sign(config: &ApnsConfig, iat: i64) -> Result<String, jsonwebtoken::errors::Error> {
    let key = EncodingKey::from_ec_pem(config.key_pem.as_bytes())?;
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(config.key_id.clone());
    jsonwebtoken::encode(
        &header,
        &Claims {
            iss: &config.team_id,
            iat,
        },
        &key,
    )
}

/// The other environment (`sandbox` ↔ `production`).
pub fn other_environment(environment: &str) -> &'static str {
    if environment == SANDBOX {
        PRODUCTION
    } else {
        SANDBOX
    }
}

/// Result of one push to one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    /// `BadDeviceToken`: unknown in this environment (maybe in the other).
    BadToken,
    /// The token is no longer valid (`410 Unregistered`, wrong topic).
    Gone(String),
    /// APNs rejected the provider token (wrong key, team or key ID).
    Unauthorized(String),
    /// Anything else (throttled, APNs unavailable, network): next round.
    Failed(String),
}

/// Sends silent pushes with token-based authentication.
pub struct ApnsClient {
    config: ApnsConfig,
    http: reqwest::Client,
    provider_token: Mutex<Option<(String, Instant)>>,
}

impl ApnsClient {
    pub fn new(config: ApnsConfig) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(concat!("lichtung/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("HTTP-Client für APNs lässt sich nicht bauen")?;
        Ok(Self {
            config,
            http,
            provider_token: Mutex::new(None),
        })
    }

    fn provider_token(&self) -> Result<String, jsonwebtoken::errors::Error> {
        let mut cached = self
            .provider_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((token, at)) = cached.as_ref() {
            if at.elapsed() < PROVIDER_TOKEN_LIFETIME {
                return Ok(token.clone());
            }
        }
        let token = sign(&self.config, now_ms() / 1000)?;
        *cached = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    fn forget_provider_token(&self) {
        *self
            .provider_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// Sends one silent push to `device_token` in `environment`.
    pub async fn send(&self, device_token: &str, environment: &str) -> Delivery {
        let base = if environment == SANDBOX {
            &self.config.sandbox_url
        } else {
            &self.config.production_url
        };
        let url = match base.join(&format!("3/device/{device_token}")) {
            Ok(url) => url,
            Err(e) => return Delivery::Failed(format!("URL: {e}")),
        };
        let jwt = match self.provider_token() {
            Ok(t) => t,
            Err(e) => return Delivery::Unauthorized(format!("JWT: {e}")),
        };
        let expiration = now_ms() / 1000 + EXPIRATION_SECS;
        let response = self
            .http
            .post(url)
            .header("authorization", format!("bearer {jwt}"))
            .header("apns-topic", &self.config.topic)
            .header("apns-push-type", "background")
            // Background pushes must use priority 5.
            .header("apns-priority", "5")
            .header("apns-collapse-id", COLLAPSE_ID)
            .header("apns-expiration", expiration.to_string())
            .header("content-type", "application/json")
            .body(PAYLOAD)
            .send()
            .await;
        let response = match response {
            Ok(r) => r,
            Err(e) => return Delivery::Failed(e.to_string()),
        };
        let status = response.status().as_u16();
        if status == 200 {
            return Delivery::Sent;
        }
        let reason = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("reason")?.as_str().map(str::to_owned))
            .unwrap_or_default();
        match (status, reason.as_str()) {
            (400, "BadDeviceToken") => Delivery::BadToken,
            (400, "DeviceTokenNotForTopic") | (410, _) => Delivery::Gone(reason),
            (403, "ExpiredProviderToken" | "InvalidProviderToken") => {
                self.forget_provider_token();
                Delivery::Unauthorized(reason)
            }
            (403, _) => Delivery::Unauthorized(reason),
            _ => Delivery::Failed(format!("{status} {reason}")),
        }
    }
}

/// Pushes to every registered device. Returns how many pushes were
/// delivered.
pub async fn notify_devices(app: &App, client: &ApnsClient) -> AppResult<usize> {
    let targets = app.with_reader(|conn| Ok(db::push_targets(conn)?)).await?;
    let mut sent = 0;
    for target in targets {
        if deliver(app, client, &target).await? {
            sent += 1;
        }
    }
    Ok(sent)
}

async fn deliver(app: &App, client: &ApnsClient, target: &PushTarget) -> AppResult<bool> {
    let device = target.device_id;
    match client.send(&target.token, &target.environment).await {
        Delivery::Sent => Ok(true),
        Delivery::BadToken => {
            let other = other_environment(&target.environment);
            match client.send(&target.token, other).await {
                Delivery::Sent => {
                    tracing::info!(%device, environment = other, "APNs token belongs to the other environment, corrected");
                    let token = target.token.clone();
                    app.with_writer(move |conn| {
                        Ok(db::set_device_push_environment(
                            conn, device, &token, other,
                        )?)
                    })
                    .await?;
                    Ok(true)
                }
                Delivery::BadToken => {
                    forget(app, device, &target.token, "BadDeviceToken").await?;
                    Ok(false)
                }
                Delivery::Gone(reason) => {
                    forget(app, device, &target.token, &reason).await?;
                    Ok(false)
                }
                other => {
                    report(device, &other);
                    Ok(false)
                }
            }
        }
        Delivery::Gone(reason) => {
            forget(app, device, &target.token, &reason).await?;
            Ok(false)
        }
        other => {
            report(device, &other);
            Ok(false)
        }
    }
}

async fn forget(app: &App, device: Uuid, token: &str, reason: &str) -> AppResult<()> {
    tracing::info!(%device, reason, "APNs rejected the device token, push turned off for the device");
    let token = token.to_owned();
    app.with_writer(move |conn| Ok(db::clear_device_push(conn, device, Some(&token), now_ms())?))
        .await?;
    Ok(())
}

fn report(device: Uuid, delivery: &Delivery) {
    match delivery {
        Delivery::Unauthorized(reason) => tracing::error!(
            %device,
            reason,
            "APNs rejected the provider token: check LICHTUNG_APNS_KEY_ID, LICHTUNG_APNS_TEAM_ID and the key"
        ),
        Delivery::Failed(reason) => {
            tracing::warn!(%device, reason, "silent push failed, retried with the next change")
        }
        _ => {}
    }
}

/// Background task: pushes after event changes until shutdown. Does
/// nothing without APNs configuration.
pub async fn run_notifier(app: App) {
    let Some(config) = app.config().apns.clone() else {
        tracing::info!("APNs not configured: no silent push notifications");
        return;
    };
    let (debounce, min_interval) = (config.debounce, config.min_interval);
    let client = match ApnsClient::new(config) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "APNs client unavailable: no silent push notifications");
            return;
        }
    };
    let shutdown = app.shutdown_token();
    let mut changes = app.subscribe_changes();
    let mut pushed = match app
        .with_reader(|conn| Ok(db::event_change_seq(conn)?))
        .await
    {
        Ok(seq) => seq,
        Err(e) => {
            tracing::error!(error = %e, "APNs notifier cannot read the change state");
            return;
        }
    };
    tracing::info!("APNs notifier running");
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return,
            changed = changes.changed() => if changed.is_err() { return },
        }
        // Let the burst settle (a sync round writes many events).
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(debounce) => {}
        }
        changes.borrow_and_update();
        let seq = match app
            .with_reader(|conn| Ok(db::event_change_seq(conn)?))
            .await
        {
            Ok(seq) => seq,
            Err(e) => {
                tracing::warn!(error = %e, "APNs notifier cannot read the change state");
                continue;
            }
        };
        if seq <= pushed {
            continue;
        }
        pushed = seq;
        match notify_devices(&app, &client).await {
            Ok(n) => tracing::debug!(sent = n, "silent push round"),
            Err(e) => tracing::warn!(error = %e, "silent push round failed"),
        }
        // Changes in the meantime mark the receiver; they follow after the pause.
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(min_interval) => {}
        }
    }
}

/// Checks the body of `PUT /api/devices/current/push`: the token as
/// lowercase hex (APNs tokens are 32 bytes today; up to 100 allowed) and
/// the environment.
pub fn validate_registration(token: &str, environment: &str) -> Result<String, String> {
    let token = token.trim();
    if token.len() < 64
        || token.len() > 200
        || !token.len().is_multiple_of(2)
        || !token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("apns_token muss der Device-Token als Hex sein (64 bis 200 Zeichen)".into());
    }
    if environment != SANDBOX && environment != PRODUCTION {
        return Err("environment muss sandbox oder production sein".into());
    }
    Ok(token.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_is_checked() {
        let token = "AB".repeat(32);
        assert_eq!(
            validate_registration(&token, "production").unwrap(),
            "ab".repeat(32)
        );
        assert!(validate_registration(&token, "development").is_err());
        assert!(validate_registration("abc", "sandbox").is_err());
        assert!(validate_registration(&"a".repeat(65), "sandbox").is_err());
        assert!(validate_registration(&"zz".repeat(32), "sandbox").is_err());
        assert!(validate_registration(&"ab".repeat(101), "sandbox").is_err());
    }

    #[test]
    fn environments_swap() {
        assert_eq!(other_environment(SANDBOX), PRODUCTION);
        assert_eq!(other_environment(PRODUCTION), SANDBOX);
    }
}
