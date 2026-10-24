//! Contract and integration tests of the MCP endpoint (`/mcp`, LIC-12):
//! protocol, auth, the frozen tool schemas and every tool via HTTP against
//! the invented CalDAV server. All data is made up.

mod common;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use lichtung_server::app::now_ms;
use lichtung_server::db::{self, OAuthClientRow, OAuthTokenKind, OAuthTokenRow};
use lichtung_server::{auth, oauth};
use serde_json::{json, Value};

use common::dav::{
    all_day, invitation, series, setup_with, single, vcal, Setup, CALENDAR, HOLIDAYS, USER, WORK,
};
use common::{api_token, device_token, Auth, TestResponse, TestServer, BOOTSTRAP};

const SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/snapshots/mcp_tools.json"
);
const BERLIN: &str = "Europe/Berlin";

fn rpc(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

async fn mcp_post(s: &TestServer, token: &str, body: Value) -> TestResponse {
    s.request(Method::POST, "/mcp", Auth::Bearer(token), Some(body))
        .await
}

/// The `tools/call` result.
async fn call(s: &TestServer, token: &str, name: &str, args: Value) -> Value {
    let r = mcp_post(
        s,
        token,
        rpc(1, "tools/call", json!({"name": name, "arguments": args})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let body = r.json();
    assert!(body.get("error").is_none(), "{body}");
    body["result"].clone()
}

/// A successful tool call's structured content.
async fn ok(s: &TestServer, token: &str, name: &str, args: Value) -> Value {
    let result = call(s, token, name, args).await;
    assert_eq!(result["isError"], false, "{name}: {result}");
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["structuredContent"]);
    result["structuredContent"].clone()
}

/// A failed tool call's structured content (`error.code`, `error.message`, details).
async fn err(s: &TestServer, token: &str, name: &str, args: Value) -> Value {
    let result = call(s, token, name, args).await;
    assert_eq!(result["isError"], true, "{name}: {result}");
    assert!(result["content"][0]["text"].is_string());
    result["structuredContent"].clone()
}

fn unfold(ics: &str) -> String {
    ics.replace("\r\n ", "")
}

/// An OAuth access token for `/mcp`, issued directly in the database for
/// an invented client.
fn oauth_token(s: &TestServer, client_name: &str) -> String {
    let conn = rusqlite::Connection::open(s.app.config().db_path()).unwrap();
    let now = now_ms();
    let client_id = format!("client-{}", uuid::Uuid::now_v7().simple());
    db::insert_oauth_client(
        &conn,
        &OAuthClientRow {
            id: client_id.clone(),
            name: client_name.to_owned(),
            redirect_uris: vec!["https://claude.example.org/callback".into()],
            created_at: now,
            last_used_at: None,
            revoked_at: None,
        },
    )
    .unwrap();
    let raw = format!("loa_test-{}", uuid::Uuid::now_v7().simple());
    let resource = oauth::normalize_resource(&oauth::mcp_resource(s.app.config())).unwrap();
    db::insert_oauth_token(
        &conn,
        &auth::sha256_hex(&raw),
        &OAuthTokenRow {
            id: uuid::Uuid::now_v7().to_string(),
            kind: OAuthTokenKind::Access,
            client_id,
            user_id: s.app.owner_id(),
            family_id: format!("familie-{client_name}"),
            resource,
            scope: None,
            created_at: now,
            expires_at: now + 3_600_000,
            used_at: None,
            revoked_at: None,
        },
    )
    .unwrap();
    raw
}

impl Setup {
    async fn settings(&self, calendar_id: &str, patch: Value) {
        self.s
            .app
            .update_calendar(calendar_id, serde_json::from_value(patch).unwrap())
            .await
            .unwrap();
    }

    fn writes(&self) -> usize {
        self.dav.with(|s| s.writes.len())
    }
}

/// A synced fake account with a token for Claude.
async fn synced(prepare: impl FnOnce(&common::dav::FakeDav)) -> (Setup, String) {
    let t = setup_with(prepare).await;
    t.sync().await;
    let token = api_token(&t.s, "claude").await;
    (t, token)
}

// ----- protocol -------------------------------------------------------------------------

#[tokio::test]
async fn initialize_negotiates_version() {
    let s = TestServer::new().await;
    let t = api_token(&s, "claude").await;
    for (requested, expected) in [
        ("2025-06-18", "2025-06-18"),
        ("2025-03-26", "2025-03-26"),
        ("2024-11-05", "2025-06-18"),
    ] {
        let r = mcp_post(
            &s,
            &t,
            rpc(
                1,
                "initialize",
                json!({
                    "protocolVersion": requested,
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1"},
                }),
            ),
        )
        .await;
        assert_eq!(r.status, StatusCode::OK);
        assert!(r
            .header("content-type")
            .unwrap()
            .starts_with("application/json"));
        assert!(r.header("mcp-session-id").is_none());
        let body = r.json();
        assert_eq!(body["jsonrpc"], "2.0");
        assert_eq!(body["id"], 1);
        let result = &body["result"];
        assert_eq!(result["protocolVersion"], expected);
        assert_eq!(result["serverInfo"]["name"], "lichtung");
        assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(result["capabilities"]["tools"]["listChanged"], false);
        let instructions = result["instructions"].as_str().unwrap();
        assert!(instructions.contains("calendar_id"));
        assert!(instructions.contains("mcp_writable"));
    }
}

#[tokio::test]
async fn notifications_ping_and_batches() {
    let s = TestServer::new().await;
    let t = api_token(&s, "claude").await;
    let r = mcp_post(
        &s,
        &t,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    assert!(r.body.is_empty());

    let r = mcp_post(&s, &t, rpc(7, "ping", json!({}))).await;
    assert_eq!(r.json(), json!({"jsonrpc": "2.0", "id": 7, "result": {}}));

    // A client response alone is accepted without answer.
    let r = mcp_post(&s, &t, json!({"jsonrpc": "2.0", "id": 3, "result": {}})).await;
    assert_eq!(r.status, StatusCode::ACCEPTED);

    let r = mcp_post(
        &s,
        &t,
        json!([
            rpc(1, "ping", json!({})),
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": "zwei", "method": "tools/list"},
            rpc(3, "tools/call", json!({"name": "create_event", "arguments": {}})),
        ]),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.json();
    let replies = body.as_array().unwrap();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[1]["id"], "zwei");
    assert!(replies[1]["result"]["tools"].is_array());
    // A failing tool does not break the batch.
    assert_eq!(replies[2]["result"]["isError"], true);

    let r = mcp_post(
        &s,
        &t,
        json!([{"jsonrpc": "2.0", "method": "notifications/initialized"}]),
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);

    let r = mcp_post(&s, &t, json!([])).await;
    assert_eq!(r.json()["error"]["code"], -32600);

    // Sessions are optional: a client-sent session ID is ignored.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::AUTHORIZATION, format!("Bearer {t}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("mcp-session-id", "irgendwas")
        .body(Body::from(rpc(1, "ping", json!({})).to_string()))
        .unwrap();
    assert_eq!(s.send(req).await.status, StatusCode::OK);
}

#[tokio::test]
async fn json_rpc_errors() {
    let s = TestServer::new().await;
    let t = api_token(&s, "claude").await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::AUTHORIZATION, format!("Bearer {t}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{kaputt"))
        .unwrap();
    let r = s.send(req).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"]["code"], -32700);
    assert_eq!(r.json()["id"], Value::Null);

    let cases = [
        (rpc(1, "resources/list", json!({})), -32601),
        (json!({"jsonrpc": "1.0", "id": 1, "method": "ping"}), -32600),
        (json!({"jsonrpc": "2.0", "id": 1}), -32600),
        (
            json!({"jsonrpc": "2.0", "id": null, "method": "ping"}),
            -32600,
        ),
        (
            json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": [1]}),
            -32602,
        ),
        (
            rpc(1, "tools/call", json!({"name": "create_opening_link"})),
            -32602,
        ),
        (rpc(1, "tools/call", json!({"arguments": {}})), -32602),
        (
            rpc(
                1,
                "tools/call",
                json!({"name": "query_events", "arguments": [1]}),
            ),
            -32602,
        ),
        (json!(42), -32600),
    ];
    for (body, code) in cases {
        let r = mcp_post(&s, &t, body.clone()).await;
        assert_eq!(r.status, StatusCode::OK, "{body}");
        assert_eq!(r.json()["error"]["code"], code, "{body}");
        assert!(r.json()["error"]["message"].is_string());
    }
}

#[tokio::test]
async fn auth_origin_and_methods() {
    let s = TestServer::new().await;
    let ping = rpc(1, "ping", json!({}));
    let send = |auth: Auth<'_>, origin: Option<&str>| {
        let mut b = Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header(header::CONTENT_TYPE, "application/json");
        b = common::authorize(b, auth);
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        b.body(Body::from(ping.to_string())).unwrap()
    };
    let metadata = format!(
        "Bearer resource_metadata=\"{}\"",
        oauth::resource_metadata_url(s.app.config())
    );

    // Without a token: 401 with the protected resource metadata.
    let r = s.send(send(Auth::None, None)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.header("www-authenticate").unwrap(), metadata);
    let r = s.send(send(Auth::Bearer("lt_falsch"), None)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let challenge = r.header("www-authenticate").unwrap();
    assert!(challenge.starts_with(&metadata), "{challenge}");
    assert!(challenge.contains("error=\"invalid_token\""), "{challenge}");

    // Device tokens of the app are only for /api.
    let device = device_token(&s, "iPhone von Anna");
    assert_eq!(common::api_status(&s, &device).await, StatusCode::OK);
    let r = s.send(send(Auth::Bearer(&device), None)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r
        .header("www-authenticate")
        .unwrap()
        .contains("invalid_token"));

    // Browser sessions do not work for /mcp.
    let session = s.session().await;
    let r = s.send(send(Auth::Web(&session), None)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // API token, bootstrap token and OAuth access token work.
    let api = api_token(&s, "claude-code").await;
    let access = oauth_token(&s, "Claude");
    for token in [api.as_str(), BOOTSTRAP, access.as_str()] {
        let r = s.send(send(Auth::Bearer(token), None)).await;
        assert_eq!(r.status, StatusCode::OK, "{token}");
    }
    // An OAuth token for /mcp does not open /api.
    assert_eq!(
        common::api_status(&s, &access).await,
        StatusCode::UNAUTHORIZED
    );

    // Origin must match the public URL.
    let r = s
        .send(send(Auth::Bearer(&api), Some("http://localhost:8080")))
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = s
        .send(send(Auth::Bearer(&api), Some("https://boese.example")))
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // Unsupported protocol version header.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::AUTHORIZATION, format!("Bearer {api}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("mcp-protocol-version", "1999-01-01")
        .body(Body::from(ping.to_string()))
        .unwrap();
    assert_eq!(s.send(req).await.status, StatusCode::BAD_REQUEST);

    // No server-initiated stream, no sessions to delete.
    for method in [Method::GET, Method::DELETE] {
        let r = s
            .request(method.clone(), "/mcp", Auth::Bearer(&api), None)
            .await;
        assert_eq!(r.status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
    }

    // Revoked tokens stop working.
    let created = s
        .app
        .create_token(s.app.owner_id(), "kurzlebig")
        .await
        .unwrap();
    assert_eq!(
        mcp_post(&s, &created.token, ping.clone()).await.status,
        StatusCode::OK
    );
    s.app
        .revoke_token(s.app.owner_id(), created.info.id)
        .await
        .unwrap();
    assert_eq!(
        mcp_post(&s, &created.token, ping.clone()).await.status,
        StatusCode::UNAUTHORIZED
    );
}

// ----- contract -------------------------------------------------------------------------

#[tokio::test]
async fn tool_schemas_match_snapshot() {
    let s = TestServer::new().await;
    let t = api_token(&s, "claude").await;
    let r = mcp_post(&s, &t, rpc(1, "tools/list", json!({}))).await;
    let tools = r.json()["result"]["tools"].clone();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_calendars",
            "list_calendar_sets",
            "query_events",
            "find_free_times",
            "list_invitations",
            "create_event",
            "update_event",
            "delete_event",
            "respond_to_invitation",
        ]
    );
    for tool in tools.as_array().unwrap() {
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object", "{}", tool["name"]);
        assert_eq!(schema["additionalProperties"], false, "{}", tool["name"]);
        for key in ["oneOf", "anyOf", "allOf"] {
            assert!(
                schema.get(key).is_none(),
                "{} uses top-level {key}",
                tool["name"]
            );
        }
        assert!(tool["description"].as_str().unwrap().len() > 20);
    }
    // No default calendar: create_event requires calendar_id.
    let create = &tools[5];
    assert!(create["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("calendar_id")));

    let actual = format!("{}\n", serde_json::to_string_pretty(&tools).unwrap());
    if std::env::var("UPDATE_SNAPSHOTS").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(std::path::Path::new(SNAPSHOT).parent().unwrap()).unwrap();
        std::fs::write(SNAPSHOT, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(SNAPSHOT).unwrap_or_default();
    assert!(
        expected == actual,
        "Die Werkzeug-Schemas weichen vom Vertrag ab ({SNAPSHOT}). \
         Absichtliche Änderung? docs/mcp.md anpassen und mit UPDATE_SNAPSHOTS=1 neu schreiben."
    );
}

// ----- calendars ------------------------------------------------------------------------

#[tokio::test]
async fn list_calendars_and_sets() {
    let (t, token) = synced(|dav| {
        dav.add_calendar(WORK, "Arbeit", true);
        dav.add_calendar(HOLIDAYS, "Feiertage", false);
    })
    .await;
    let work = t.calendar_named("Arbeit").await;
    let holidays = t.calendar_named("Feiertage").await;
    t.settings(&work, json!({"mcp_write": false, "hidden": true}))
        .await;
    let set =
        t.s.app
            .create_calendar_set(
                serde_json::from_value(json!({"name": "Privat & Feiertage",
                                          "calendar_ids": [t.calendar_id, holidays]}))
                .unwrap(),
            )
            .await
            .unwrap();

    let r = ok(&t.s, &token, "list_calendars", json!({})).await;
    let cals = r["calendars"].as_array().unwrap();
    assert_eq!(cals.len(), 3);
    let by_name = |n: &str| cals.iter().find(|c| c["name"] == n).unwrap().clone();
    let privat = by_name("Privat");
    assert_eq!(privat["id"], t.calendar_id);
    assert_eq!(privat["account"]["kind"], "caldav");
    assert_eq!(privat["account"]["name"], "Beispiel");
    assert_eq!(privat["writable"], true);
    assert_eq!(privat["mcp_writable"], true);
    assert_eq!(privat["hidden"], false);
    assert_eq!(privat["openings_busy"], true);
    assert!(privat["color"].as_str().unwrap().starts_with('#'));
    let arbeit = by_name("Arbeit");
    assert_eq!(
        (
            &arbeit["writable"],
            &arbeit["mcp_writable"],
            &arbeit["hidden"]
        ),
        (&json!(true), &json!(false), &json!(true))
    );
    let feiertage = by_name("Feiertage");
    assert_eq!(
        (&feiertage["writable"], &feiertage["mcp_writable"]),
        (&json!(false), &json!(false))
    );

    let r = ok(&t.s, &token, "list_calendar_sets", json!({})).await;
    assert_eq!(
        r["calendar_sets"],
        json!([{"id": set.id, "name": "Privat & Feiertage",
                "calendar_ids": [t.calendar_id, holidays]}])
    );
}

// ----- reading --------------------------------------------------------------------------

fn new_york_call(uid: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\n\
         DTSTART;TZID=America/New_York:20261104T090000\r\n\
         DTEND;TZID=America/New_York:20261104T100000\r\n\
         SUMMARY:Call mit New York\r\nDESCRIPTION:Quartalszahlen besprechen\r\nEND:VEVENT\r\n"
    ))
}

fn titles(r: &Value) -> Vec<String> {
    r["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn query_events_expands_series_and_searches() {
    let (t, token) = synced(|dav| {
        dav.add_calendar(WORK, "Arbeit", true);
        dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"));
        dav.put("team.ics", &series("team-sync@example.org"));
        dav.put("wandern.ics", &all_day("wandern@example.org"));
        dav.put("ny.ics", &new_york_call("ny@example.org"));
        dav.put_in(
            WORK,
            "projekt.ics",
            &single("projekt@example.org", "Projekttermin"),
        );
    })
    .await;
    let work = t.calendar_named("Arbeit").await;
    t.settings(&work, json!({"hidden": true})).await;

    let r = ok(
        &t.s,
        &token,
        "query_events",
        json!({"start": "2026-11-01", "end": "2026-11-30", "time_zone": BERLIN}),
    )
    .await;
    assert_eq!(r["time_zone"], BERLIN);
    assert_eq!(r["truncated"], false);
    assert_eq!(
        titles(&r),
        [
            "Team-Sync",
            "Zahnarzt",
            "Call mit New York",
            "Wanderwochenende",
            "Team-Sync (verschoben)",
            "Team-Sync",
        ],
        "hidden calendar left out, 16 Nov excluded"
    );
    let events = r["events"].as_array().unwrap();
    let first = &events[0];
    assert_eq!(first["start"], "2026-11-02T09:00:00+01:00");
    assert_eq!(first["end"], "2026-11-02T09:30:00+01:00");
    assert_eq!(first["duration_minutes"], 30);
    assert_eq!(first["recurrence_id"], "2026-11-02T09:00:00+01:00");
    assert_eq!(first["is_recurring"], true);
    assert_eq!(first["event_time_zone"], BERLIN);
    assert_eq!(first["all_day"], false);
    assert_eq!(first["status"], "confirmed");
    assert_eq!(first["availability"], "busy");
    assert_eq!(first["calendar"]["id"], t.calendar_id);
    assert_eq!(first["calendar"]["name"], "Privat");
    assert_eq!(first["calendar"]["mcp_writable"], true);

    let dentist = &events[1];
    assert_eq!(dentist["recurrence_id"], Value::Null);
    assert_eq!(dentist["is_recurring"], false);
    assert_eq!(dentist["location"], "Praxis am Markt");
    assert_eq!(dentist["attendees"], json!([]));
    assert_eq!(dentist["self_status"], Value::Null);

    let ny = &events[2];
    assert_eq!(ny["start"], "2026-11-04T15:00:00+01:00");
    assert_eq!(ny["event_time_zone"], "America/New_York");
    assert_eq!(ny["notes"], "Quartalszahlen besprechen");

    let hike = &events[3];
    assert_eq!(hike["all_day"], true);
    assert_eq!(hike["start"], "2026-11-05");
    assert_eq!(hike["end"], "2026-11-06", "last day, inclusive");
    assert_eq!(hike["duration_minutes"], 2 * 24 * 60);
    assert_eq!(hike["availability"], "free");

    let moved = &events[4];
    assert_eq!(moved["recurrence_id"], "2026-11-09T09:00:00+01:00");
    assert_eq!(moved["start"], "2026-11-09T10:00:00+01:00");
    assert_eq!(moved["event_id"], first["event_id"]);

    // Output in another zone.
    let r = ok(
        &t.s,
        &token,
        "query_events",
        json!({"start": "2026-11-03T00:00:00+01:00", "end": "2026-11-04T00:00:00+01:00",
               "time_zone": "America/New_York"}),
    )
    .await;
    assert_eq!(titles(&r), ["Zahnarzt"]);
    assert_eq!(r["events"][0]["start"], "2026-11-03T08:00:00-05:00");
    assert_eq!(r["events"][0]["event_time_zone"], BERLIN);

    // Search in title, location and notes; all words must match.
    for (query, expected) in [
        ("praxis", vec!["Zahnarzt"]),
        ("TEAM verschoben", vec!["Team-Sync (verschoben)"]),
        ("quartalszahlen", vec!["Call mit New York"]),
        ("gibt es nicht", vec![]),
    ] {
        let r = ok(
            &t.s,
            &token,
            "query_events",
            json!({"start": "2026-11-01", "end": "2026-11-30", "query": query}),
        )
        .await;
        assert_eq!(titles(&r), expected, "{query}");
    }

    // Explicit calendars include hidden ones; a set selects its calendars.
    let r = ok(
        &t.s,
        &token,
        "query_events",
        json!({"start": "2026-11-01", "end": "2026-11-30", "calendar_ids": [work]}),
    )
    .await;
    assert_eq!(titles(&r), ["Projekttermin"]);
    let set =
        t.s.app
            .create_calendar_set(
                serde_json::from_value(json!({"name": "Arbeit", "calendar_ids": [work]})).unwrap(),
            )
            .await
            .unwrap();
    let r = ok(
        &t.s,
        &token,
        "query_events",
        json!({"start": "2026-11-01", "end": "2026-11-30", "calendar_set_id": set.id}),
    )
    .await;
    assert_eq!(titles(&r), ["Projekttermin"]);

    // Limits.
    let r = ok(
        &t.s,
        &token,
        "query_events",
        json!({"start": "2026-11-01", "end": "2026-11-30", "limit": 2}),
    )
    .await;
    assert_eq!(r["count"], 2);
    assert_eq!(r["truncated"], true);
    let e = err(
        &t.s,
        &token,
        "query_events",
        json!({"start": "2026-01-01", "end": "2027-12-31"}),
    )
    .await;
    assert_eq!(e["error"]["code"], "invalid_arguments");
    assert!(e["error"]["message"].as_str().unwrap().contains("400 Tage"));
    let e = err(
        &t.s,
        &token,
        "query_events",
        json!({"start": "2026-11-01", "end": "2026-11-30", "calendar_ids": ["cal_gibtsnicht"]}),
    )
    .await;
    assert_eq!(e["error"]["code"], "calendar_not_found");
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("list_calendars"));
}

/// Events on Tuesday, 10 Nov 2026 (Berlin) for the free-time rules.
fn busy_day(dav: &common::dav::FakeDav) {
    let event = |uid: &str, start: &str, end: &str, extra: &str| {
        vcal(&format!(
            "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\n\
             DTSTART;TZID=Europe/Berlin:{start}\r\nDTEND;TZID=Europe/Berlin:{end}\r\n\
             SUMMARY:{uid}\r\n{extra}END:VEVENT\r\n"
        ))
    };
    dav.put(
        "arzt.ics",
        &event("arzt", "20261110T100000", "20261110T110000", ""),
    );
    dav.put(
        "abgesagt.ics",
        &event(
            "abgesagt",
            "20261110T120000",
            "20261110T130000",
            "STATUS:CANCELLED\r\n",
        ),
    );
    dav.put(
        "frei.ics",
        &event(
            "frei",
            "20261110T140000",
            "20261110T150000",
            "TRANSP:TRANSPARENT\r\n",
        ),
    );
    dav.put(
        "abgelehnt.ics",
        &event(
            "abgelehnt",
            "20261110T153000",
            "20261110T160000",
            &format!(
                "ORGANIZER:mailto:bert@example.org\r\n\
                 ATTENDEE;PARTSTAT=DECLINED:mailto:{USER}\r\n"
            ),
        ),
    );
    dav.put(
        "ganztags.ics",
        &vcal(
            "BEGIN:VEVENT\r\nUID:ganztags\r\nDTSTAMP:20261020T070000Z\r\n\
             DTSTART;VALUE=DATE:20261110\r\nDTEND;VALUE=DATE:20261111\r\n\
             SUMMARY:Messe\r\nEND:VEVENT\r\n",
        ),
    );
}

#[tokio::test]
async fn find_free_times_follows_the_opening_rules() {
    let (t, token) = synced(busy_day).await;
    let args = json!({
        "start": "2026-11-10", "end": "2026-11-10", "min_duration_minutes": 30,
        "buffer_minutes": 15, "day_start": "09:00", "day_end": "17:00", "time_zone": BERLIN,
    });
    let r = ok(&t.s, &token, "find_free_times", args.clone()).await;
    // Busy only 10:00–11:00 (+15 min buffer): cancelled, free, declined
    // and all-day events (all_day_blocks off) do not block.
    assert_eq!(
        r["free"],
        json!([
            {"start": "2026-11-10T09:00:00+01:00", "end": "2026-11-10T09:45:00+01:00",
             "duration_minutes": 45},
            {"start": "2026-11-10T11:15:00+01:00", "end": "2026-11-10T17:00:00+01:00",
             "duration_minutes": 345},
        ])
    );
    assert_eq!(r["calendar_ids"], json!([t.calendar_id]));

    // A longer minimum drops the short slot.
    let mut longer = args.clone();
    longer["min_duration_minutes"] = json!(60);
    let r = ok(&t.s, &token, "find_free_times", longer).await;
    assert_eq!(r["count"], 1);

    // Weekdays: Tuesday is not among them.
    let mut weekend = args.clone();
    weekend["weekdays"] = json!(["sa", "su"]);
    let r = ok(&t.s, &token, "find_free_times", weekend).await;
    assert_eq!(r["free"], json!([]));

    // All-day events block where the calendar says so.
    t.settings(&t.calendar_id, json!({"all_day_blocks": true}))
        .await;
    let r = ok(&t.s, &token, "find_free_times", args.clone()).await;
    assert_eq!(r["free"], json!([]));

    // Calendars without openings_busy do not count by default.
    t.settings(&t.calendar_id, json!({"openings_busy": false}))
        .await;
    let r = ok(&t.s, &token, "find_free_times", args.clone()).await;
    assert_eq!(r["calendar_ids"], json!([]));
    assert_eq!(r["free"][0]["duration_minutes"], 8 * 60);

    // Bad windows are explained.
    let mut bad = args.clone();
    bad["day_end"] = json!("08:00");
    let e = err(&t.s, &token, "find_free_times", bad).await;
    assert!(e["error"]["message"].as_str().unwrap().contains("day_end"));
    let mut bad = args;
    bad.as_object_mut().unwrap().remove("day_end");
    let e = err(&t.s, &token, "find_free_times", bad).await;
    assert_eq!(e["error"]["code"], "invalid_arguments");
}

// ----- writing --------------------------------------------------------------------------

#[tokio::test]
async fn create_update_and_delete_single_events() {
    let (t, token) = synced(|_| {}).await;
    let r = ok(
        &t.s,
        &token,
        "create_event",
        json!({
            "calendar_id": t.calendar_id, "title": "Lauftreff",
            "start": "2026-11-12T18:00:00+01:00", "end": "2026-11-12T19:00:00+01:00",
            "time_zone": BERLIN, "location": "Stadtpark", "notes": "Stirnlampe mitnehmen",
            "alarms": [30], "availability": "busy",
            "attendees": [{"email": "carla@example.org", "name": "Carla Beispiel"}],
        }),
    )
    .await;
    assert_eq!(r["status"], "applied");
    let ev = &r["events"][0];
    let id = ev["event_id"].as_str().unwrap().to_owned();
    assert_eq!(ev["title"], "Lauftreff");
    assert_eq!(ev["start"], "2026-11-12T18:00:00+01:00");
    assert_eq!(ev["duration_minutes"], 60);
    assert_eq!(ev["event_time_zone"], BERLIN);
    assert_eq!(ev["calendar"]["id"], t.calendar_id);
    assert_eq!(ev["alarms"], json!([{"minutes_before": 30}]));
    assert_eq!(ev["attendees"][0]["email"], "carla@example.org");
    let uid = t.s.app.event(&id).await.unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_owned();
    let (_, _, data) = t.dav.find_uid(CALENDAR, &uid).unwrap();
    let data = unfold(&data);
    assert!(
        data.contains("DTSTART;TZID=Europe/Berlin:20261112T180000"),
        "{data}"
    );
    assert!(
        data.contains("DTEND;TZID=Europe/Berlin:20261112T190000"),
        "{data}"
    );
    assert!(data.contains("LOCATION:Stadtpark"), "{data}");
    assert!(data.contains("mailto:carla@example.org"), "{data}");
    assert!(data.contains("BEGIN:VALARM"), "{data}");

    // All-day: end is the last day (inclusive).
    let r = ok(
        &t.s,
        &token,
        "create_event",
        json!({"calendar_id": t.calendar_id, "title": "Brückentag",
               "start": "2026-11-20", "end": "2026-11-20"}),
    )
    .await;
    let day = &r["events"][0];
    assert_eq!(
        (&day["start"], &day["end"], &day["all_day"]),
        (&json!("2026-11-20"), &json!("2026-11-20"), &json!(true))
    );
    assert_eq!(day["duration_minutes"], 24 * 60);
    let day_uid =
        t.s.app
            .event(day["event_id"].as_str().unwrap())
            .await
            .unwrap()["uid"]
            .as_str()
            .unwrap()
            .to_owned();
    let (_, _, data) = t.dav.find_uid(CALENDAR, &day_uid).unwrap();
    assert!(data.contains("DTSTART;VALUE=DATE:20261120"), "{data}");
    assert!(data.contains("DTEND;VALUE=DATE:20261121"), "{data}");

    // Only start: the length stays, the zone of the event stays.
    let r = ok(
        &t.s,
        &token,
        "update_event",
        json!({"event_id": id, "start": "2026-11-12T18:30:00+01:00", "location": null}),
    )
    .await;
    assert_eq!(r["events"][0]["duration_minutes"], 60);
    assert_eq!(r["events"][0]["location"], Value::Null);
    let (_, _, data) = t.dav.find_uid(CALENDAR, &uid).unwrap();
    let data = unfold(&data);
    assert!(
        data.contains("DTSTART;TZID=Europe/Berlin:20261112T183000"),
        "{data}"
    );
    assert!(
        data.contains("DTEND;TZID=Europe/Berlin:20261112T193000"),
        "{data}"
    );
    assert!(!data.contains("\r\nLOCATION"), "{data}");

    // Nothing to change.
    let e = err(&t.s, &token, "update_event", json!({"event_id": id})).await;
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Keine Änderung"));

    // Delete (a scope on a single event means the whole event).
    let r = ok(&t.s, &token, "delete_event", json!({"event_id": id})).await;
    assert_eq!(r["deleted_event_ids"], json!([id]));
    assert!(t.dav.find_uid(CALENDAR, &uid).is_none());
    let e = err(&t.s, &token, "delete_event", json!({"event_id": id})).await;
    assert_eq!(e["error"]["code"], "conflict", "deleted before: {e}");
    let e = err(
        &t.s,
        &token,
        "update_event",
        json!({"event_id": "ev_gibtsnicht", "title": "x"}),
    )
    .await;
    assert_eq!(e["error"]["code"], "event_not_found");
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("query_events"));
}

async fn occurrence_titles(t: &Setup, token: &str) -> Vec<(String, String)> {
    let r = ok(
        &t.s,
        token,
        "query_events",
        json!({"start": "2026-11-01", "end": "2026-11-30", "time_zone": BERLIN}),
    )
    .await;
    r["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["start"].as_str().unwrap().to_owned(),
                e["title"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn series_need_a_scope() {
    let (t, token) = synced(|dav| dav.put("team.ics", &series("team-sync@example.org"))).await;
    let id = t.event_by_uid("team-sync@example.org").await["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let e = err(
        &t.s,
        &token,
        "update_event",
        json!({"event_id": id, "title": "Neu"}),
    )
    .await;
    assert_eq!(e["error"]["code"], "scope_required");
    assert!(e["error"]["message"].as_str().unwrap().contains("Serie"));
    let e = err(
        &t.s,
        &token,
        "delete_event",
        json!({"event_id": id, "scope": "this"}),
    )
    .await;
    assert_eq!(e["error"]["code"], "occurrence_required");

    // Only this occurrence.
    ok(
        &t.s,
        &token,
        "update_event",
        json!({"event_id": id, "scope": "this", "occurrence": "2026-11-23T09:00:00+01:00",
               "title": "Team-Sync (Workshop)", "start": "2026-11-23T13:00:00+01:00"}),
    )
    .await;
    assert_eq!(
        occurrence_titles(&t, &token).await,
        [
            ("2026-11-02T09:00:00+01:00".into(), "Team-Sync".into()),
            (
                "2026-11-09T10:00:00+01:00".into(),
                "Team-Sync (verschoben)".into()
            ),
            (
                "2026-11-23T13:00:00+01:00".into(),
                "Team-Sync (Workshop)".into()
            ),
        ]
    );
    let (_, _, data) = t.dav.find_uid(CALENDAR, "team-sync@example.org").unwrap();
    let data = unfold(&data);
    assert!(
        data.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261123T090000"),
        "{data}"
    );
    assert!(
        data.contains("DTEND;TZID=Europe/Berlin:20261123T133000"),
        "{data}"
    );

    // Delete one occurrence, then this and the following ones.
    ok(
        &t.s,
        &token,
        "delete_event",
        json!({"event_id": id, "scope": "this", "occurrence": "2026-11-02T09:00:00+01:00"}),
    )
    .await;
    let after: Vec<String> = occurrence_titles(&t, &token)
        .await
        .into_iter()
        .map(|(s, _)| s)
        .collect();
    assert_eq!(
        after,
        ["2026-11-09T10:00:00+01:00", "2026-11-23T13:00:00+01:00"]
    );
    ok(
        &t.s,
        &token,
        "delete_event",
        json!({"event_id": id, "scope": "this_and_following",
               "occurrence": "2026-11-23T09:00:00+01:00"}),
    )
    .await;
    let after: Vec<String> = occurrence_titles(&t, &token)
        .await
        .into_iter()
        .map(|(s, _)| s)
        .collect();
    assert_eq!(after, ["2026-11-09T10:00:00+01:00"]);

    // The whole series.
    let r = ok(
        &t.s,
        &token,
        "update_event",
        json!({"event_id": id, "scope": "all", "notes": "Agenda im Dokument"}),
    )
    .await;
    assert_eq!(r["events"][0]["notes"], "Agenda im Dokument");
    let r = ok(
        &t.s,
        &token,
        "delete_event",
        json!({"event_id": id, "scope": "all"}),
    )
    .await;
    assert_eq!(r["deleted_event_ids"], json!([id]));
    assert!(occurrence_titles(&t, &token).await.is_empty());
}

#[tokio::test]
async fn invitations_are_listed_and_answered() {
    let (t, token) = synced(|dav| dav.put("sitzung.ics", &invitation("sitzung@example.org"))).await;
    let range = json!({"start": "2026-11-01", "end": "2026-11-30", "time_zone": BERLIN});
    let r = ok(&t.s, &token, "list_invitations", range.clone()).await;
    assert_eq!(r["count"], 1, "{r}");
    let inv = &r["invitations"][0];
    assert_eq!(inv["title"], "Vereinssitzung");
    assert_eq!(inv["self_status"], "needs_action");
    assert_eq!(inv["pending_occurrences"], 3);
    assert_eq!(inv["occurrence"], Value::Null);
    assert_eq!(inv["can_respond"], true);
    assert_eq!(inv["start"], "2026-11-04T10:00:00+01:00");
    assert_eq!(inv["organizer"]["email"], "bert@example.org");
    let id = inv["event_id"].as_str().unwrap().to_owned();

    // One occurrence: maybe.
    ok(
        &t.s,
        &token,
        "respond_to_invitation",
        json!({"event_id": id, "occurrence": "2026-11-11T10:00:00+01:00",
               "response": "tentative"}),
    )
    .await;
    let (_, _, data) = t.dav.find_uid(CALENDAR, "sitzung@example.org").unwrap();
    let data = unfold(&data);
    assert!(
        data.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261111T100000"),
        "{data}"
    );
    assert!(data.contains("PARTSTAT=TENTATIVE"), "{data}");
    let r = ok(&t.s, &token, "list_invitations", range.clone()).await;
    assert_eq!(r["invitations"][0]["pending_occurrences"], 2, "{r}");

    // The whole series: yes.
    let r = ok(
        &t.s,
        &token,
        "respond_to_invitation",
        json!({"event_id": id, "response": "accepted", "comment": "Ich bringe Kuchen mit."}),
    )
    .await;
    assert_eq!(r["events"][0]["self_status"], "accepted");
    let (_, _, data) = t.dav.find_uid(CALENDAR, "sitzung@example.org").unwrap();
    let data = unfold(&data);
    // Answering the whole series answers every occurrence.
    assert!(
        data.contains("PARTSTAT=ACCEPTED:mailto:anna@example.org"),
        "{data}"
    );
    assert!(!data.contains("PARTSTAT=TENTATIVE"), "{data}");
    assert!(data.contains("COMMENT:Ich bringe Kuchen mit."), "{data}");
    assert!(data.contains("RECURRENCE-ID"), "{data}");
    let r = ok(&t.s, &token, "list_invitations", range).await;
    assert_eq!(r["count"], 0, "{r}");

    let e = err(
        &t.s,
        &token,
        "respond_to_invitation",
        json!({"event_id": id, "response": "vielleicht"}),
    )
    .await;
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("accepted, tentative, declined"));
}

// ----- protection -----------------------------------------------------------------------

#[tokio::test]
async fn mcp_write_false_locks_every_write_tool() {
    let (t, token) = synced(|dav| {
        dav.add_calendar(HOLIDAYS, "Feiertage", false);
        dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"));
        dav.put("team.ics", &series("team-sync@example.org"));
        dav.put("sitzung.ics", &invitation("sitzung@example.org"));
    })
    .await;
    let holidays = t.calendar_named("Feiertage").await;
    t.settings(&t.calendar_id, json!({"mcp_write": false}))
        .await;
    let single_id = t.event_by_uid("zahnarzt@example.org").await["id"].clone();
    let series_id = t.event_by_uid("team-sync@example.org").await["id"].clone();
    let invitation_id = t.event_by_uid("sitzung@example.org").await["id"].clone();
    let writes = t.writes();

    let calls = [
        (
            "create_event",
            json!({"calendar_id": t.calendar_id, "title": "Von Claude",
                   "start": "2026-11-12T18:00:00+01:00", "end": "2026-11-12T19:00:00+01:00"}),
        ),
        (
            "update_event",
            json!({"event_id": single_id, "title": "Von Claude"}),
        ),
        (
            "update_event",
            json!({"event_id": series_id, "scope": "this",
                   "occurrence": "2026-11-02T09:00:00+01:00", "title": "Von Claude"}),
        ),
        ("delete_event", json!({"event_id": single_id})),
        (
            "delete_event",
            json!({"event_id": series_id, "scope": "all"}),
        ),
        (
            "respond_to_invitation",
            json!({"event_id": invitation_id, "response": "accepted"}),
        ),
        (
            "respond_to_invitation",
            json!({"event_id": invitation_id, "response": "declined",
                   "occurrence": "2026-11-11T10:00:00+01:00"}),
        ),
    ];
    for (tool, args) in calls {
        let e = err(&t.s, &token, tool, args.clone()).await;
        assert_eq!(
            e["error"]["code"], "mcp_write_forbidden",
            "{tool} {args}: {e}"
        );
        let message = e["error"]["message"].as_str().unwrap();
        assert!(message.contains("„Privat“"), "{message}");
        assert!(message.contains("in der App"), "{message}");
    }
    assert_eq!(t.writes(), writes, "nothing reached the provider");

    let r = ok(
        &t.s,
        &token,
        "list_invitations",
        json!({"start": "2026-11-01", "end": "2026-11-30"}),
    )
    .await;
    assert_eq!(r["invitations"][0]["can_respond"], false);

    // Read-only at the provider stays calendar_read_only.
    let e = err(
        &t.s,
        &token,
        "create_event",
        json!({"calendar_id": holidays, "title": "Brückentag",
               "start": "2026-12-28", "end": "2026-12-28"}),
    )
    .await;
    assert_eq!(e["error"]["code"], "calendar_read_only");

    // The app is not affected by the MCP guard.
    let outcome =
        t.s.app
            .apply_mutation(
                &lichtung_server::writes::Origin::Web,
                &json!({"op": "update_event", "event_id": single_id, "base_version": 1,
                    "changes": {"title": "Aus der App"}}),
            )
            .await
            .unwrap()
            .to_json("m");
    assert_eq!(outcome["status"], "applied", "{outcome}");
}

#[tokio::test]
async fn arguments_are_explained() {
    let (t, token) = synced(|_| {}).await;
    let base = json!({"title": "Lauftreff", "start": "2026-11-12T18:00:00+01:00",
                      "end": "2026-11-12T19:00:00+01:00"});

    // No default calendar.
    let e = err(&t.s, &token, "create_event", base.clone()).await;
    assert_eq!(e["error"]["code"], "calendar_id_missing");
    let message = e["error"]["message"].as_str().unwrap();
    assert!(message.contains("list_calendars"), "{message}");
    assert!(message.contains("Standardkalender"), "{message}");
    let mut null_calendar = base.clone();
    null_calendar["calendar_id"] = Value::Null;
    let e = err(&t.s, &token, "create_event", null_calendar).await;
    assert_eq!(e["error"]["code"], "calendar_id_missing");

    let with_calendar = |patch: Value| {
        let mut v = base.clone();
        v["calendar_id"] = json!(t.calendar_id);
        for (k, val) in patch.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    };
    // Times need an offset.
    let e = err(
        &t.s,
        &token,
        "create_event",
        with_calendar(json!({"start": "2026-11-12T18:00:00"})),
    )
    .await;
    assert_eq!(e["error"]["code"], "invalid_arguments");
    let message = e["error"]["message"].as_str().unwrap();
    assert!(message.contains("keine Zeitzone"), "{message}");
    assert!(message.contains("+01:00"), "{message}");
    // Mixed forms, end before start, unknown zone, unknown calendar and
    // parameter, bad rule.
    for (patch, code, needle) in [
        (
            json!({"end": "2026-11-12"}),
            "invalid_arguments",
            "dieselbe Form",
        ),
        (
            json!({"end": "2026-11-12T17:00:00+01:00"}),
            "invalid_arguments",
            "vor start",
        ),
        (
            json!({"time_zone": "Mars/Olympus"}),
            "invalid_arguments",
            "IANA",
        ),
        (
            json!({"calendar_id": "cal_gibtsnicht"}),
            "calendar_not_found",
            "list_calendars",
        ),
        (
            json!({"farbe": "rot"}),
            "invalid_arguments",
            "Unbekannter Parameter",
        ),
        (
            json!({"recurrence": "JEDEN MONTAG"}),
            "invalid_event",
            "rrule",
        ),
        (
            json!({"availability": "vielleicht"}),
            "invalid_arguments",
            "busy, free",
        ),
    ] {
        let e = err(&t.s, &token, "create_event", with_calendar(patch.clone())).await;
        assert_eq!(e["error"]["code"], code, "{patch}: {e}");
        let message = e["error"]["message"].as_str().unwrap();
        assert!(message.contains(needle), "{patch}: {message}");
    }
    assert_eq!(t.dav.put_count(), 0);
}

#[tokio::test]
async fn conflicts_show_the_current_state() {
    let (t, _) =
        synced(|dav| dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"))).await;
    let token = oauth_token(&t.s, "Claude");
    let id = t.event_by_uid("zahnarzt@example.org").await["id"].clone();
    // Changed at the provider after the last sync.
    t.dav.put(
        "zahnarzt.ics",
        &single("zahnarzt@example.org", "Zahnarzt (verlegt)"),
    );
    let e = err(
        &t.s,
        &token,
        "update_event",
        json!({"event_id": id, "title": "Zahnarzt mit Claude"}),
    )
    .await;
    assert_eq!(e["error"]["code"], "conflict", "{e}");
    let message = e["error"]["message"].as_str().unwrap();
    assert!(message.contains("Anbieter gewinnt"), "{message}");
    assert_eq!(e["current"][0]["title"], "Zahnarzt (verlegt)");
    assert_eq!(e["current"][0]["event_id"], id);
    let conflict_id = e["conflict_id"].as_str().unwrap().to_owned();

    // Visible to the app, with the MCP origin.
    let conflicts =
        t.s.get("/api/conflicts", Auth::Bearer(BOOTSTRAP))
            .await
            .json();
    let c = conflicts
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == conflict_id.as_str())
        .unwrap()
        .clone();
    assert_eq!(c["origin"], "mcp:oauth:Claude");
    assert_eq!(c["event_id"], id);
    assert_eq!(c["mutation"]["changes"]["title"], "Zahnarzt mit Claude");
}
