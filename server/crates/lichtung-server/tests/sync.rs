//! Event sync against an invented, stateful CalDAV server (wiremock with a
//! custom responder): first and incremental sync, invalid sync tokens, the
//! ETag fallback, multiget with 404, an aborted round, unreadable data, 401,
//! vanished calendars, and `GET /api/occurrences`. All data is made up.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use common::dav::{all_day, series, setup, setup_with, single};
use common::{Auth, BOOTSTRAP};

// ----- tests ---------------------------------------------------------------------------

#[tokio::test]
async fn first_and_incremental_sync() {
    let t = setup().await;
    let cursor = t.s.app.subscribe_changes();

    let r = t.sync().await;
    assert_eq!(
        (r.events.added, r.events.updated, r.events.deleted),
        (2, 0, 0)
    );
    assert!(r.events.full, "no token yet: full comparison");
    assert!(t.sync_token().is_some(), "token stored after the round");
    assert_eq!(t.uids(), ["team-sync@example.org", "zahnarzt@example.org"]);
    assert!(*cursor.borrow() > 0);
    let (state, _) = t.status().await;
    assert_eq!(state, "ok");
    let account = t.s.app.account(&t.account_id).await.unwrap();
    assert!(account.status.last_sync_at.is_some());

    let zahnarzt = t.event_by_uid("zahnarzt@example.org").await;
    assert_eq!(zahnarzt["version"], 1);
    assert_eq!(zahnarzt["title"], "Zahnarzt");
    assert_eq!(
        zahnarzt["start"],
        json!({"date_time": "2026-11-03T14:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-03T13:00:00Z"})
    );
    let team = t.event_by_uid("team-sync@example.org").await;
    assert_eq!(team["recurrence"]["rrule"], "FREQ=WEEKLY;BYDAY=MO;COUNT=4");
    assert_eq!(team["overrides"].as_array().unwrap().len(), 1);
    assert_eq!(team["overrides"][0]["title"], "Team-Sync (verschoben)");

    // Nothing changed: no fetch, no new versions.
    let before = t.dav.multiget_count();
    let r = t.sync().await;
    assert_eq!(r.events, Default::default());
    assert_eq!(t.dav.multiget_count(), before);

    // Changed, new and deleted at the provider.
    t.dav.put(
        "zahnarzt.ics",
        &single("zahnarzt@example.org", "Zahnarzt (neu)"),
    );
    t.dav.put("urlaub.ics", &all_day("urlaub@example.org"));
    t.dav.remove("team.ics");
    // Reported again with the same ETag: no fetch, no new version.
    t.dav.touch("urlaub.ics");
    let r = t.sync().await;
    assert!(!r.events.full, "incremental with sync-collection");
    assert_eq!(
        (r.events.added, r.events.updated, r.events.deleted),
        (1, 1, 1)
    );
    assert_eq!(t.uids(), ["urlaub@example.org", "zahnarzt@example.org"]);
    let z = t.event_by_uid("zahnarzt@example.org").await;
    assert_eq!(z["version"], 2);
    assert_eq!(z["id"], zahnarzt["id"], "same event, new version");
    assert_eq!(z["title"], "Zahnarzt (neu)");
    assert_eq!(t.event_tombstones(), [team["id"].as_str().unwrap()]);

    t.dav.touch("urlaub.ics");
    let before = t.dav.multiget_count();
    let r = t.sync().await;
    assert_eq!(r.events.unchanged, 1);
    assert_eq!(
        t.dav.multiget_count(),
        before,
        "unchanged ETag is not fetched"
    );
    assert_eq!(t.event_by_uid("urlaub@example.org").await["version"], 1);
}

