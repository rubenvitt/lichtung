//! Silent push notifications (APNs, LIC-27): registering the token of a
//! device, the push to APNs (against a mock server, with an invented key)
//! and the notifier that only pushes after event changes. All data is made
//! up.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use base64::Engine;
use serde_json::json;
use wiremock::matchers::{header as has_header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::dav::{setup_configured, single};
use common::{api_token, device_token, Auth, TestServer};
use lichtung_server::apns::{self, ApnsClient, Delivery};
use lichtung_server::config::ApnsConfig;
use lichtung_server::db::{self, PushTarget};

const KEY_ID: &str = "ABC123DEFG";
const TEAM_ID: &str = "TEAM456XYZ";
const TOPIC: &str = "dev.rubeen.lichtung";

/// An invented P-256 key as a `.p8` file would contain it.
fn p8_key() -> String {
    let rng = ring::rand::SystemRandom::new();
    let der = ring::signature::EcdsaKeyPair::generate_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        &rng,
    )
    .unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(der.as_ref());
    let lines: Vec<&str> = b64
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        lines.join("\n")
    )
}

fn apns_config(production: &MockServer, sandbox: &MockServer) -> ApnsConfig {
    let mut c = ApnsConfig::new(p8_key(), KEY_ID.into(), TEAM_ID.into(), TOPIC.into());
    c.production_url = production.uri().parse().unwrap();
    c.sandbox_url = sandbox.uri().parse().unwrap();
    c.debounce = Duration::from_millis(50);
    c.min_interval = Duration::ZERO;
    c
}

fn hex_token(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

async fn put_push(s: &TestServer, auth: Auth<'_>, body: serde_json::Value) -> StatusCode {
    let r = s
        .request(Method::PUT, "/api/devices/current/push", auth, Some(body))
        .await;
    r.status
}

fn targets(s: &TestServer) -> Vec<PushTarget> {
    let conn = rusqlite::Connection::open(s.app.config().db_path()).unwrap();
    db::push_targets(&conn).unwrap()
}

async fn device_id(s: &TestServer, token: &str) -> uuid::Uuid {
    let me = s.get("/api/me", Auth::Bearer(token)).await.json();
    me["device"]["id"].as_str().unwrap().parse().unwrap()
}

#[tokio::test]
async fn devices_register_and_remove_their_token() {
    let s = TestServer::new().await;
    let phone = device_token(&s, "iPhone");
    let token = "AB".repeat(32);

    assert_eq!(
        put_push(
            &s,
            Auth::Bearer(&phone),
            json!({"apns_token": token, "environment": "production"})
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let id = device_id(&s, &phone).await;
    assert_eq!(
        targets(&s),
        vec![PushTarget {
            device_id: id,
            token: "ab".repeat(32),
            environment: "production".into()
        }]
    );

    // A new login on the same phone takes the token over.
    let again = device_token(&s, "iPhone (neu)");
    assert_eq!(
        put_push(
            &s,
            Auth::Bearer(&again),
            json!({"apns_token": token, "environment": "sandbox"})
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let all = targets(&s);
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].device_id, device_id(&s, &again).await);
    assert_eq!(all[0].environment, "sandbox");

    // Turning it off; twice is fine.
    for _ in 0..2 {
        let r = s
            .request(
                Method::DELETE,
                "/api/devices/current/push",
                Auth::Bearer(&again),
                None,
            )
            .await;
        assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.text());
    }
    assert!(targets(&s).is_empty());
}

#[tokio::test]
async fn only_devices_register_and_bodies_are_checked() {
    let s = TestServer::new().await;
    let tool = api_token(&s, "werkzeug").await;
    let body = json!({"apns_token": hex_token(1), "environment": "production"});
    let r = s
        .request(
            Method::PUT,
            "/api/devices/current/push",
            Auth::Bearer(&tool),
            Some(body),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.error_code(), "forbidden");

    let phone = device_token(&s, "iPhone");
    for bad in [
        json!({"apns_token": "kein-hex", "environment": "production"}),
        json!({"apns_token": hex_token(1), "environment": "development"}),
        json!({"apns_token": hex_token(1)}),
        json!({"apns_token": hex_token(1), "environment": "sandbox", "extra": 1}),
    ] {
        let r = s
            .request(
                Method::PUT,
                "/api/devices/current/push",
                Auth::Bearer(&phone),
                Some(bad.clone()),
            )
            .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}: {}", r.text());
        assert_eq!(r.error_code(), "invalid_request", "{bad}");
    }
    assert!(targets(&s).is_empty());
}

#[tokio::test]
async fn pushes_go_out_and_apns_answers_are_applied() {
    let production = MockServer::start().await;
    let sandbox = MockServer::start().await;
    let config = apns_config(&production, &sandbox);
    let s = TestServer::new().await;

    // 1: valid in production. 2: registered as sandbox, but a TestFlight
    // build (production). 3: no longer installed.
    let phones: Vec<String> = (1..=3)
        .map(|n| device_token(&s, &format!("Gerät {n}")))
        .collect();
    let envs = ["production", "sandbox", "production"];
    for (n, (phone, env)) in phones.iter().zip(envs).enumerate() {
        let n = u8::try_from(n + 1).unwrap();
        assert_eq!(
            put_push(
                &s,
                Auth::Bearer(phone),
                json!({"apns_token": hex_token(n), "environment": env})
            )
            .await,
            StatusCode::NO_CONTENT
        );
    }
    let ok = || ResponseTemplate::new(200);
    Mock::given(method("POST"))
        .and(path(format!("/3/device/{}", hex_token(1))))
        .and(has_header("apns-topic", TOPIC))
        .and(has_header("apns-push-type", "background"))
        .and(has_header("apns-priority", "5"))
        .respond_with(ok())
        .expect(1)
        .mount(&production)
        .await;
    Mock::given(path(format!("/3/device/{}", hex_token(2))))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"reason": "BadDeviceToken"})))
        .expect(1)
        .mount(&sandbox)
        .await;
    Mock::given(path(format!("/3/device/{}", hex_token(2))))
        .respond_with(ok())
        .expect(1)
        .mount(&production)
        .await;
    Mock::given(path(format!("/3/device/{}", hex_token(3))))
        .respond_with(
            ResponseTemplate::new(410)
                .set_body_json(json!({"reason": "Unregistered", "timestamp": 1})),
        )
        .expect(1)
        .mount(&production)
        .await;
    let client = ApnsClient::new(config).unwrap();
    let sent = apns::notify_devices(&s.app, &client).await.unwrap();
    assert_eq!(sent, 2);

    // Device 2 now counts as production, device 3 is gone.
    let left: Vec<(String, String)> = targets(&s)
        .into_iter()
        .map(|t| (t.token, t.environment))
        .collect();
    assert_eq!(
        left,
        vec![
            (hex_token(1), "production".into()),
            (hex_token(2), "production".into()),
        ]
    );

    // The provider token: ES256 with key and team ID.
    let request = &production.received_requests().await.unwrap()[0];
    let auth = request.headers["authorization"].to_str().unwrap();
    let jwt = auth.strip_prefix("bearer ").expect("bearer token");
    let head = jsonwebtoken::decode_header(jwt).unwrap();
    assert_eq!(head.alg, jsonwebtoken::Algorithm::ES256);
    assert_eq!(head.kid.as_deref(), Some(KEY_ID));
    let claims: serde_json::Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(jwt.split('.').nth(1).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(claims["iss"], TEAM_ID);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&request.body).unwrap(),
        json!({"aps": {"content-available": 1}})
    );
}

