//! `/healthz`, `/`, `/api/me`, API tokens, CSRF rule, dev auth and the
//! error format.

mod common;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use serde_json::json;

use common::{api_status, api_token, Auth, TestServer, BOOTSTRAP};

#[tokio::test]
async fn healthz_and_index_need_no_login() {
    let s = TestServer::new().await;
    let r = s.get("/healthz", Auth::None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["status"], "ok");
    assert_eq!(r.json()["version"], env!("CARGO_PKG_VERSION"));

    let r = s.get("/", Auth::None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.header("content-type").unwrap().starts_with("text/plain"));
    assert!(r.text().contains("Lichtung-Server"), "{}", r.text());

    // Security headers on every response.
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("x-frame-options"), Some("DENY"));

    // Unknown paths answer in the JSON error format.
    let r = s.get("/gibt-es-nicht", Auth::None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.error_code(), "not_found");
}

#[tokio::test]
async fn api_requires_authentication() {
    let s = TestServer::new().await;
    let r = s.get("/api/me", Auth::None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.error_code(), "unauthorized");
    assert_eq!(r.json()["error"]["message"], "Anmeldung erforderlich");
    assert_eq!(
        r.header("www-authenticate"),
        Some("Bearer realm=\"lichtung\"")
    );

    // Unknown or malformed credentials never fall back to anything else.
    for token in ["lt_unbekannt", "ld_unbekannt", "loa_unbekannt", "irgendwas"] {
        assert_eq!(
            api_status(&s, token).await,
            StatusCode::UNAUTHORIZED,
            "{token}"
        );
    }
    let req = Request::get("/api/me")
        .header(header::AUTHORIZATION, "Basic Zm9vOmJhcg==")
        .body(Body::empty())
        .unwrap();
    assert_eq!(s.send(req).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        s.get("/api/me", Auth::Cookie("erfunden")).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Unknown API paths behind the login.
    let r = s.get("/api/gibt-es-nicht", Auth::Bearer(BOOTSTRAP)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.json()["error"]["message"], "Unbekannter API-Pfad");
}

#[tokio::test]
async fn me_names_the_owner_and_the_credential() {
    let s = TestServer::new().await;
    let r = s.get("/api/me", Auth::Bearer(BOOTSTRAP)).await;
    assert_eq!(r.status, StatusCode::OK);
    let me = r.json();
    assert_eq!(me["user_id"], s.app.owner_id().to_string());
    assert_eq!(me["auth"], "api:bootstrap");
    assert!(me["device"].is_null());
    // Server name and version (contract section 7).
    assert_eq!(me["server_name"], "lichtung-server");
    assert_eq!(me["server_version"], env!("CARGO_PKG_VERSION"));
    let mut keys: Vec<&str> = me.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "auth",
            "device",
            "email",
            "name",
            "server_name",
            "server_version",
            "user_id"
        ]
    );

    let token = api_token(&s, "werkzeug").await;
    let r = s.get("/api/me", Auth::Bearer(&token)).await;
    assert_eq!(r.json()["auth"], "api:werkzeug");

    let session = s.session().await;
    let r = s.get("/api/me", Auth::Cookie(&session)).await;
    assert_eq!(r.json()["auth"], "web");
}

