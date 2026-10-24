//! MCP endpoint (LIC-12): Streamable HTTP under `/mcp`, hand-rolled
//! JSON-RPC 2.0. Transport, protocol versions and error forms follow
//! Zunder's MCP endpoint.
//!
//! - `POST /mcp` takes one JSON-RPC message or a batch (array). Requests get
//!   `200 application/json`; notifications and responses alone get `202`
//!   without body. There is no server-initiated stream: `GET` and `DELETE`
//!   answer `405`.
//! - Stateless: no `Mcp-Session-Id` is issued; a header sent by the client
//!   is ignored.
//! - Auth: bearer token only (API token `lt_…`, bootstrap token or OAuth
//!   access token `loa_…` of [`crate::oauth`]); device tokens (`ld_…`),
//!   session cookies and dev auth are not accepted. A `401` names the
//!   protected resource metadata in `WWW-Authenticate` (MCP authorization).
//!   Writes carry the origin `mcp:<token-name>` (`mcp:oauth:<client_name>`
//!   for OAuth clients) and are subject to the calendar setting `mcp_write`.
//! - A present `Origin` header must match `LICHTUNG_PUBLIC_URL` (protection
//!   against DNS rebinding); a present `MCP-Protocol-Version` header must
//!   name a supported version.
//! - Tool failures are results with `isError: true` and a German message;
//!   protocol failures are JSON-RPC errors.
//!
//! The tools are described in `docs/mcp.md`; names and schemas are frozen
//! by the snapshot test `tests/snapshots/mcp_tools.json`.

pub mod args;
pub mod free;
pub mod schema;
pub mod time;
pub mod tools;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::header::WWW_AUTHENTICATE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::auth::middleware::bearer_token;
use crate::auth::{Principal, PrincipalKind, BOOTSTRAP_TOKEN_NAME};
use crate::error::AppError;
use crate::oauth;
use crate::writes::Origin;

/// The protocol version this server speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// All accepted protocol versions (newest first).
pub const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26"];
/// Body limit of `POST /mcp`.
pub const MCP_BODY_LIMIT: usize = 1024 * 1024;

/// JSON-RPC error codes.
pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
}

const INSTRUCTIONS: &str = "Lichtung ist der Kalender einer Person: Termine aus mehreren Konten \
(Fastmail, iCloud, CalDAV, ICS-Abos, später Microsoft) in Kalendern; Kalender lassen sich in \
Kalender-Sets gruppieren. IDs liefern list_calendars und list_calendar_sets. \
Lesen: query_events liefert Termine (Serien als einzelne Vorkommen) in einem Zeitraum, mit \
Suchbegriff gefiltert; find_free_times sucht freie Zeiten; list_invitations zeigt offene \
Terminanfragen. \
Schreiben: create_event braucht immer calendar_id, es gibt keinen Standardkalender. Vorher mit \
list_calendars einen Kalender mit mcp_writable = true wählen; im Zweifel den Nutzer fragen. \
Kalender mit mcp_writable = false (z. B. der Arbeitskalender) sind für Claude gesperrt, auch für \
respond_to_invitation; freigeben kann das nur der Nutzer in der App. \
Zeitangaben: immer RFC 3339 mit Offset (2026-11-03T14:00:00+01:00 oder …Z), ganztägige Termine \
als Datum JJJJ-MM-TT; bei ganztägigen Terminen ist end der letzte Tag (einschließlich). \
time_zone (IANA, z. B. Europe/Berlin) legt bei Abfragen die Zeitzone der Ausgabe fest und beim \
Anlegen die Zeitzone des Termins (wichtig für Serien über die Zeitumstellung). \
Serien: update_event und delete_event brauchen bei Serien scope (this = nur dieses Vorkommen, \
this_and_following = dieses und alle folgenden, all = die ganze Serie) und bei this bzw. \
this_and_following occurrence = recurrence_id aus query_events. \
Konflikte: Hat sich ein Termin inzwischen beim Anbieter geändert, gewinnt der Anbieter; die \
Änderung wird nicht übernommen und das Ergebnis zeigt den aktuellen Stand. Dann neu entscheiden, \
nicht blind wiederholen. Teilnehmer bekommen echte Einladungen vom Anbieter. \
Abgelehnte Aufrufe kommen mit deutscher Begründung und einem Hinweis, wie es richtig geht.";

