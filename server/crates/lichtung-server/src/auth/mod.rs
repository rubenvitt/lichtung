//! Authentication: principals, API tokens, device tokens, sessions, CSRF
//! rule and OIDC.
//!
//! - **API tokens**: `Authorization: Bearer lt_<32 bytes base64url>`, stored
//!   as SHA-256 hex, accepted by `/api` and `/mcp`. `LICHTUNG_BOOTSTRAP_TOKEN`
//!   is accepted as an additional fixed token.
//! - **Device tokens**: `ld_…`, issued to the iOS/macOS app by the built-in
//!   authorization server (resource `<public>/api`, see [`crate::oauth`]),
//!   stored as SHA-256 hex, only accepted by `/api`.
//! - **OAuth access tokens**: `loa_…` (refresh `lor_…`), issued for
//!   `<public>/mcp`, only accepted by `/mcp` (see [`crate::oauth`]).
//! - **Sessions**: cookie [`SESSION_COOKIE`] with a random 32-byte ID, stored
//!   as HMAC-SHA256 (key `LICHTUNG_SESSION_SECRET`), 30 days sliding expiry.
//!   Lichtung has no web UI; the browser session exists for the OAuth consent
//!   page (claude.ai, device login) and for managing tokens with a browser.
//! - **CSRF**: cookie-authenticated requests that change state must carry the
//!   header [`CLIENT_HEADER`] (`X-Lichtung-Client`). Browsers cannot add
//!   custom headers cross-site without a CORS preflight, which Lichtung never
//!   grants.

pub mod middleware;
pub mod oidc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "lichtung_session";
/// Header that cookie-authenticated, state-changing requests must send.
pub const CLIENT_HEADER: &str = "x-lichtung-client";
/// Prefix of API tokens.
pub const TOKEN_PREFIX: &str = "lt_";
/// Prefix of device tokens (iOS/macOS app, see [`crate::oauth`]).
pub const DEVICE_TOKEN_PREFIX: &str = "ld_";
/// Session lifetime (sliding).
pub const SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// A session is extended at most this often.
pub const SESSION_RENEW_AFTER_MS: i64 = 24 * 60 * 60 * 1000;
/// `last_used_at` of a token is written at most this often.
pub const TOKEN_TOUCH_INTERVAL_MS: i64 = 60 * 1000;

/// Who is making a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub user_id: Uuid,
    pub kind: PrincipalKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrincipalKind {
    /// Browser session (cookie).
    Session,
    /// `LICHTUNG_DEV_AUTH=true` without any credentials.
    DevAuto,
    /// API token (bearer).
    Token { id: Uuid, name: String },
    /// `LICHTUNG_BOOTSTRAP_TOKEN` (bearer).
    Bootstrap,
    /// OAuth access token of the built-in authorization server (bearer,
    /// only accepted by `/mcp`).
    OAuth {
        client_id: String,
        client_name: String,
    },
    /// Device token of the iOS/macOS app (bearer, only accepted by `/api`).
    Device { id: Uuid, name: String },
}

/// Name used for the bootstrap token in labels and logs.
pub const BOOTSTRAP_TOKEN_NAME: &str = "bootstrap";

impl Principal {
    /// Authenticated like a browser (cookie or dev auth): subject to the CSRF
    /// rule.
    pub fn is_browser(&self) -> bool {
        matches!(self.kind, PrincipalKind::Session | PrincipalKind::DevAuto)
    }

    /// Whether this is a device of the app.
    pub fn is_device(&self) -> bool {
        matches!(self.kind, PrincipalKind::Device { .. })
    }

    /// May manage credentials (API tokens, devices, connected OAuth clients):
    /// a browser session or a device of the app. Lichtung has no web UI, so
    /// unlike Zunder the app manages tokens; API, bootstrap and OAuth tokens
    /// cannot mint or revoke credentials.
    pub fn may_manage_credentials(&self) -> bool {
        self.is_browser() || self.is_device()
    }

