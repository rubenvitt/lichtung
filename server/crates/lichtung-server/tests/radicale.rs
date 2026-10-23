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

fn vcal(body: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//Test//DE\r\n{body}END:VCALENDAR\r\n"
    )
}

fn single(uid: &str, title: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\n\
         DTSTART;TZID=Europe/Berlin:20261103T140000\r\nDTEND;TZID=Europe/Berlin:20261103T150000\r\n\
         SUMMARY:{title}\r\nEND:VEVENT\r\n"
    ))
}

fn series(uid: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\n\
         DTSTART;TZID=Europe/Berlin:20261102T090000\r\nDTEND;TZID=Europe/Berlin:20261102T093000\r\n\
         RRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=4\r\nSUMMARY:Team-Sync\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\n\
         RECURRENCE-ID;TZID=Europe/Berlin:20261109T090000\r\n\
         DTSTART;TZID=Europe/Berlin:20261109T100000\r\nDTEND;TZID=Europe/Berlin:20261109T103000\r\n\
         SUMMARY:Team-Sync (verschoben)\r\nEND:VEVENT\r\n"
    ))
}

fn all_day(uid: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\nDTSTART;VALUE=DATE:20261105\r\n\
         DTEND;VALUE=DATE:20261107\r\nSUMMARY:Wanderwochenende\r\nEND:VEVENT\r\n"
    ))
}

