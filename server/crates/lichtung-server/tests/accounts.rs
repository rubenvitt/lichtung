//! Accounts, calendars and calendar sets against an invented, Fastmail-style
//! CalDAV server (wiremock): creating accounts, provider errors, calendar
//! refresh with tombstones, settings, sets, access rules, and that
//! passwords never leave the server.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use base64::Engine;
use lichtung_server::changes;
use serde_json::{json, Value};
use url::Url;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{api_token, Auth, TestServer, BOOTSTRAP};

const USER: &str = "anna@example.org";
const PASSWORD: &str = "app-passwort-1234";
const XML: &str = "application/xml; charset=utf-8";
const HOME: &str = "/dav/calendars/user/anna%40example.org/";

/// An invented calendar collection in the mock's calendar home.
#[derive(Clone)]
struct FakeCalendar {
    slug: &'static str,
    name: &'static str,
    color: Option<&'static str>,
    order: Option<i64>,
    writable: bool,
    shared: bool,
    components: &'static [&'static str],
}

fn cal(slug: &'static str, name: &'static str) -> FakeCalendar {
    FakeCalendar {
        slug,
        name,
        color: None,
        order: None,
        writable: true,
        shared: false,
        components: &["VEVENT"],
    }
}

fn default_calendars() -> Vec<FakeCalendar> {
    vec![
        FakeCalendar {
            color: Some("#2E8B57"),
            writable: false,
            shared: true,
            ..cal("Team-Sync", "Team-Sync")
        },
        FakeCalendar {
            color: Some("#3A7BD5FF"),
            order: Some(1),
            components: &["VEVENT", "VTODO"],
            ..cal("Privat", "Privat")
        },
        FakeCalendar {
            components: &["VTODO"],
            ..cal("Aufgaben", "Aufgaben")
        },
    ]
}

fn basic(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
}

fn ms(body: String) -> ResponseTemplate {
    ResponseTemplate::new(207).set_body_raw(body, XML)
}