#[tokio::test]
async fn invalid_token_falls_back_to_full_comparison_without_loss_or_doubles() {
    let t = setup().await;
    t.sync().await;
    let zahnarzt = t.event_by_uid("zahnarzt@example.org").await;
    let team = t.event_by_uid("team-sync@example.org").await;

    t.dav.with(|s| s.tokens_invalid = true);
    t.dav.put(
        "zahnarzt.ics",
        &single("zahnarzt@example.org", "Zahnarzt (verlegt)"),
    );
    t.dav.put("urlaub.ics", &all_day("urlaub@example.org"));
    t.dav.remove("team.ics");
    let r = t.sync().await;
    assert!(r.events.full);
    assert_eq!(
        (r.events.added, r.events.updated, r.events.deleted),
        (1, 1, 1)
    );
    assert_eq!(t.uids(), ["urlaub@example.org", "zahnarzt@example.org"]);
    let z = t.event_by_uid("zahnarzt@example.org").await;
    assert_eq!(
        (z["id"].clone(), z["version"].clone()),
        (zahnarzt["id"].clone(), json!(2))
    );
    assert_eq!(t.event_tombstones(), [team["id"].as_str().unwrap()]);

    // The new token works again.
    t.dav.with(|s| s.tokens_invalid = false);
    let r = t.sync().await;
    assert!(!r.events.full);
    assert_eq!(r.events, Default::default());
}

#[tokio::test]
async fn etag_fallback_without_sync_collection() {
    let t = setup_with(|dav| {
        dav.with(|s| s.sync_unsupported = true);
        dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"));
    })
    .await;
    let r = t.sync().await;
    assert!(r.events.full);
    assert_eq!(r.events.added, 1);
    assert_eq!(t.sync_token(), None);

    // Unchanged ctag: no listing at all.
    let listings = t.dav.with(|s| s.etag_listings);
    let r = t.sync().await;
    assert_eq!(r.events.added + r.events.updated + r.events.deleted, 0);
    assert_eq!(t.dav.with(|s| s.etag_listings), listings);
    assert_eq!(t.dav.with(|s| s.sync_reports), 0);

    t.dav.put("urlaub.ics", &all_day("urlaub@example.org"));
    t.dav.remove("zahnarzt.ics");
    let r = t.sync().await;
    assert_eq!((r.events.added, r.events.deleted), (1, 1));
    assert_eq!(t.uids(), ["urlaub@example.org"]);
}