/// The `/mcp` route (without state).
pub fn routes() -> Router<App> {
    Router::new()
        .route("/mcp", post(handle_post))
        .layer(DefaultBodyLimit::max(MCP_BODY_LIMIT))
}

/// The origin of writes made through `/mcp` by this principal:
/// `mcp:<token-name>`, `mcp:oauth:<client_name>`.
pub fn origin_for(principal: &Principal) -> Origin {
    Origin::Mcp(match &principal.kind {
        PrincipalKind::Token { name, .. } => name.clone(),
        PrincipalKind::Bootstrap => BOOTSTRAP_TOKEN_NAME.to_owned(),
        PrincipalKind::OAuth { client_name, .. } => format!("oauth:{client_name}"),
        // Not accepted by `/mcp`; labelled anyway.
        _ => principal.label(),
    })
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn check_origin(app: &App, headers: &HeaderMap) -> Result<(), AppError> {
    let Some(origin) = headers.get(axum::http::header::ORIGIN) else {
        return Ok(());
    };
    let allowed = app.config().public_origin();
    if origin.to_str().ok() == Some(allowed.as_str()) {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Origin ist für /mcp nicht erlaubt".into(),
        ))
    }
}

/// `401` with the MCP authorization challenge: `WWW-Authenticate: Bearer
/// resource_metadata="…"` plus `error="invalid_token"` when a token was
/// presented but not accepted.
fn challenge(app: &App, error: Option<&str>) -> Response {
    let mut response = AppError::Unauthorized.into_response();
    let mut value = format!(
        "Bearer resource_metadata=\"{}\"",
        oauth::resource_metadata_url(app.config())
    );
    if let Some(error) = error {
        value.push_str(&format!(", error=\"{error}\""));
    }
    if let Ok(v) = HeaderValue::from_str(&value) {
        response.headers_mut().insert(WWW_AUTHENTICATE, v);
    }
    response
}

/// Authenticates `/mcp` with [`oauth::authenticate_mcp`] (OAuth access,
/// API and bootstrap tokens; device tokens are refused).
async fn authenticate(app: &App, headers: &HeaderMap) -> Result<Principal, Response> {
    let token = match bearer_token(headers) {
        Ok(Some(token)) => token,
        Ok(None) => return Err(challenge(app, None)),
        Err(_) => return Err(challenge(app, Some("invalid_request"))),
    };
    match oauth::authenticate_mcp(app, token).await {
        Ok(Some(principal)) => Ok(principal),
        Ok(None) => Err(challenge(app, Some("invalid_token"))),
        Err(e) => Err(e.into_response()),
    }
}