    /// May change calendar settings and calendar sets: a browser session or
    /// a device of the app. API and bootstrap tokens may not: they also open
    /// `/mcp`, and `mcp_write` is the guard against Claude writing into a
    /// calendar, so a token for Claude must not be able to lift it.
    pub fn may_change_settings(&self) -> bool {
        self.is_browser() || self.is_device()
    }

    /// A short label for logs and change sources: `web`, `app:<device>`,
    /// `api:<token>`, `oauth:<client>`.
    pub fn label(&self) -> String {
        match &self.kind {
            PrincipalKind::Session | PrincipalKind::DevAuto => "web".to_owned(),
            PrincipalKind::Token { name, .. } => format!("api:{name}"),
            PrincipalKind::Bootstrap => format!("api:{BOOTSTRAP_TOKEN_NAME}"),
            PrincipalKind::OAuth { client_name, .. } => format!("oauth:{client_name}"),
            PrincipalKind::Device { name, .. } => format!("app:{name}"),
        }
    }
}

/// Whether a request carries the CSRF header ([`CLIENT_HEADER`], non-empty).
pub fn has_client_header(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(CLIENT_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| !v.trim().is_empty())
}

/// The error for a missing CSRF header.
pub fn missing_client_header() -> crate::error::AppError {
    crate::error::AppError::Forbidden(
        "Header X-Lichtung-Client fehlt (Schutz vor Cross-Site-Anfragen)".into(),
    )
}

/// 32 random bytes as base64url without padding (43 characters).
pub fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating system RNG is available");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A fresh API token (`lt_…`).
pub fn new_api_token() -> String {
    format!("{TOKEN_PREFIX}{}", random_secret())
}

/// SHA-256 hex digest (storage form of tokens and codes).
pub fn sha256_hex(input: &str) -> String {
    hex(&Sha256::digest(input.as_bytes()))
}

/// Keyed hash of a session ID (storage form of sessions).
pub fn session_hash(key: &[u8], session_id: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(session_id.as_bytes());
    hex(&mac.finalize().into_bytes())
}

/// PKCE S256 code challenge for a verifier.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Constant-time equality for secrets of equal length.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_format() {
        let t = new_api_token();
        assert!(t.starts_with("lt_"));
        assert_eq!(t.len(), 3 + 43);
        assert_ne!(t, new_api_token());
        assert!(t[3..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn hashes() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_ne!(session_hash(b"k1", "s"), session_hash(b"k2", "s"));
        assert_eq!(session_hash(b"k1", "s").len(), 64);
        // RFC 7636 appendix B
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn labels_and_permissions() {
        let user_id = Uuid::nil();
        let p = |kind| Principal { user_id, kind };
        let web = p(PrincipalKind::Session);
        assert_eq!(web.label(), "web");
        assert!(web.is_browser() && web.may_manage_credentials());
        let token = p(PrincipalKind::Token {
            id: Uuid::nil(),
            name: "werkzeug".into(),
        });
        assert_eq!(token.label(), "api:werkzeug");
        assert!(!token.is_browser() && !token.may_manage_credentials());
        assert!(!token.may_change_settings());
        let boot = p(PrincipalKind::Bootstrap);
        assert_eq!(boot.label(), "api:bootstrap");
        assert!(!boot.may_manage_credentials());
        let oauth = p(PrincipalKind::OAuth {
            client_id: "c".into(),
            client_name: "Claude".into(),
        });
        assert_eq!(oauth.label(), "oauth:Claude");
        assert!(!oauth.may_manage_credentials());
        let device = p(PrincipalKind::Device {
            id: Uuid::nil(),
            name: "Testgerät".into(),
        });
        assert_eq!(device.label(), "app:Testgerät");
        assert!(!device.is_browser() && device.may_manage_credentials());
        assert!(device.may_change_settings() && web.may_change_settings());
    }
}