fn calendar_listing(calendars: &[FakeCalendar]) -> String {
    let mut out = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:CS="http://calendarserver.org/ns/" xmlns:A="http://apple.com/ns/ical/">
<D:response><D:href>{HOME}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"#
    );
    for c in calendars {
        let shared = if c.shared { "<CS:shared/>" } else { "" };
        let owner = if c.shared {
            "/dav/principals/user/ben%40example.org/"
        } else {
            "/dav/principals/user/anna%40example.org/"
        };
        let privileges = if c.writable {
            "<D:privilege><D:read/></D:privilege><D:privilege><D:write/></D:privilege>"
        } else {
            "<D:privilege><D:read/></D:privilege>"
        };
        let comps: String = c
            .components
            .iter()
            .map(|n| format!(r#"<C:comp name="{n}"/>"#))
            .collect();
        let color = c
            .color
            .map(|v| format!("<A:calendar-color>{v}</A:calendar-color>"))
            .unwrap_or_default();
        let order = c
            .order
            .map(|v| format!("<A:calendar-order>{v}</A:calendar-order>"))
            .unwrap_or_default();
        out.push_str(&format!(
            r#"<D:response><D:href>{HOME}{slug}/</D:href><D:propstat><D:prop>
<D:resourcetype><D:collection/><C:calendar/>{shared}</D:resourcetype>
<D:displayname>{name}</D:displayname>
<D:owner><D:href>{owner}</D:href></D:owner>
<D:current-user-privilege-set>{privileges}</D:current-user-privilege-set>
<D:sync-token>https://example.org/ns/sync/1</D:sync-token>
<C:supported-calendar-component-set>{comps}</C:supported-calendar-component-set>
<CS:getctag>1</CS:getctag>{color}{order}
</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"#,
            slug = c.slug,
            name = c.name,
        ));
    }
    out.push_str("</D:multistatus>");
    out
}

/// Mounts an invented Fastmail-style account that accepts `password`.
async fn mount_provider(server: &MockServer, password: &str, calendars: &[FakeCalendar]) {
    server.reset().await;
    let auth = basic(USER, password);
    let principal_ms = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:"><D:response><D:href>/dav/</D:href><D:propstat><D:prop><D:current-user-principal><D:href>/dav/principals/user/anna%40example.org/</D:href></D:current-user-principal></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
    Mock::given(method("PROPFIND"))
        .and(path("/.well-known/caldav"))
        .and(header("authorization", auth.as_str()))
        .respond_with(ms(principal_ms.to_owned()))
        .mount(server)
        .await;
    Mock::given(method("PROPFIND"))
        .and(path("/dav/principals/user/anna@example.org/"))
        .and(header("authorization", auth.as_str()))
        .and(body_string_contains("calendar-home-set"))
        .respond_with(ms(format!(
            r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/dav/principals/user/anna%40example.org/</D:href><D:propstat><D:prop><C:calendar-home-set><D:href>{HOME}</D:href></C:calendar-home-set><C:calendar-user-address-set><D:href>mailto:{USER}</D:href></C:calendar-user-address-set><D:displayname>Anna Beispiel</D:displayname></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#
        )))
        .mount(server)
        .await;
    Mock::given(method("OPTIONS"))
        .and(path("/dav/calendars/user/anna@example.org/"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("DAV", "1, 2, 3, calendar-access, calendar-auto-schedule"),
        )
        .mount(server)
        .await;
    Mock::given(method("PROPFIND"))
        .and(path("/dav/calendars/user/anna@example.org/"))
        .and(header("authorization", auth.as_str()))
        .and(header("depth", "1"))
        .respond_with(ms(calendar_listing(calendars)))
        .mount(server)
        .await;
    // Anything else (wrong password, no credentials) is refused.
    Mock::given(method("PROPFIND"))
        .respond_with(ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Basic"))
        .with_priority(10)
        .mount(server)
        .await;
}

async fn setup() -> (TestServer, MockServer, String) {
    let provider = MockServer::start().await;
    mount_provider(&provider, PASSWORD, &default_calendars()).await;
    let base = Url::parse(&format!("{}/dav/", provider.uri())).unwrap();
    let s = TestServer::with_config(|c| c.fastmail_url = base).await;
    let session = s.session().await;
    (s, provider, session)
}

fn fastmail_body() -> Value {
    json!({"kind": "fastmail", "username": USER, "password": PASSWORD, "name": "Fastmail"})
}

async fn create_account(s: &TestServer, session: &str) -> Value {
    let r = s
        .post("/api/accounts", Auth::Web(session), fastmail_body())
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert!(!r.text().contains(PASSWORD));
    r.json()
}

async fn calendars(s: &TestServer) -> Vec<Value> {
    let r = s.get("/api/calendars", Auth::Bearer(BOOTSTRAP)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    r.json().as_array().unwrap().clone()
}

fn by_name<'a>(cals: &'a [Value], name: &str) -> &'a Value {
    cals.iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no calendar {name}: {cals:?}"))
}

async fn patch(s: &TestServer, path: &str, auth: Auth<'_>, body: Value) -> common::TestResponse {
    s.request(Method::PATCH, path, auth, Some(body)).await
}

fn db(s: &TestServer) -> rusqlite::Connection {
    rusqlite::Connection::open(s.app.config().db_path()).unwrap()
}

fn tombstones(s: &TestServer) -> Vec<(String, String)> {
    changes::tombstones_since(&db(s), 0)
        .unwrap()
        .into_iter()
        .map(|t| (t.kind, t.id))
        .collect()
}

#[tokio::test]
async fn fastmail_account_is_created_with_its_event_calendars() {
    let (s, _provider, session) = setup().await;
    let mut cursor = s.app.subscribe_changes();
    assert_eq!(*cursor.borrow_and_update(), 0);

    let account = create_account(&s, &session).await;
    let id = account["id"].as_str().unwrap();
    assert!(id.starts_with("acc_") && id.len() == 36, "{id}");
    assert_eq!(
        account,
        json!({
            "id": id,
            "kind": "fastmail",
            "name": "Fastmail",
            "username": USER,
            "status": {"state": "ok", "message": null, "last_sync_at": account["status"]["last_sync_at"]}
        })
    );
    assert!(account["status"]["last_sync_at"]
        .as_str()
        .unwrap()
        .ends_with('Z'));

    // Account + two event calendars, the VTODO-only collection is skipped.
    assert!(cursor.has_changed().unwrap());
    assert_eq!(*cursor.borrow_and_update(), 3);
    assert_eq!(s.app.change_cursor(), 3);

    let cals = calendars(&s).await;
    assert_eq!(cals.len(), 2, "{cals:?}");
    // Ordered by calendar-order, then as listed.
    assert_eq!(cals[0]["name"], "Privat");
    assert_eq!(cals[1]["name"], "Team-Sync");
    let privat = &cals[0];
    assert!(privat["id"].as_str().unwrap().starts_with("cal_"));
    assert_eq!(
        privat,
        &json!({
            "id": privat["id"],
            "account_id": id,
            "name": "Privat",
            "color": "#3A7BD5",
            "provider_color": "#3A7BD5",
            "writable": true,
            "can_invite": true,
            "hidden": false,
            "mcp_write": true,
            "openings_busy": true,
            "all_day_blocks": false,
            "position": 0
        })
    );
    let team = &cals[1];
    assert_eq!(team["writable"], false);
    assert_eq!(team["can_invite"], false);
    assert_eq!(team["mcp_write"], false);
    assert_eq!(team["position"], 1);

    let list = s.get("/api/accounts", Auth::Bearer(BOOTSTRAP)).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.json(), json!([account]));

    // Stored encrypted (v1 || nonce || ciphertext+tag), never in plain text.
    let blob: Vec<u8> = db(&s)
        .query_row(
            "SELECT credentials FROM accounts WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(blob[0], 1);
    assert!(!blob
        .windows(PASSWORD.len())
        .any(|w| w == PASSWORD.as_bytes()));
    let shared: bool = db(&s)
        .query_row(
            "SELECT shared FROM calendars WHERE name = 'Team-Sync'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(shared);

    // The same account cannot be connected twice.
    let r = s
        .post("/api/accounts", Auth::Web(&session), fastmail_body())
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text());
}

#[tokio::test]
async fn provider_errors_map_to_api_errors() {
    let (s, provider, session) = setup().await;

    // Wrong password → 422.
    let mut body = fastmail_body();
    body["password"] = json!("falsches-passwort");
    let r = s.post("/api/accounts", Auth::Web(&session), body).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    assert_eq!(r.error_code(), "invalid_credentials");
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Fastmail"));
    assert!(!r.text().contains("falsches-passwort"));

    // Provider down → 502.
    provider.reset().await;
    Mock::given(method("PROPFIND"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&provider)
        .await;
    let r = s
        .post("/api/accounts", Auth::Web(&session), fastmail_body())
        .await;
    assert_eq!(r.status, StatusCode::BAD_GATEWAY, "{}", r.text());
    assert_eq!(r.error_code(), "provider_unavailable");
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("nicht erreichbar"));

    // Throttled → 502 as well.
    provider.reset().await;
    Mock::given(method("PROPFIND"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "30"))
        .mount(&provider)
        .await;
    let r = s
        .post("/api/accounts", Auth::Web(&session), fastmail_body())
        .await;
    assert_eq!(r.status, StatusCode::BAD_GATEWAY);
    assert_eq!(r.error_code(), "provider_unavailable");

    // Nothing was stored, no change was recorded.
    let list = s.get("/api/accounts", Auth::Bearer(BOOTSTRAP)).await.json();
    assert_eq!(list, json!([]));
    assert_eq!(s.app.change_cursor(), 0);
}

#[tokio::test]
async fn account_input_is_validated() {
    let (s, _provider, session) = setup().await;
    let cases = [
        (
            json!({"kind": "ics", "url": "https://example.org/feed.ics", "name": "Feed"}),
            "LIC-14",
        ),
        (
            json!({"kind": "microsoft", "username": USER, "password": "x"}),
            "Microsoft",
        ),
        (
            json!({"kind": "exchange", "username": USER, "password": "x"}),
            "Unbekannte Kontoart",
        ),
        (
            json!({"kind": "fastmail", "password": PASSWORD}),
            "username",
        ),
        (json!({"kind": "fastmail", "username": USER}), "password"),
        (
            json!({"kind": "fastmail", "username": USER, "password": ""}),
            "password",
        ),
        (
            json!({"kind": "fastmail", "username": USER, "password": PASSWORD, "url": "https://dav.example.org/"}),
            "url",
        ),
        (
            json!({"kind": "caldav", "username": USER, "password": PASSWORD}),
            "url",
        ),
        (
            json!({"kind": "caldav", "username": USER, "password": PASSWORD, "url": "ftp://dav.example.org/"}),
            "url",
        ),
        (
            json!({"kind": "fastmail", "username": USER, "password": PASSWORD, "name": " "}),
            "Name",
        ),
        (
            json!({"kind": "fastmail", "username": USER, "password": PASSWORD, "farbe": "rot"}),
            "farbe",
        ),
    ];
    for (body, hint) in cases {
        let r = s
            .post("/api/accounts", Auth::Web(&session), body.clone())
            .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{body}: {}", r.text());
        assert_eq!(r.error_code(), "invalid_request");
        let message = r.json()["error"]["message"].as_str().unwrap().to_owned();
        assert!(message.contains(hint), "{body}: {message}");
    }
}

#[tokio::test]
async fn generic_caldav_account_uses_its_url() {
    let (s, provider, session) = setup().await;
    let r = s
        .post(
            "/api/accounts",
            Auth::Web(&session),
            json!({"kind": "caldav", "url": format!("{}/dav/", provider.uri()),
                   "username": USER, "password": PASSWORD}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let account = r.json();
    assert_eq!(account["kind"], "caldav");
    assert_eq!(account["url"], format!("{}/dav/", provider.uri()));
    assert_eq!(account["name"], "127.0.0.1");
    assert_eq!(calendars(&s).await.len(), 2);
}

#[tokio::test]
async fn refresh_adds_updates_and_removes_calendars() {
    let (s, provider, session) = setup().await;
    let account = create_account(&s, &session).await;
    let account_id = account["id"].as_str().unwrap().to_owned();
    let before = calendars(&s).await;
    let privat_id = by_name(&before, "Privat")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let team_id = by_name(&before, "Team-Sync")["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // User settings and a set with both calendars.
    let r = patch(
        &s,
        &format!("/api/calendars/{privat_id}"),
        Auth::Web(&session),
        json!({"hidden": true, "mcp_write": false}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let r = s
        .post(
            "/api/calendar-sets",
            Auth::Web(&session),
            json!({"name": "Alles", "calendar_ids": [team_id, privat_id]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let set_id = r.json()["id"].as_str().unwrap().to_owned();
    let set_seq = |s: &TestServer| -> i64 {
        db(s)
            .query_row(
                "SELECT seq FROM calendar_sets WHERE id = ?1",
                [&set_id],
                |r| r.get(0),
            )
            .unwrap()
    };
    let set_seq_before = set_seq(&s);

    // Provider: Privat renamed, recolored and read-only; Team-Sync gone;
    // a new calendar "Sport".
    let changed = vec![
        FakeCalendar {
            color: Some("#FF8800"),
            order: Some(1),
            writable: false,
            ..cal("Privat", "Zuhause")
        },
        FakeCalendar {
            order: Some(2),
            ..cal("Sport", "Sport")
        },
    ];
    mount_provider(&provider, PASSWORD, &changed).await;
    let cursor_before = s.app.change_cursor();
    let report = s.app.refresh_calendars(&account_id).await.unwrap();
    assert_eq!(
        report,
        lichtung_server::accounts::RefreshReport {
            added: 1,
            updated: 1,
            removed: 1
        }
    );
    assert!(s.app.change_cursor() > cursor_before);

    let after = calendars(&s).await;
    assert_eq!(after.len(), 2, "{after:?}");
    let privat = by_name(&after, "Zuhause");
    assert_eq!(privat["id"], privat_id.as_str(), "same calendar, same ID");
    assert_eq!(privat["provider_color"], "#FF8800");
    assert_eq!(privat["color"], "#FF8800");
    assert_eq!(privat["writable"], false);
    assert_eq!(privat["can_invite"], false);
    // User settings survive.
    assert_eq!(privat["hidden"], true);
    assert_eq!(privat["mcp_write"], false);
    let sport = by_name(&after, "Sport");
    assert_eq!(sport["writable"], true);
    assert_eq!(sport["mcp_write"], true);
    assert_eq!(sport["position"], 2, "appended after existing calendars");

    // The vanished calendar left a tombstone and its set changed.
    assert!(tombstones(&s).contains(&("calendar".into(), team_id.clone())));
    let sets = s
        .get("/api/calendar-sets", Auth::Bearer(BOOTSTRAP))
        .await
        .json();
    assert_eq!(sets[0]["calendar_ids"], json!([privat_id]));
    assert!(set_seq(&s) > set_seq_before);

    // A refresh without changes records only the account (last_sync_at).
    let cursor = s.app.change_cursor();
    let report = s.app.refresh_calendars(&account_id).await.unwrap();
    assert_eq!(report, Default::default());
    assert_eq!(s.app.change_cursor(), cursor + 1);
}

#[tokio::test]
async fn sync_endpoint_runs_in_the_background_and_reports_status() {
    let (s, provider, session) = setup().await;
    let account = create_account(&s, &session).await;
    let id = account["id"].as_str().unwrap().to_owned();
    let path = format!("/api/accounts/{id}/sync");

    // Any principal of /api may trigger a sync.
    let token = api_token(&s, "werkzeug").await;
    let r = s
        .request(Method::POST, &path, Auth::Bearer(&token), None)
        .await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text());
    assert_eq!(r.json()["status"]["state"], "syncing");
    wait_for_sync(&s, &id).await;
    let a = s.app.account(&id).await.unwrap();
    assert_eq!(a.status.state, "ok");

    // Credentials revoked at the provider → auth_failed.
    mount_provider(&provider, "neues-passwort", &default_calendars()).await;
    let r = s
        .request(Method::POST, &path, Auth::Bearer(BOOTSTRAP), None)
        .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    wait_for_sync(&s, &id).await;
    let a = s.app.account(&id).await.unwrap();
    assert_eq!(a.status.state, "auth_failed");
    assert!(a.status.message.unwrap().contains("Fastmail"));

    // Provider down → error, with the reason.
    provider.reset().await;
    Mock::given(method("PROPFIND"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&provider)
        .await;
    let err = s.app.refresh_calendars(&id).await.unwrap_err();
    assert_eq!(err.code(), "provider_unavailable");
    let a = s.app.account(&id).await.unwrap();
    assert_eq!(a.status.state, "error");
    assert!(a.status.message.unwrap().contains("nicht erreichbar"));

    let r = s
        .request(
            Method::POST,
            "/api/accounts/acc_unbekannt/sync",
            Auth::Bearer(BOOTSTRAP),
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

async fn wait_for_sync(s: &TestServer, id: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while s.app.sync_running(id) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "sync did not finish"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn new_credentials_are_checked_before_they_are_stored() {
    let (s, provider, session) = setup().await;
    let account = create_account(&s, &session).await;
    let id = account["id"].as_str().unwrap().to_owned();
    let path = format!("/api/accounts/{id}");

    // Rename only: no provider call needed.
    provider.reset().await;
    let r = patch(
        &s,
        &path,
        Auth::Web(&session),
        json!({"name": "Privat-Mail"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["name"], "Privat-Mail");

    // The provider now only accepts a new app password.
    mount_provider(&provider, "neues-app-passwort", &default_calendars()).await;
    let r = patch(
        &s,
        &path,
        Auth::Web(&session),
        json!({"password": "noch-ein-falsches"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    assert_eq!(r.error_code(), "invalid_credentials");
    // The old (now refused) password is still stored: a refresh fails.
    assert!(s.app.refresh_calendars(&id).await.is_err());

    let r = patch(
        &s,
        &path,
        Auth::Web(&session),
        json!({"password": "neues-app-passwort"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["status"]["state"], "ok");
    assert!(!r.text().contains("neues-app-passwort"));
    // Stored: the refresh works with the new password.
    s.app.refresh_calendars(&id).await.unwrap();

    // url belongs to caldav accounts only; unknown fields are refused.
    let r = patch(
        &s,
        &path,
        Auth::Web(&session),
        json!({"url": "https://dav.example.org/"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = patch(&s, &path, Auth::Web(&session), json!({"kind": "icloud"})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = patch(
        &s,
        "/api/accounts/acc_unbekannt",
        Auth::Web(&session),
        json!({"name": "x"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_an_account_removes_its_calendars_with_tombstones() {
    let (s, _provider, session) = setup().await;
    let account = create_account(&s, &session).await;
    let id = account["id"].as_str().unwrap().to_owned();
    let cals = calendars(&s).await;
    let cal_ids: Vec<String> = cals
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect();
    let r = s
        .post(
            "/api/calendar-sets",
            Auth::Web(&session),
            json!({"name": "Privat", "calendar_ids": [cal_ids[0]]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED);

    let cursor = s.app.change_cursor();
    let r = s
        .delete(&format!("/api/accounts/{id}"), Auth::Web(&session))
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.text());
    assert!(s.app.change_cursor() > cursor);
    assert!(calendars(&s).await.is_empty());
    let t = tombstones(&s);
    assert!(t.contains(&("account".into(), id.clone())), "{t:?}");
    for cal_id in &cal_ids {
        assert!(t.contains(&("calendar".into(), cal_id.clone())), "{t:?}");
    }
    let sets = s
        .get("/api/calendar-sets", Auth::Bearer(BOOTSTRAP))
        .await
        .json();
    assert_eq!(sets[0]["calendar_ids"], json!([]));

    let r = s
        .delete(&format!("/api/accounts/{id}"), Auth::Web(&session))
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn calendar_settings_are_validated() {
    let (s, _provider, session) = setup().await;
    create_account(&s, &session).await;
    let cals = calendars(&s).await;
    let id = by_name(&cals, "Privat")["id"].as_str().unwrap().to_owned();
    let path = format!("/api/calendars/{id}");

    let cursor = s.app.change_cursor();
    let r = patch(
        &s,
        &path,
        Auth::Web(&session),
        json!({"color": "#aabbcc", "openings_busy": false, "all_day_blocks": true, "position": 7}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let c = r.json();
    assert_eq!(c["color"], "#AABBCC");
    assert_eq!(c["provider_color"], "#3A7BD5");
    assert_eq!(c["openings_busy"], false);
    assert_eq!(c["all_day_blocks"], true);
    assert_eq!(c["position"], 7);
    assert_eq!(s.app.change_cursor(), cursor + 1);

    // The same values again change nothing.
    let r = patch(&s, &path, Auth::Web(&session), json!({"position": 7})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(s.app.change_cursor(), cursor + 1);

    // null returns to the provider color.
    let r = patch(&s, &path, Auth::Web(&session), json!({"color": null})).await;
    assert_eq!(r.json()["color"], "#3A7BD5");

    for body in [
        json!({"color": "#12"}),
        json!({"color": "blau"}),
        json!({"color": "#AABBCCDD"}),
        json!({"position": -1}),
        json!({"hidden": "ja"}),
        json!({"name": "Umbenannt"}),
    ] {
        let r = patch(&s, &path, Auth::Web(&session), body.clone()).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{body}: {}", r.text());
        assert_eq!(r.error_code(), "invalid_request");
    }
    let r = patch(
        &s,
        "/api/calendars/cal_unbekannt",
        Auth::Web(&session),
        json!({"hidden": true}),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn calendar_sets_can_be_managed() {
    let (s, _provider, session) = setup().await;
    create_account(&s, &session).await;
    let cals = calendars(&s).await;
    let a = cals[0]["id"].as_str().unwrap().to_owned();
    let b = cals[1]["id"].as_str().unwrap().to_owned();

    let r = s
        .post(
            "/api/calendar-sets",
            Auth::Web(&session),
            json!({"name": " Arbeit ", "calendar_ids": [b, a]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let set = r.json();
    let id = set["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("set_"));
    assert_eq!(
        set,
        json!({"id": id, "name": "Arbeit", "calendar_ids": [b, a], "position": 0})
    );
    let r = s
        .post(
            "/api/calendar-sets",
            Auth::Web(&session),
            json!({"name": "Privat"}),
        )
        .await;
    assert_eq!(r.json()["position"], 1);
    assert_eq!(r.json()["calendar_ids"], json!([]));

    let path = format!("/api/calendar-sets/{id}");
    let r = patch(
        &s,
        &path,
        Auth::Web(&session),
        json!({"calendar_ids": [a], "position": 5, "name": "Job"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(
        r.json(),
        json!({"id": id, "name": "Job", "calendar_ids": [a], "position": 5})
    );

    for body in [
        json!({"calendar_ids": [a, a]}),
        json!({"calendar_ids": ["cal_unbekannt"]}),
        json!({"name": ""}),
        json!({"position": -3}),
        json!({"farbe": "rot"}),
    ] {
        let r = patch(&s, &path, Auth::Web(&session), body.clone()).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{body}: {}", r.text());
    }
    let r = s
        .post(
            "/api/calendar-sets",
            Auth::Web(&session),
            json!({"calendar_ids": []}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    let list = s
        .get("/api/calendar-sets", Auth::Bearer(BOOTSTRAP))
        .await
        .json();
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["name"], "Privat", "ordered by position");

    let r = s.delete(&path, Auth::Web(&session)).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert!(tombstones(&s).contains(&("calendar_set".into(), id.clone())));
    let r = s.delete(&path, Auth::Web(&session)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tokens_may_read_but_not_manage_accounts_or_settings() {
    let (s, _provider, session) = setup().await;
    let account = create_account(&s, &session).await;
    let account_id = account["id"].as_str().unwrap().to_owned();
    let cal_id = calendars(&s).await[0]["id"].as_str().unwrap().to_owned();
    let token = api_token(&s, "werkzeug").await;

    for auth in [Auth::Bearer(BOOTSTRAP), Auth::Bearer(&token)] {
        for path in ["/api/accounts", "/api/calendars", "/api/calendar-sets"] {
            assert_eq!(s.get(path, auth).await.status, StatusCode::OK, "{path}");
        }
        let forbidden = [
            s.post("/api/accounts", auth, fastmail_body()).await,
            patch(
                &s,
                &format!("/api/accounts/{account_id}"),
                auth,
                json!({"name": "x"}),
            )
            .await,
            s.delete(&format!("/api/accounts/{account_id}"), auth).await,
            patch(
                &s,
                &format!("/api/calendars/{cal_id}"),
                auth,
                json!({"mcp_write": true}),
            )
            .await,
            s.post("/api/calendar-sets", auth, json!({"name": "x"}))
                .await,
        ];
        for r in forbidden {
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
            assert_eq!(r.error_code(), "forbidden");
        }
    }

    // Browser sessions need the CSRF header for changes.
    let r = patch(
        &s,
        &format!("/api/calendars/{cal_id}"),
        Auth::Cookie(&session),
        json!({"hidden": true}),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // Without login nothing.
    assert_eq!(
        s.get("/api/accounts", Auth::None).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_wrong_secret_key_stops_the_server_and_can_be_reset() {
    let (s, provider, session) = setup().await;
    let account = create_account(&s, &session).await;
    let id = account["id"].as_str().unwrap().to_owned();
    let mut config = s.app.config().clone();
    // Keep the data directory, close the server.
    let TestServer { app, router, dir } = s;
    drop((app, router));

    // Another key: the server refuses to start.
    config.secret_key = std::array::from_fn(|i| (i as u8).wrapping_mul(13).wrapping_add(5));
    let err = lichtung_server::build(config.clone())
        .await
        .err()
        .expect("start with a wrong key fails");
    assert!(
        err.to_string().contains("LICHTUNG_SECRET_KEY passt nicht"),
        "{err}"
    );

    // After a reset the server starts with the new key; the account stays
    // but has to be connected again.
    assert_eq!(
        lichtung_server::accounts::reset_secret_key(&config).unwrap(),
        1
    );
    let (app, router) = lichtung_server::build(config).await.unwrap();
    let a = app.account(&id).await.unwrap();
    assert_eq!(a.status.state, "auth_failed");
    let err = app.refresh_calendars(&id).await.unwrap_err();
    assert_eq!(err.code(), "invalid_credentials");
    assert_eq!(app.list_calendars().await.unwrap().len(), 2);

    // Reconnecting needs the password again.
    let s = TestServer { app, router, dir };
    let session = s.session().await;
    let path = format!("/api/accounts/{id}");
    let r = patch(&s, &path, Auth::Web(&session), json!({"name": "Neu"})).await;
    assert_eq!(r.json()["status"]["state"], "auth_failed");
    let r = patch(&s, &path, Auth::Web(&session), json!({"username": USER})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text());
    let r = patch(
        &s,
        &path,
        Auth::Web(&session),
        json!({"password": PASSWORD}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["status"]["state"], "ok");
    drop(provider);
}
