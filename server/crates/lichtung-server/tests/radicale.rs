//! Accounts, event sync and the write path with push and pull against a
//! real local CalDAV server (Radicale).
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

use common::{device_token, Auth, TestServer};

/// The tests share one calendar home, and a sync covers every calendar in
/// it; they run one after another so they do not see each other's changes.
static RADICALE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
#[ignore = "needs a local CalDAV server, see LICHTUNG_TEST_CALDAV_URL"]
async fn caldav_account_against_radicale() {
    let _serial = RADICALE.lock().await;
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
    let _serial = RADICALE.lock().await;
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

#[tokio::test]
#[ignore = "needs a local CalDAV server, see LICHTUNG_TEST_CALDAV_URL"]
async fn push_and_pull_against_radicale() {
    let _serial = RADICALE.lock().await;
    let Ok(url) = std::env::var("LICHTUNG_TEST_CALDAV_URL") else {
        eprintln!("LICHTUNG_TEST_CALDAV_URL nicht gesetzt, Test übersprungen");
        return;
    };
    let user = std::env::var("LICHTUNG_TEST_CALDAV_USER").unwrap_or_else(|_| "lichtung".into());
    let password = std::env::var("LICHTUNG_TEST_CALDAV_PASSWORD").unwrap_or_default();

    let client = Client::new(url.clone(), Credentials::basic(&user, &password)).unwrap();
    let discovery = client.discover().await.expect("discovery");
    let home = discovery.calendar_home().expect("calendar home").clone();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("Schreib-Test {nanos}");
    let cal = client
        .make_calendar(&format!("{home}lichtung-schreiben-{nanos}/"), &name)
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
    s.app.sync_account(&account_id).await.expect("sync");
    let device = device_token(&s, "iPad");
    let push = |mutations: serde_json::Value| {
        let s = &s;
        let device = device.clone();
        async move {
            let r = s
                .post(
                    "/api/sync/push",
                    Auth::Bearer(&device),
                    json!({"mutations": mutations}),
                )
                .await;
            assert_eq!(r.status, StatusCode::OK, "{}", r.text());
            r.json()["results"].as_array().unwrap().clone()
        }
    };
    let zoned = |local: &str| json!({"date_time": local, "time_zone": "Europe/Berlin"});
    let uid = format!("chor-{nanos}@example.org");

    // Create a weekly series.
    let r = push(json!([{
        "id": format!("c-{nanos}"), "op": "create_event", "calendar_id": cal_id,
        "event": {"uid": uid, "title": "Chorprobe", "location": "Gemeindehaus",
                  "start": zoned("2026-11-02T19:00:00"), "end": zoned("2026-11-02T21:00:00"),
                  "recurrence": {"rrule": "FREQ=WEEKLY;COUNT=6"},
                  "alarms": [{"minutes_before": 30}]}
    }]))
    .await;
    assert_eq!(r[0]["status"], "applied", "{r:?}");
    let ev = r[0]["events"][0].clone();
    let id = ev["id"].as_str().unwrap().to_owned();
    let href = client.new_resource_href(cal.as_str(), &uid).unwrap();
    let at_server = client.get(href.as_str()).await.expect("GET");
    assert!(at_server.data.contains("SUMMARY:Chorprobe"));

    // Change all, then one occurrence, then split.
    let r = push(json!([
        {"id": format!("u1-{nanos}"), "op": "update_event", "event_id": id, "base_version": 1,
         "changes": {"title": "Chorprobe Sopran"}},
        {"id": format!("u2-{nanos}"), "op": "update_event", "event_id": id, "base_version": 2,
         "scope": "this", "occurrence": zoned("2026-11-09T19:00:00"),
         "changes": {"location": "Kirche"}},
        {"id": format!("u3-{nanos}"), "op": "update_event", "event_id": id, "base_version": 3,
         "scope": "this_and_following", "occurrence": zoned("2026-11-23T19:00:00"),
         "changes": {"start": zoned("2026-11-23T18:30:00"), "end": zoned("2026-11-23T20:30:00")}}
    ]))
    .await;
    let statuses: Vec<&str> = r.iter().map(|x| x["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["applied", "applied", "applied"], "{r:?}");
    let split = r[2]["events"].as_array().unwrap();
    assert_eq!(split.len(), 2);
    assert_eq!(split[1]["start"]["date_time"], "2026-11-23T18:30:00");
    let new_id = split[1]["id"].as_str().unwrap().to_owned();
    let at_server = client.get(href.as_str()).await.expect("GET");
    assert!(at_server.data.contains("LOCATION:Kirche"));

    // Delete one occurrence of the new series.
    let r = push(
        json!([{"id": format!("d1-{nanos}"), "op": "delete_event", "event_id": new_id,
                         "base_version": 1, "scope": "this",
                         "occurrence": zoned("2026-11-30T18:30:00")}]),
    )
    .await;
    assert_eq!(r[0]["status"], "applied", "{r:?}");

    // Stored as the server has it: a sync changes nothing in this calendar
    // (other calendars of the home may belong to tests running alongside).
    let before = s.app.calendar_events(&cal_id).await.unwrap();
    s.app.sync_account(&account_id).await.expect("sync");
    assert_eq!(s.app.calendar_events(&cal_id).await.unwrap(), before);
    let page = s
        .get("/api/sync/pull?since=0", Auth::Bearer(&device))
        .await
        .json();
    let events: Vec<&serde_json::Value> = page["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "event" && c["data"]["calendar_id"] == cal_id.as_str())
        .map(|c| &c["data"])
        .collect();
    assert_eq!(events.len(), 2, "{page}");
    let old = events.iter().find(|e| e["id"] == id.as_str()).unwrap();
    assert_eq!(old["title"], "Chorprobe Sopran");
    assert_eq!(old["overrides"][0]["location"], "Kirche");
    let new = events.iter().find(|e| e["id"] == new_id.as_str()).unwrap();
    assert_eq!(
        new["recurrence"]["exdates"][0]["date_time"],
        "2026-11-30T18:30:00"
    );
    let cursor = page["cursor"].as_i64().unwrap();

    // Conflict: someone changes the object directly at the server.
    let current = client.get(href.as_str()).await.expect("GET");
    let changed = current.data.replace(
        "SUMMARY:Chorprobe Sopran",
        "SUMMARY:Chorprobe (vom Kalender)",
    );
    client
        .put(
            href.as_str(),
            &changed,
            lichtung_caldav::Precondition::IfMatch(current.etag.clone().unwrap()),
        )
        .await
        .expect("direct PUT");
    let version = old["version"].as_i64().unwrap();
    let r = push(
        json!([{"id": format!("k1-{nanos}"), "op": "update_event", "event_id": id,
                         "base_version": version, "changes": {"title": "Chorprobe Alt"}}]),
    )
    .await;
    assert_eq!(r[0]["status"], "conflict", "{r:?}");
    assert_eq!(r[0]["events"][0]["title"], "Chorprobe (vom Kalender)");
    assert_eq!(r[0]["conflict"]["origin"], "app:iPad");
    // An outdated base_version is a conflict as well.
    let r = push(
        json!([{"id": format!("k2-{nanos}"), "op": "delete_event", "event_id": id,
                         "base_version": version}]),
    )
    .await;
    assert_eq!(r[0]["status"], "conflict", "{r:?}");
    assert!(client.get(href.as_str()).await.is_ok(), "not deleted");

    // Delete both series; the pull reports it.
    let current = r[0]["events"][0]["version"].as_i64().unwrap();
    let r = push(json!([
        {"id": format!("d2-{nanos}"), "op": "delete_event", "event_id": id, "base_version": current},
        {"id": format!("d3-{nanos}"), "op": "delete_event", "event_id": new_id, "base_version": 2}
    ]))
    .await;
    let statuses: Vec<&str> = r.iter().map(|x| x["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["applied", "applied"], "{r:?}");
    assert!(matches!(
        client.get(href.as_str()).await,
        Err(lichtung_caldav::Error::NotFound)
    ));
    let page = s
        .get(
            &format!("/api/sync/pull?since={cursor}"),
            Auth::Bearer(&device),
        )
        .await
        .json();
    let changes = page["changes"].as_array().unwrap();
    for gone in [&id, &new_id] {
        assert!(
            changes
                .iter()
                .any(|c| c["type"] == "event" && c["op"] == "delete" && c["id"] == gone.as_str()),
            "{page}"
        );
    }
    assert_eq!(
        changes.iter().filter(|c| c["type"] == "conflict").count(),
        2,
        "{page}"
    );

    client.delete(cal.as_str(), None).await.expect("DELETE");
    let r = s
        .delete(&format!("/api/accounts/{account_id}"), Auth::Web(&session))
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
}