#[tokio::test]
async fn tokens_are_created_listed_and_revoked() {
    let s = TestServer::new().await;
    let session = s.session().await;

    let r = s
        .post(
            "/api/tokens",
            Auth::Web(&session),
            json!({"name": " claude-code "}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let created = r.json();
    assert_eq!(created["name"], "claude-code");
    let token = created["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("lt_"));
    assert!(created["last_used_at"].is_null());
    let id = created["id"].as_str().unwrap().to_owned();

    // The token works for the API and would work for /mcp.
    assert_eq!(api_status(&s, &token).await, StatusCode::OK);
    assert!(common::mcp_accepts(&s, &token).await);

    // The list never shows the secret but the last use.
    let list = s.get("/api/tokens", Auth::Cookie(&session)).await.json();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], id.as_str());
    assert!(list[0].get("token").is_none());
    assert!(list[0]["last_used_at"].is_string());

    // Names are unique among active tokens, 1–64 characters, no "bootstrap".
    for (name, status, code) in [
        ("claude-code", StatusCode::CONFLICT, "conflict"),
        ("", StatusCode::BAD_REQUEST, "invalid_request"),
        ("   ", StatusCode::BAD_REQUEST, "invalid_request"),
        ("bootstrap", StatusCode::BAD_REQUEST, "invalid_request"),
        ("a\u{7}b", StatusCode::BAD_REQUEST, "invalid_request"),
    ] {
        let r = s
            .post("/api/tokens", Auth::Web(&session), json!({"name": name}))
            .await;
        assert_eq!(r.status, status, "{name:?}: {}", r.text());
        assert_eq!(r.error_code(), code, "{name:?}");
    }
    let r = s
        .post(
            "/api/tokens",
            Auth::Web(&session),
            json!({"name": "x".repeat(65)}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = s
        .post("/api/tokens", Auth::Web(&session), json!({"nom": "x"}))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("Ungültiger JSON-Body"));

    // Revoke.
    let path = format!("/api/tokens/{id}");
    assert_eq!(
        s.delete(&path, Auth::Web(&session)).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(api_status(&s, &token).await, StatusCode::UNAUTHORIZED);
    assert!(!common::mcp_accepts(&s, &token).await);
    let r = s.delete(&path, Auth::Web(&session)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.json()["error"]["message"], "Token nicht gefunden");
    let r = s
        .delete("/api/tokens/keine-uuid", Auth::Web(&session))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    // A revoked token's name is free again.
    let r = s
        .post(
            "/api/tokens",
            Auth::Web(&session),
            json!({"name": "claude-code"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED);

    // Tokens survive a restart, sessions too.
    let s = s.restart().await;
    let list = s.get("/api/tokens", Auth::Cookie(&session)).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.json().as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn only_browsers_and_devices_manage_credentials() {
    let s = TestServer::new().await;
    let token = api_token(&s, "werkzeug").await;
    for auth in [Auth::Bearer(BOOTSTRAP), Auth::Bearer(&token)] {
        for path in ["/api/tokens", "/api/devices", "/api/oauth/clients"] {
            let r = s.get(path, auth).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{path}");
            assert_eq!(r.error_code(), "forbidden");
        }
        let r = s.post("/api/tokens", auth, json!({"name": "neu"})).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn cookie_requests_that_change_state_need_the_client_header() {
    let s = TestServer::new().await;
    let session = s.session().await;
    // Without X-Lichtung-Client: refused (CSRF).
    let r = s
        .post(
            "/api/tokens",
            Auth::Cookie(&session),
            json!({"name": "csrf"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("X-Lichtung-Client"));
    // Reading needs no header; bearer tokens never need it.
    assert_eq!(
        s.get("/api/tokens", Auth::Cookie(&session)).await.status,
        StatusCode::OK
    );
    assert_eq!(
        s.post("/api/tokens", Auth::Web(&session), json!({"name": "csrf"}))
            .await
            .status,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn logout_ends_the_session() {
    let s = TestServer::new().await;
    let session = s.session().await;
    // CSRF header required.
    let r = s
        .request(Method::POST, "/auth/logout", Auth::Cookie(&session), None)
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = s
        .request(Method::POST, "/auth/logout", Auth::Web(&session), None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(r.cookie("lichtung_session").as_deref(), Some(""));
    assert_eq!(
        s.get("/api/me", Auth::Cookie(&session)).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn login_without_oidc_is_reported() {
    let s = TestServer::new().await;
    let r = s.get("/auth/login", Auth::None).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(r.text().contains("LICHTUNG_OIDC_ISSUER"));
}

#[tokio::test]
async fn dev_auth_applies_on_loopback_only() {
    let s = TestServer::with_config(|c| c.dev_auth = true).await;
    let r = s.get("/api/me", Auth::None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["email"], "dev@localhost");
    assert_eq!(r.json()["auth"], "web");

    // Dev auth counts as a browser: state changes need the CSRF header.
    let r = s
        .post("/api/tokens", Auth::None, json!({"name": "dev"}))
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let req = Request::post("/api/tokens")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-lichtung-client", "test")
        .body(Body::from(json!({"name": "dev"}).to_string()))
        .unwrap();
    assert_eq!(s.send(req).await.status, StatusCode::CREATED);

    // A request for another host name (other machine, DNS rebinding) is
    // not the owner.
    for host in [
        "lichtung.example.org",
        "192.0.2.10:8080",
        "localhost.example.com",
    ] {
        let req = Request::get("/api/me")
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap();
        assert_eq!(s.send(req).await.status, StatusCode::UNAUTHORIZED, "{host}");
    }
    for host in ["127.0.0.1:8080", "[::1]:8080", "LOCALHOST"] {
        let req = Request::get("/api/me")
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap();
        assert_eq!(s.send(req).await.status, StatusCode::OK, "{host}");
    }

    // /auth/login creates a session and returns to the local path.
    let r = s.get("/auth/login?return_to=/zurueck", Auth::None).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.header("location"), Some("/zurueck"));
    assert!(r.cookie("lichtung_session").is_some());
    let req = Request::get("/auth/login")
        .header(header::HOST, "lichtung.example.org")
        .body(Body::empty())
        .unwrap();
    assert_eq!(s.send(req).await.status, StatusCode::FORBIDDEN);
}