#[tokio::test]
async fn rejected_provider_tokens_are_reported() {
    let production = MockServer::start().await;
    let sandbox = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"reason": "InvalidProviderToken"})),
        )
        .mount(&production)
        .await;
    let client = ApnsClient::new(apns_config(&production, &sandbox)).unwrap();
    assert_eq!(
        client.send(&hex_token(9), "production").await,
        Delivery::Unauthorized("InvalidProviderToken".into())
    );
}

#[tokio::test]
async fn the_notifier_pushes_after_event_changes_only() {
    let production = MockServer::start().await;
    let sandbox = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&production)
        .await;
    let config = apns_config(&production, &sandbox);
    let t = setup_configured(|_| {}, move |c| c.apns = Some(config)).await;
    let phone = device_token(&t.s, "iPhone");
    assert_eq!(
        put_push(
            &t.s,
            Auth::Bearer(&phone),
            json!({"apns_token": hex_token(7), "environment": "production"})
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let notifier = tokio::spawn(apns::run_notifier(t.s.app.clone()));
    // Let it read the starting point.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let pushes = || async { production.received_requests().await.unwrap().len() };
    let wait_for = |n: usize| async move {
        for _ in 0..100 {
            if pushes().await >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };

    // A sync run without new events moves the cursor (account status), but
    // wakes nobody.
    let before = t.s.app.change_cursor();
    t.sync().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(t.s.app.change_cursor() > before);
    assert_eq!(pushes().await, 0);

    // A new event (say, created through MCP at the provider) does.
    t.dav
        .put("frueh.ics", &single("frueh@example.org", "Fokuszeit"));
    t.sync().await;
    wait_for(1).await;
    assert_eq!(pushes().await, 1);

    t.s.app.shutdown_token().cancel();
    tokio::time::timeout(Duration::from_secs(5), notifier)
        .await
        .expect("notifier stops on shutdown")
        .unwrap();
}
