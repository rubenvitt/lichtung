//! Accounts against a real local CalDAV server (Radicale).
//!
//! Ignored by default. Run with (see `lichtung-caldav/README.md` for
//! starting Radicale):
//!
//! ```sh
//! LICHTUNG_TEST_CALDAV_URL=http://127.0.0.1:5232/ \
//!   cargo test -p lichtung-server --test radicale -- --ignored
//! ```
//!
//! `LICHTUNG_TEST_CALDAV_USER` / `LICHTUNG_TEST_CALDAV_PASSWORD` default to
//! `lichtung` / empty (fine for Radicale with `[auth] type = none`).

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use lichtung_caldav::{Client, Credentials};
use serde_json::json;

use common::{Auth, TestServer};

#[tokio::test]
#[ignore = "needs a local CalDAV server, see LICHTUNG_TEST_CALDAV_URL"]
async fn caldav_account_against_radicale() {
    let Ok(url) = std::env::var("LICHTUNG_TEST_CALDAV_URL") else {
        eprintln!("LICHTUNG_TEST_CALDAV_URL nicht gesetzt, Test übersprungen");
        return;
    };
    let user = std::env::var("LICHTUNG_TEST_CALDAV_USER").unwrap_or_else(|_| "lichtung".into());
    let password = std::env::var("LICHTUNG_TEST_CALDAV_PASSWORD").unwrap_or_default();

    // A fresh calendar on the server.
    let client = Client::new(url.clone(), Credentials::basic(&user, &password)).unwrap();
    let discovery = client.discover().await.expect("discovery");
    let home = discovery.calendar_home().expect("calendar home").clone();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let cal = client
        .make_calendar(&format!("{home}lichtung-konto-{nanos}/"), "Wanderverein")
        .await
        .expect("MKCALENDAR");

    let s = TestServer::new().await;
    let session = s.session().await;
    let r = s
        .post(
            "/api/accounts",
            Auth::Web(&session),
            json!({"kind": "caldav", "url": url, "username": user, "password": password,
                   "name": "Radicale"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let account = r.json();
    assert_eq!(account["status"]["state"], "ok");
    let account_id = account["id"].as_str().unwrap().to_owned();

    let cals = s.get("/api/calendars", Auth::Web(&session)).await.json();
    let ours = cals
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Wanderverein")
        .unwrap_or_else(|| panic!("calendar missing: {cals}"));
    assert_eq!(ours["writable"], true);
    assert_eq!(ours["mcp_write"], true);
    let cal_id = ours["id"].as_str().unwrap().to_owned();

    // Gone on the server → gone after a refresh, with a tombstone.
    client.delete(cal.as_str(), None).await.expect("DELETE");
    let report = s.app.refresh_calendars(&account_id).await.unwrap();
    assert_eq!(report.removed, 1, "{report:?}");
    let db = rusqlite::Connection::open(s.app.config().db_path()).unwrap();
    let tombstones = lichtung_server::changes::tombstones_since(&db, 0).unwrap();
    assert!(tombstones.iter().any(|t| t.id == cal_id));

    let r = s
        .delete(&format!("/api/accounts/{account_id}"), Auth::Web(&session))
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
}