async fn handle_post(
    State(app): State<App>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    if let Err(e) = check_origin(&app, &headers) {
        return e.into_response();
    }
    let principal = match authenticate(&app, &headers).await {
        Ok(p) => p,
        Err(response) => return response,
    };
    if let Some(version) = headers.get("mcp-protocol-version") {
        let supported = version
            .to_str()
            .is_ok_and(|v| SUPPORTED_VERSIONS.contains(&v));
        if !supported {
            let message = format!(
                "Nicht unterstützte MCP-Protokollversion. Unterstützt: {}",
                SUPPORTED_VERSIONS.join(", ")
            );
            return (
                StatusCode::BAD_REQUEST,
                Json(rpc_error(Value::Null, codes::INVALID_REQUEST, message)),
            )
                .into_response();
        }
    }
    let body = match body {
        Ok(b) => b,
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return AppError::PayloadTooLarge("Die Anfrage ist zu groß".into()).into_response()
        }
        Err(rejection) => return AppError::bad_request(rejection.body_text()).into_response(),
    };
    let message: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(rpc_error(
                    Value::Null,
                    codes::PARSE_ERROR,
                    format!("Ungültiges JSON: {e}"),
                )),
            )
                .into_response()
        }
    };

    let origin = origin_for(&principal);
    let reply = match message {
        Value::Array(items) if items.is_empty() => Some(rpc_error(
            Value::Null,
            codes::INVALID_REQUEST,
            "Leerer Batch",
        )),
        Value::Array(items) => {
            let mut responses = Vec::new();
            for item in items {
                if let Some(r) = handle_message(&app, &origin, item).await {
                    responses.push(r);
                }
            }
            (!responses.is_empty()).then_some(Value::Array(responses))
        }
        single => handle_message(&app, &origin, single).await,
    };
    match reply {
        Some(body) => Json(body).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// Handles one message; `None` for notifications and client responses.
async fn handle_message(app: &App, origin: &Origin, message: Value) -> Option<Value> {
    let Value::Object(obj) = message else {
        return Some(rpc_error(
            Value::Null,
            codes::INVALID_REQUEST,
            "Eine JSON-RPC-Nachricht muss ein Objekt sein",
        ));
    };
    let id = obj.get("id").cloned();
    let valid_id = matches!(id, Some(Value::String(_)) | Some(Value::Number(_)));
    let reply_id = match &id {
        Some(v) if valid_id => v.clone(),
        _ => Value::Null,
    };
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(
            reply_id,
            codes::INVALID_REQUEST,
            "jsonrpc muss \"2.0\" sein",
        ));
    }
    let Some(method) = obj.get("method") else {
        // A response from the client (we never send requests): ignore.
        if id.is_some() && (obj.contains_key("result") || obj.contains_key("error")) {
            return None;
        }
        return Some(rpc_error(reply_id, codes::INVALID_REQUEST, "method fehlt"));
    };
    let Some(method) = method.as_str() else {
        return Some(rpc_error(
            reply_id,
            codes::INVALID_REQUEST,
            "method muss ein Text sein",
        ));
    };
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    match id {
        // Notification (initialized, cancelled …): nothing to answer.
        None => None,
        Some(_) if !valid_id => Some(rpc_error(
            Value::Null,
            codes::INVALID_REQUEST,
            "id muss ein Text oder eine Zahl sein",
        )),
        Some(id) => Some(handle_request(app, origin, id, method, params).await),
    }
}

async fn handle_request(
    app: &App,
    origin: &Origin,
    id: Value,
    method: &str,
    params: Value,
) -> Value {
    if !(params.is_null() || params.is_object()) {
        return rpc_error(id, codes::INVALID_PARAMS, "params muss ein Objekt sein");
    }
    match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str);
            let version = requested
                .filter(|v| SUPPORTED_VERSIONS.contains(v))
                .unwrap_or(PROTOCOL_VERSION);
            rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {
                        "name": "lichtung",
                        "title": "Lichtung",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, tools::list()),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return rpc_error(id, codes::INVALID_PARAMS, "params.name fehlt");
            };
            let args = match params.get("arguments") {
                None | Some(Value::Null) => Map::new(),
                Some(Value::Object(map)) => map.clone(),
                Some(_) => {
                    return rpc_error(
                        id,
                        codes::INVALID_PARAMS,
                        "params.arguments muss ein Objekt sein",
                    )
                }
            };
            match tools::call(app, origin, name, args).await {
                Some(result) => rpc_result(id, result),
                None => rpc_error(
                    id,
                    codes::INVALID_PARAMS,
                    format!("Unbekanntes Werkzeug: {name}"),
                ),
            }
        }
        other => rpc_error(
            id,
            codes::METHOD_NOT_FOUND,
            format!("Unbekannte Methode: {other}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_name_token_or_client() {
        let p = |kind| Principal {
            user_id: uuid::Uuid::nil(),
            kind,
        };
        let token = p(PrincipalKind::Token {
            id: uuid::Uuid::nil(),
            name: "claude-code".into(),
        });
        assert_eq!(origin_for(&token).label(), "mcp:claude-code");
        assert_eq!(
            origin_for(&p(PrincipalKind::Bootstrap)).label(),
            "mcp:bootstrap"
        );
        let oauth = p(PrincipalKind::OAuth {
            client_id: "c1".into(),
            client_name: "Claude".into(),
        });
        assert_eq!(origin_for(&oauth).label(), "mcp:oauth:Claude");
    }
}
