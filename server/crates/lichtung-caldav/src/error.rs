//! Typed errors so the caller can decide what to do next (retry, full resync,
//! report a conflict, ask for new credentials).

use std::time::Duration;

/// Maximum number of characters kept from a server error body.
const BODY_SUMMARY_LIMIT: usize = 300;

/// Everything that can go wrong when talking to a CalDAV server.
///
/// `Display` messages are German because they may be shown to the user.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The server rejected the credentials (401, or 403 during discovery).
    #[error("Anmeldung beim Kalenderanbieter fehlgeschlagen (Zugangsdaten prüfen)")]
    Unauthorized,

    /// `If-Match` / `If-None-Match` did not hold (412). The provider's version
    /// wins; the caller should refetch and report the conflict.
    #[error("Konflikt: Der Termin wurde beim Anbieter inzwischen geändert")]
    PreconditionFailed,

    /// The resource or collection does not exist (404/410).
    #[error("Ressource beim Anbieter nicht gefunden")]
    NotFound,

    /// The sync token is no longer valid; the caller must fall back to a full
    /// comparison (e.g. [`crate::Client::list_etags`]).
    #[error("Sync-Token ist nicht mehr gültig, vollständiger Abgleich nötig")]
    SyncTokenInvalid,

    /// The account lacks the privilege for this operation (403), e.g. writing
    /// into a read-only shared calendar.
    #[error("Keine Berechtigung für diese Aktion beim Anbieter{}", summary_suffix(.detail))]
    Forbidden {
        /// Short summary of the server's error body (may be empty).
        detail: String,
    },

    /// The server asked us to slow down (429, or 503 with `Retry-After`).
    #[error("Anbieter drosselt Anfragen, später erneut versuchen")]
    RateLimited {
        /// Delay requested by the server, if it sent `Retry-After` in seconds.
        retry_after: Option<Duration>,
    },

    /// Temporary failure: 5xx, timeout or network error. Retry later.
    #[error("Kalenderanbieter derzeit nicht erreichbar: {reason}")]
    Unavailable {
        /// What happened, for logs.
        reason: String,
    },

    /// Any other client error (4xx), e.g. 415 or `CALDAV:valid-calendar-data`.
    #[error("Anbieter hat die Anfrage abgelehnt (HTTP {status}){}", summary_suffix(.body))]
    Rejected {
        /// HTTP status code.
        status: u16,
        /// Short summary of the server's error body (precondition name or text).
        body: String,
    },

    /// The server answered in a way this client does not understand.
    #[error("Unerwartete Antwort vom Kalenderanbieter: {0}")]
    Protocol(String),

    /// The caller passed something unusable (bad URL, bad time range).
    #[error("Ungültige Eingabe: {0}")]
    InvalidInput(String),
}

fn summary_suffix(s: &str) -> String {
    if s.is_empty() {
        String::new()
    } else {
        format!(": {s}")
    }
}

impl Error {
    /// `true` for errors where trying again later may succeed
    /// ([`Error::RateLimited`], [`Error::Unavailable`]).
    pub fn is_retryable(&self) -> bool {
        matches!(self, Error::RateLimited { .. } | Error::Unavailable { .. })
    }

    pub(crate) fn from_reqwest(err: reqwest::Error) -> Self {
        let reason = if err.is_timeout() {
            "Zeitüberschreitung".to_string()
        } else if err.is_connect() {
            "Verbindung fehlgeschlagen".to_string()
        } else {
            "Netzwerkfehler".to_string()
        };
        // Include the source chain for logs without the URL (it may be long).
        let detail = err.without_url().to_string();
        Error::Unavailable {
            reason: format!("{reason} ({detail})"),
        }
    }
}

/// Convenience alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Summarises an error body: the first precondition element inside a
/// `DAV:error` document (`CALDAV:valid-calendar-data`), otherwise the
/// whitespace-collapsed text, cut to a few hundred characters.
pub(crate) fn summarize_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(doc) = roxmltree::Document::parse(trimmed) {
        let root = doc.root_element();
        if root.tag_name().namespace() == Some(crate::xml::NS_DAV)
            && root.tag_name().name() == "error"
        {
            let names: Vec<String> = root
                .children()
                .filter(|n| n.is_element())
                .map(|n| {
                    let prefix = match n.tag_name().namespace() {
                        Some(crate::xml::NS_DAV) => "DAV:",
                        Some(crate::xml::NS_CALDAV) => "CALDAV:",
                        Some(crate::xml::NS_CS) => "CS:",
                        _ => "",
                    };
                    let text = n.text().map(str::trim).unwrap_or_default();
                    if text.is_empty() {
                        format!("{prefix}{}", n.tag_name().name())
                    } else {
                        format!("{prefix}{} ({text})", n.tag_name().name())
                    }
                })
                .collect();
            if !names.is_empty() {
                return truncate(&names.join(", "));
            }
        }
        let text: Vec<&str> = doc
            .descendants()
            .filter(|n| n.is_text())
            .filter_map(|n| n.text())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect();
        return truncate(&text.join(" "));
    }
    truncate(&trimmed.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn truncate(s: &str) -> String {
    if s.chars().count() <= BODY_SUMMARY_LIMIT {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(BODY_SUMMARY_LIMIT).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_dav_error_precondition() {
        let body = r#"<?xml version="1.0"?><D:error xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><C:valid-calendar-data/></D:error>"#;
        assert_eq!(summarize_body(body), "CALDAV:valid-calendar-data");
    }

    #[test]
    fn summarizes_plain_text_and_truncates() {
        assert_eq!(summarize_body("  Bad\n  request  "), "Bad request");
        let long = "x".repeat(1000);
        assert_eq!(
            summarize_body(&long).chars().count(),
            BODY_SUMMARY_LIMIT + 1
        );
    }

    #[test]
    fn retryable_classification() {
        assert!(Error::RateLimited { retry_after: None }.is_retryable());
        assert!(Error::Unavailable { reason: "x".into() }.is_retryable());
        assert!(!Error::PreconditionFailed.is_retryable());
        assert!(!Error::Unauthorized.is_retryable());
        assert!(!Error::Rejected {
            status: 415,
            body: String::new()
        }
        .is_retryable());
    }

    #[test]
    fn display_is_german() {
        let e = Error::Rejected {
            status: 415,
            body: "CALDAV:supported-calendar-data".into(),
        };
        assert_eq!(
            e.to_string(),
            "Anbieter hat die Anfrage abgelehnt (HTTP 415): CALDAV:supported-calendar-data"
        );
    }
}