#[tokio::test]
#[ignore = "needs a local CalDAV server, see LICHTUNG_TEST_CALDAV_URL"]
async fn event_sync_against_radicale() {
    use lichtung_caldav::Precondition;

    let Ok(url) = std::env::var("LICHTUNG_TEST_CALDAV_URL") else {
        eprintln!("LICHTUNG_TEST_CALDAV_URL nicht gesetzt, Test übersprungen");
        return;
    };
    let user = std::env::var("LICHTUNG_TEST_CALDAV_USER").unwrap_or_else(|_| "lichtung".into());
    let password = std::env::var("LICHTUNG_TEST_CALDAV_PASSWORD").unwrap_or_default();

    // A fresh calendar with three events, written directly over CalDAV.
    let client = Client::new(url.clone(), Credentials::basic(&user, &password)).unwrap();
    let discovery = client.discover().await.expect("discovery");
    let home = discovery.calendar_home().expect("calendar home").clone();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("Sync-Test {nanos}");
    let cal = client
        .make_calendar(&format!("{home}lichtung-sync-{nanos}/"), &name)
        .await
        .expect("MKCALENDAR");
    let uid = |n: &str| format!("{n}-{nanos}@example.org");
    let mut hrefs = Vec::new();
    for (n, ics) in [
        ("zahnarzt", single(&uid("zahnarzt"), "Zahnarzt")),
        ("team", series(&uid("team"))),
        ("urlaub", all_day(&uid("urlaub"))),
    ] {
        let href = client.new_resource_href(cal.as_str(), &uid(n)).unwrap();
        let put = client
            .put(href.as_str(), &ics, Precondition::Create)
            .await
            .expect("PUT");
        hrefs.push((href, put.etag));
    }

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
    let account_id = r.json()["id"].as_str().unwrap().to_owned();
    let cals = s.get("/api/calendars", Auth::Web(&session)).await.json();
    let cal_id = cals
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name.as_str())
        .unwrap_or_else(|| panic!("calendar missing: {cals}"))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let events = |s: &TestServer| {
        let app = s.app.clone();
        let cal_id = cal_id.clone();
        async move { app.calendar_events(&cal_id).await.unwrap() }
    };
    let by_title = |events: &[serde_json::Value], title: &str| {
        events
            .iter()
            .find(|e| e["title"] == title)
            .unwrap_or_else(|| panic!("no {title}: {events:?}"))
            .clone()
    };

    // First sync.
    s.app.sync_account(&account_id).await.expect("sync 1");
    let first = events(&s).await;
    assert_eq!(first.len(), 3, "{first:?}");
    assert!(first.iter().all(|e| e["version"] == 1));
    let team = by_title(&first, "Team-Sync");
    assert_eq!(team["recurrence"]["rrule"], "FREQ=WEEKLY;BYDAY=MO;COUNT=4");
    assert_eq!(team["overrides"][0]["title"], "Team-Sync (verschoben)");
    assert_eq!(team["overrides"][0]["start"]["utc"], "2026-11-09T09:00:00Z");
    let urlaub = by_title(&first, "Wanderwochenende");
    assert_eq!(urlaub["all_day"], true);
    assert_eq!(urlaub["end"], json!({"date": "2026-11-07"}));
    let zahnarzt = by_title(&first, "Zahnarzt");
    assert_eq!(zahnarzt["start"]["time_zone"], "Europe/Berlin");
    assert_eq!(zahnarzt["start"]["utc"], "2026-11-03T13:00:00Z");

    // Change one, delete one, sync again.
    let (href0, etag0) = &hrefs[0];
    client
        .put(
            href0.as_str(),
            &single(&uid("zahnarzt"), "Zahnarzt (verlegt)"),
            Precondition::IfMatch(etag0.clone().expect("etag")),
        )
        .await
        .expect("PUT ändern");
    client
        .delete(hrefs[2].0.as_str(), None)
        .await
        .expect("DELETE");
    let report = s.app.sync_account(&account_id).await.expect("sync 2");
    assert!(!report.events.full, "incremental: {report:?}");
    let second = events(&s).await;
    assert_eq!(second.len(), 2, "{second:?}");
    let z = by_title(&second, "Zahnarzt (verlegt)");
    assert_eq!(z["id"], zahnarzt["id"]);
    assert_eq!(z["version"], 2);
    assert_eq!(by_title(&second, "Team-Sync")["version"], 1);
    let db = rusqlite::Connection::open(s.app.config().db_path()).unwrap();
    let tombstones = lichtung_server::changes::tombstones_since(&db, 0).unwrap();
    assert!(tombstones
        .iter()
        .any(|t| t.kind == "event" && t.id == urlaub["id"].as_str().unwrap()));

    // Nothing new: nothing changes.
    s.app.sync_account(&account_id).await.expect("sync 3");
    assert_eq!(events(&s).await, second);

    // A token the server no longer knows: full comparison, no doubles.
    db.execute(
        "UPDATE calendars SET sync_token = 'http://radicale.org/ns/sync/0000' WHERE id = ?1",
        [&cal_id],
    )
    .unwrap();
    let report = s.app.sync_account(&account_id).await.expect("sync 4");
    assert!(report.events.full);
    assert_eq!(events(&s).await, second);

    // Occurrences of the calendar.
    let r = s
        .get(
            &format!(
                "/api/occurrences?from=2026-11-01&to=2026-12-01&calendar_ids={cal_id}&time_zone=Europe/Berlin"
            ),
            Auth::Web(&session),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let titles: Vec<String> = r.json()["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        titles,
        [
            "Team-Sync",
            "Zahnarzt (verlegt)",
            "Team-Sync (verschoben)",
            "Team-Sync",
            "Team-Sync"
        ]
    );

    // Calendar gone at the server: its events go with tombstones.
    client.delete(cal.as_str(), None).await.expect("DELETE");
    let report = s.app.sync_account(&account_id).await.expect("sync 5");
    assert_eq!(report.calendars.removed, 1);
    assert!(s.app.calendar_events(&cal_id).await.unwrap().is_empty());
    let tombstones = lichtung_server::changes::tombstones_since(&db, 0).unwrap();
    for e in &second {
        assert!(tombstones.iter().any(|t| t.id == e["id"].as_str().unwrap()));
    }

    let r = s
        .delete(&format!("/api/accounts/{account_id}"), Auth::Web(&session))
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
}