#[tokio::test]
async fn refused_sync_collection_is_remembered() {
    let t = setup().await;
    t.sync().await;
    t.dav.with(|s| s.sync_unsupported = true);
    t.dav.put("urlaub.ics", &all_day("urlaub@example.org"));
    let r = t.sync().await;
    assert!(r.events.full);
    assert_eq!(r.events.added, 1);
    let unsupported: bool = t
        .db()
        .query_row(
            "SELECT sync_unsupported FROM calendars WHERE id = ?1",
            [&t.calendar_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(unsupported);
    let reports = t.dav.with(|s| s.sync_reports);
    t.sync().await;
    assert_eq!(t.dav.with(|s| s.sync_reports), reports, "not asked again");
    assert_eq!(t.uids().len(), 3);
}

#[tokio::test]
async fn missing_in_multiget_is_deleted() {
    let t = setup().await;
    t.sync().await;
    let team = t.event_by_uid("team-sync@example.org").await;
    // Reported as changed, but gone when fetched.
    t.dav.put("team.ics", &series("team-sync@example.org"));
    t.dav.with(|s| {
        s.multiget_missing.insert("team.ics".into());
    });
    let r = t.sync().await;
    assert_eq!(r.events.deleted, 1);
    assert_eq!(t.uids(), ["zahnarzt@example.org"]);
    assert_eq!(t.event_tombstones(), [team["id"].as_str().unwrap()]);
}

#[tokio::test]
async fn aborted_round_keeps_the_token_and_the_next_run_repeats_it() {
    let t = setup().await;
    t.sync().await;
    let token = t.sync_token();
    let cursor = t.s.app.change_cursor();

    t.dav.put(
        "zahnarzt.ics",
        &single("zahnarzt@example.org", "Zahnarzt (verlegt)"),
    );
    t.dav.put("urlaub.ics", &all_day("urlaub@example.org"));
    t.dav.with(|s| s.multiget_status = Some(503));
    let err =
        t.s.app
            .sync_account(&t.account_id)
            .await
            .expect_err("multiget fails");
    assert_eq!(err.code(), "provider_unavailable");
    assert_eq!(t.sync_token(), token, "token unchanged");
    assert_eq!(t.uids().len(), 2, "nothing written");
    assert_eq!(t.event_by_uid("zahnarzt@example.org").await["version"], 1);
    let (state, message) = t.status().await;
    assert_eq!(state, "error");
    assert!(message.unwrap().contains("Privat"));
    // Only the status changed.
    assert!(t.s.app.change_cursor() > cursor);

    t.dav.with(|s| s.multiget_status = None);
    let r = t.sync().await;
    assert_eq!((r.events.added, r.events.updated), (1, 1));
    assert_eq!(
        t.uids(),
        [
            "team-sync@example.org",
            "urlaub@example.org",
            "zahnarzt@example.org"
        ]
    );
    assert_eq!(t.event_by_uid("zahnarzt@example.org").await["version"], 2);
    assert_eq!(t.status().await.0, "ok");
}

#[tokio::test]
async fn unreadable_data_is_stored_and_line_feeds_are_tolerated() {
    let t = setup_with(|dav| {
        dav.with(|s| s.lf_only = true);
        dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"));
        dav.put(
            "kaputt.ics",
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:kaputt@example.org\r\nSUMMARY:ohne Ende\r\n",
        );
    })
    .await;
    let r = t.sync().await;
    assert_eq!((r.events.added, r.events.unreadable), (2, 1));

    let (ics, parse_error): (String, Option<String>) = t
        .db()
        .query_row(
            "SELECT ics, parse_error FROM events WHERE uid = 'kaputt@example.org'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(ics.contains("SUMMARY:ohne Ende\r\n"), "{ics:?}");
    assert!(parse_error.unwrap().contains("END:"));
    let broken = t.event_by_uid("kaputt@example.org").await;
    assert_eq!(broken["title"], "(nicht lesbarer Termin)");
    assert_eq!(broken["start"], Value::Null);
    assert!(broken["parse_error"].is_string());

    // LF from the REPORT answer is stored as CRLF and parses.
    let ics: String = t
        .db()
        .query_row(
            "SELECT ics FROM events WHERE uid = 'zahnarzt@example.org'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ics, single("zahnarzt@example.org", "Zahnarzt"));
    assert_eq!(
        t.event_by_uid("zahnarzt@example.org").await["title"],
        "Zahnarzt"
    );
}

#[tokio::test]
async fn unauthorized_sets_auth_failed() {
    let t = setup().await;
    t.sync().await;
    t.dav.with(|s| s.unauthorized = true);
    let err = t.s.app.sync_account(&t.account_id).await.unwrap_err();
    assert_eq!(err.code(), "invalid_credentials");
    let (state, message) = t.status().await;
    assert_eq!(state, "auth_failed");
    assert!(message.unwrap().contains("Zugangsdaten"));
    assert_eq!(t.uids().len(), 2, "events stay");
}

#[tokio::test]
async fn vanished_calendar_deletes_its_events_with_tombstones() {
    let t = setup().await;
    t.sync().await;
    let ids: Vec<String> = t
        .events()
        .await
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect();
    t.dav.with(|s| s.calendar_gone = true);
    let r = t.sync().await;
    assert_eq!(r.calendars.removed, 1);
    assert!(t.uids().is_empty());
    let mut tomb = t.event_tombstones();
    tomb.sort();
    let mut ids = ids;
    ids.sort();
    assert_eq!(tomb, ids);
}

#[tokio::test]
async fn sync_endpoint_runs_the_event_sync() {
    let t = setup().await;
    let path = format!("/api/accounts/{}/sync", t.account_id);
    let r =
        t.s.request(Method::POST, &path, Auth::Bearer(BOOTSTRAP), None)
            .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while t.s.app.sync_running(&t.account_id) {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(t.uids().len(), 2);
    assert_eq!(t.status().await.0, "ok");
}

#[tokio::test]
async fn occurrences_are_expanded_in_the_range() {
    let t = setup_with(|dav| {
        dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"));
        dav.put("team.ics", &series("team-sync@example.org"));
        dav.put("urlaub.ics", &all_day("urlaub@example.org"));
    })
    .await;
    t.sync().await;

    let r =
        t.s.get(
            "/api/occurrences?from=2026-11-01&to=2026-12-01&time_zone=Europe/Berlin",
            Auth::Bearer(BOOTSTRAP),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let body = r.json();
    assert_eq!(body["truncated"], false);
    let occ = body["occurrences"].as_array().unwrap();
    let summary: Vec<(String, String)> = occ
        .iter()
        .map(|o| {
            let start = o["start"]["utc"]
                .as_str()
                .or(o["start"]["date"].as_str())
                .unwrap()
                .to_owned();
            (start, o["title"].as_str().unwrap().to_owned())
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("2026-11-02T08:00:00Z".to_owned(), "Team-Sync".to_owned()),
            ("2026-11-03T13:00:00Z".to_owned(), "Zahnarzt".to_owned()),
            ("2026-11-05".to_owned(), "Wanderwochenende".to_owned()),
            (
                "2026-11-09T09:00:00Z".to_owned(),
                "Team-Sync (verschoben)".to_owned()
            ),
            // 16 Nov is excluded (EXDATE).
            ("2026-11-23T08:00:00Z".to_owned(), "Team-Sync".to_owned()),
        ]
    );
    let moved = &occ[3];
    assert_eq!(moved["is_override"], true);
    assert_eq!(
        moved["recurrence_id"],
        json!({"date_time": "2026-11-09T09:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T08:00:00Z"})
    );
    assert_eq!(
        moved["start"],
        json!({"date_time": "2026-11-09T10:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-09T09:00:00Z"})
    );
    let urlaub = &occ[2];
    assert_eq!(urlaub["all_day"], true);
    assert_eq!(urlaub["end"], json!({"date": "2026-11-07"}));
    assert_eq!(urlaub["availability"], "free");
    assert_eq!(urlaub["recurrence_id"], Value::Null);
    assert_eq!(occ[1]["calendar_id"], t.calendar_id.as_str());
    assert!(occ[1]["event_id"].as_str().unwrap().starts_with("ev_"));

    // Filter by calendar, RFC 3339 bounds.
    let r = t
        .s
        .get(
            &format!(
                "/api/occurrences?from=2026-11-03T00:00:00Z&to=2026-11-04T00:00:00Z&calendar_ids={}",
                t.calendar_id
            ),
            Auth::Bearer(BOOTSTRAP),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["occurrences"].as_array().unwrap().len(), 1);

    for bad in [
        "/api/occurrences?to=2026-12-01",
        "/api/occurrences?from=morgen&to=2026-12-01",
        "/api/occurrences?from=2026-12-01&to=2026-11-01",
        "/api/occurrences?from=2026-01-01&to=2028-01-01",
        "/api/occurrences?from=2026-11-01&to=2026-12-01&calendar_ids=cal_unbekannt",
        "/api/occurrences?from=2026-11-01&to=2026-12-01&time_zone=Mars/Olymp",
        "/api/occurrences?from=2026-11-01&to=2026-12-01&foo=1",
    ] {
        let r = t.s.get(bad, Auth::Bearer(BOOTSTRAP)).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}: {}", r.text());
        assert_eq!(r.error_code(), "invalid_request");
    }
    let r =
        t.s.get("/api/occurrences?from=2026-11-01&to=2026-12-01", Auth::None)
            .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn concurrent_runs_of_one_account_are_refused() {
    let t = setup().await;
    let (a, b) = tokio::join!(
        t.s.app.sync_account(&t.account_id),
        t.s.app.sync_account(&t.account_id)
    );
    assert!(a.is_ok() != b.is_ok(), "exactly one runs");
    let err = a.err().or(b.err()).unwrap();
    assert_eq!(err.code(), "conflict");
    assert_eq!(t.uids().len(), 2);
}

#[tokio::test]
async fn scheduler_syncs_every_account_at_startup_and_stops_on_shutdown() {
    let t = setup().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(lichtung_server::serve(
        listener,
        t.s.app.clone(),
        t.s.router.clone(),
        async move {
            let _ = stopped.await;
        },
        Duration::from_millis(300),
    ));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while t.uids().len() < 2 || t.status().await.0 != "ok" {
        assert!(
            tokio::time::Instant::now() < deadline,
            "scheduler did not sync"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("serve returns")
        .unwrap()
        .unwrap();
}
