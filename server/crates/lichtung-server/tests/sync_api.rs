//! The sync API (LIC-11) over HTTP: pull with cursor and paging, deletions,
//! conflicts, `410 cursor_expired`, SSE, push with idempotency, order,
//! `not_attempted`, limits and validation. Against the invented CalDAV
//! server; all data is made up.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use common::dav::{setup, single, Setup, CALENDAR};
use common::{api_token, device_token, Auth, TestServer, BOOTSTRAP};

fn zoned(local: &str) -> Value {
    json!({"date_time": local, "time_zone": "Europe/Berlin"})
}

fn create(id: &str, calendar_id: &str, title: &str) -> Value {
    json!({"id": id, "op": "create_event", "calendar_id": calendar_id,
           "event": {"title": title, "start": zoned("2026-11-05T18:00:00"),
                     "end": zoned("2026-11-05T19:00:00")}})
}

async fn pull(s: &TestServer, since: i64, limit: u32) -> Value {
    let r = s
        .get(
            &format!("/api/sync/pull?since={since}&limit={limit}"),
            Auth::Bearer(BOOTSTRAP),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    r.json()
}

/// `(type, op, id)` of a page.
fn entries(page: &Value) -> Vec<(String, String, String)> {
    page["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let id = c["id"].as_str().or(c["data"]["id"].as_str()).unwrap();
            (
                c["type"].as_str().unwrap().to_owned(),
                c["op"].as_str().unwrap().to_owned(),
                id.to_owned(),
            )
        })
        .collect()
}

fn assert_unique(page: &Value) {
    let all = entries(page);
    let ids: HashSet<(String, String)> = all
        .iter()
        .map(|(t, _, id)| (t.clone(), id.clone()))
        .collect();
    assert_eq!(ids.len(), all.len(), "an object twice in one page: {page}");
}

async fn push(s: &TestServer, auth: Auth<'_>, mutations: Vec<Value>) -> Vec<Value> {
    let r = s
        .post("/api/sync/push", auth, json!({"mutations": mutations}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    r.json()["results"].as_array().unwrap().clone()
}

async fn synced() -> Setup {
    let t = setup().await;
    t.sync().await;
    t
}

#[tokio::test]
async fn first_pull_returns_everything_once() {
    let t = synced().await;
    let page = pull(&t.s, 0, 500).await;
    assert_eq!(page["has_more"], false);
    assert_eq!(page["cursor"], t.s.app.change_cursor());
    assert_unique(&page);
    let kinds: Vec<String> = entries(&page).into_iter().map(|(k, _, _)| k).collect();
    assert_eq!(kinds.iter().filter(|k| *k == "account").count(), 1);
    assert_eq!(kinds.iter().filter(|k| *k == "calendar").count(), 1);
    assert_eq!(kinds.iter().filter(|k| *k == "event").count(), 2);
    let account = page["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "account")
        .unwrap();
    assert_eq!(account["op"], "upsert");
    assert_eq!(account["data"]["id"], t.account_id);
    assert_eq!(account["data"]["status"]["state"], "ok");
    let event = page["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "event" && c["data"]["uid"] == "team-sync@example.org")
        .unwrap();
    assert_eq!(
        event["data"]["recurrence"]["rrule"],
        "FREQ=WEEKLY;BYDAY=MO;COUNT=4"
    );
    // Nothing new: an empty page with the same cursor.
    let again = pull(&t.s, page["cursor"].as_i64().unwrap(), 500).await;
    assert_eq!(again["changes"], json!([]));
    assert_eq!(again["cursor"], page["cursor"]);
}

#[tokio::test]
async fn paging_covers_everything_in_order() {
    let t = synced().await;
    let full = entries(&pull(&t.s, 0, 500).await);
    let mut seen = Vec::new();
    let mut since = 0;
    let mut pages = 0;
    loop {
        let page = pull(&t.s, since, 2).await;
        assert_unique(&page);
        assert!(page["changes"].as_array().unwrap().len() <= 2);
        seen.extend(entries(&page));
        since = page["cursor"].as_i64().unwrap();
        pages += 1;
        if page["has_more"] == false {
            break;
        }
        assert!(pages < 20, "paging does not end");
    }
    assert!(pages >= 2);
    assert_eq!(seen, full);
    assert_eq!(since, t.s.app.change_cursor());

    // A limit above the maximum is capped, 0 is refused.
    assert_eq!(
        t.s.get("/api/sync/pull?since=0&limit=5000", Auth::Bearer(BOOTSTRAP))
            .await
            .status,
        StatusCode::OK
    );
    for bad in ["limit=0", "since=-1", "since=abc", "seit=0"] {
        let r =
            t.s.get(&format!("/api/sync/pull?{bad}"), Auth::Bearer(BOOTSTRAP))
                .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(r.error_code(), "invalid_request");
    }
}

#[tokio::test]
async fn deletions_and_newest_state() {
    let t = synced().await;
    let cursor = pull(&t.s, 0, 500).await["cursor"].as_i64().unwrap();
    let zahnarzt = t.event_by_uid("zahnarzt@example.org").await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let team = t.event_by_uid("team-sync@example.org").await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Two changes of one event, one deletion.
    t.dav.put(
        "zahnarzt.ics",
        &single("zahnarzt@example.org", "Zahnarzt 2"),
    );
    t.sync().await;
    t.dav.put(
        "zahnarzt.ics",
        &single("zahnarzt@example.org", "Zahnarzt 3"),
    );
    t.dav.remove("team.ics");
    t.sync().await;

    let page = pull(&t.s, cursor, 500).await;
    assert_unique(&page);
    let changes = page["changes"].as_array().unwrap();
    let updated: Vec<&Value> = changes
        .iter()
        .filter(|c| c["type"] == "event" && c["op"] == "upsert")
        .collect();
    assert_eq!(updated.len(), 1, "{page}");
    assert_eq!(updated[0]["data"]["id"], zahnarzt);
    assert_eq!(updated[0]["data"]["title"], "Zahnarzt 3");
    assert_eq!(updated[0]["data"]["version"], 3);
    assert!(changes
        .iter()
        .any(|c| c == &json!({"type": "event", "op": "delete", "id": team})));
}

#[tokio::test]
async fn conflicts_appear_in_the_pull_and_can_be_dismissed() {
    let t = synced().await;
    let device = device_token(&t.s, "iPhone von Anna");
    let ev = t.event_by_uid("zahnarzt@example.org").await;
    t.dav.put(
        "zahnarzt.ics",
        &single("zahnarzt@example.org", "Zahnarzt (geändert)"),
    );
    let cursor = t.s.app.change_cursor();
    let results = push(
        &t.s,
        Auth::Bearer(&device),
        vec![
            json!({"id": "m-konflikt", "op": "update_event", "event_id": ev["id"],
                    "base_version": 1, "changes": {"title": "Zahnarzt (meins)"}}),
        ],
    )
    .await;
    assert_eq!(results[0]["status"], "conflict", "{results:?}");
    assert_eq!(results[0]["conflict"]["origin"], "app:iPhone von Anna");
    let conflict_id = results[0]["conflict"]["id"].as_str().unwrap().to_owned();

    let page = pull(&t.s, cursor, 500).await;
    let conflict = page["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "conflict")
        .unwrap_or_else(|| panic!("{page}"));
    assert_eq!(conflict["op"], "upsert");
    assert_eq!(conflict["data"]["id"], conflict_id);
    assert_eq!(
        conflict["data"]["mutation"]["changes"]["title"],
        "Zahnarzt (meins)"
    );
    let event = page["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "event")
        .unwrap();
    assert_eq!(event["data"]["title"], "Zahnarzt (geändert)");

    let listed =
        t.s.get("/api/conflicts", Auth::Bearer(&device))
            .await
            .json();
    assert_eq!(listed[0]["id"], conflict_id);

    let cursor = page["cursor"].as_i64().unwrap();
    let r =
        t.s.post(
            &format!("/api/conflicts/{conflict_id}/dismiss"),
            Auth::Bearer(&device),
            json!({}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.text());
    let page = pull(&t.s, cursor, 500).await;
    assert_eq!(
        page["changes"],
        json!([{"type": "conflict", "op": "delete", "id": conflict_id}])
    );
    let r =
        t.s.post(
            &format!("/api/conflicts/{conflict_id}/dismiss"),
            Auth::Bearer(&device),
            json!({}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn old_cursors_expire() {
    let t = synced().await;
    t.dav.remove("team.ics");
    t.sync().await;
    let now = lichtung_server::app::now_ms();
    // Nothing is old yet.
    let purged = t.s.app.purge_sync_history(now).await.unwrap();
    assert_eq!(purged.tombstones, 0);
    let r =
        t.s.get("/api/sync/pull?since=1", Auth::Bearer(BOOTSTRAP))
            .await;
    assert_eq!(r.status, StatusCode::OK);
    // 91 days later the tombstone is gone, and so are old cursors.
    let later = now + 91 * 24 * 60 * 60 * 1000;
    let purged = t.s.app.purge_sync_history(later).await.unwrap();
    assert_eq!(purged.tombstones, 1);
    let r =
        t.s.get("/api/sync/pull?since=1", Auth::Bearer(BOOTSTRAP))
            .await;
    assert_eq!(r.status, StatusCode::GONE, "{}", r.text());
    assert_eq!(r.error_code(), "cursor_expired");
    // From scratch still works, and a current cursor too.
    let page = pull(&t.s, 0, 500).await;
    let cursor = page["cursor"].as_i64().unwrap();
    pull(&t.s, cursor, 500).await;
    // A cursor from the future (e.g. a restored database) expires as well.
    let r =
        t.s.get(
            &format!("/api/sync/pull?since={}", cursor + 10),
            Auth::Bearer(BOOTSTRAP),
        )
        .await;
    assert_eq!(r.status, StatusCode::GONE);
}

/// Reads the next SSE frame (skipping keep-alive comments).
async fn next_frame(body: &mut Body) -> String {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("an SSE frame within 5 s")
            .expect("stream open")
            .unwrap();
        if let Ok(data) = frame.into_data() {
            let text = String::from_utf8(data.to_vec()).unwrap();
            if !text.starts_with(':') {
                return text;
            }
        }
    }
}

#[tokio::test]
async fn sse_sends_the_cursor_at_once_and_after_changes() {
    let s = TestServer::new().await;
    let req = Request::builder()
        .uri("/api/sync/events")
        .header(header::AUTHORIZATION, format!("Bearer {BOOTSTRAP}"))
        .header(header::ACCEPT, "text/event-stream")
        .header(header::HOST, "localhost")
        .body(Body::empty())
        .unwrap();
    let response = s.router.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    assert_eq!(response.headers()["x-accel-buffering"], "no");
    let mut body = response.into_body();
    let cursor = s.app.change_cursor();
    assert_eq!(
        next_frame(&mut body).await,
        format!("event: cursor\ndata: {cursor}\n\n")
    );

    s.app
        .create_calendar_set(serde_json::from_value(json!({"name": "Ehrenamt"})).unwrap())
        .await
        .unwrap();
    let after = s.app.change_cursor();
    assert!(after > cursor);
    assert_eq!(
        next_frame(&mut body).await,
        format!("event: cursor\ndata: {after}\n\n")
    );

    // Shutdown ends the stream.
    s.app.shutdown_token().cancel();
    let end = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .expect("stream ends");
    assert!(end.is_none(), "no more frames after shutdown");
}

#[tokio::test]
async fn push_is_idempotent_per_mutation_id() {
    let t = synced().await;
    let device = device_token(&t.s, "Mac");
    let m = create(
        "6f1d1a52-0000-4000-8000-000000000001",
        &t.calendar_id,
        "Chorprobe",
    );
    let first = push(&t.s, Auth::Bearer(&device), vec![m.clone()]).await;
    assert_eq!(first[0]["status"], "applied", "{first:?}");
    assert_eq!(first[0]["id"], "6f1d1a52-0000-4000-8000-000000000001");
    assert_eq!(first[0]["events"][0]["title"], "Chorprobe");
    let puts = t.dav.put_count();
    let second = push(&t.s, Auth::Bearer(&device), vec![m.clone(), m]).await;
    assert_eq!(second, vec![first[0].clone(), first[0].clone()]);
    assert_eq!(t.dav.put_count(), puts, "no second PUT at the provider");
    assert_eq!(t.dav.names_in(CALENDAR).len(), 3);
}

#[tokio::test]
async fn push_keeps_the_order_and_stops_after_retry() {
    let t = synced().await;
    let token = api_token(&t.s, "werkzeug").await;
    let bad = json!({"id": "m1", "op": "create_event", "calendar_id": t.calendar_id,
                     "event": {"title": "Ohne Zeit"}});
    let batch = vec![
        bad,
        create("m2", &t.calendar_id, "Erst später"),
        create("m3", &t.calendar_id, "Danach"),
    ];
    t.dav.with(|s| s.put_status = Some(503));
    let results = push(&t.s, Auth::Bearer(&token), batch.clone()).await;
    let statuses: Vec<&str> = results
        .iter()
        .map(|r| r["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["rejected", "retry", "retry"]);
    assert_eq!(results[0]["error"]["code"], "invalid_event");
    assert_eq!(results[1]["error"]["code"], "provider_unavailable");
    assert_eq!(results[1]["retry_after_seconds"], 60);
    assert_eq!(results[2]["error"]["code"], "not_attempted");
    assert_eq!(
        results[2]["error"]["message"],
        "Nicht versucht, weil eine vorherige Änderung warten muss."
    );
    assert_eq!(
        results.iter().map(|r| r["id"].clone()).collect::<Vec<_>>(),
        [json!("m1"), json!("m2"), json!("m3")]
    );

    // Later: the rejected one keeps its answer, the others go through.
    t.dav.with(|s| s.put_status = None);
    let again = push(&t.s, Auth::Bearer(&token), batch).await;
    assert_eq!(again[0], results[0]);
    assert_eq!(again[1]["status"], "applied", "{again:?}");
    assert_eq!(again[2]["status"], "applied");
    assert!(t.dav.find_uid(CALENDAR, "zahnarzt@example.org").is_some());
}

#[tokio::test]
async fn push_limits_and_validation() {
    let t = synced().await;
    // 101 mutations: refused as a whole.
    let many: Vec<Value> = (0..101)
        .map(|i| create(&format!("m{i}"), &t.calendar_id, "Viel"))
        .collect();
    let r =
        t.s.post(
            "/api/sync/push",
            Auth::Bearer(BOOTSTRAP),
            json!({"mutations": many}),
        )
        .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE, "{}", r.text());
    assert_eq!(r.error_code(), "payload_too_large");

    // 100 mutations with long notes (more than the usual 64 KiB) are fine.
    let notes = "Wegbeschreibung ".repeat(60);
    let hundred: Vec<Value> = (0..100)
        .map(|i| {
            json!({"id": format!("v{i}"), "op": "create_event", "calendar_id": t.calendar_id,
                   "event": {"title": "", "notes": notes, "start": {"date": "2026-11-05"},
                             "end": {"date": "2026-11-06"}}})
        })
        .collect();
    let results = push(&t.s, Auth::Bearer(BOOTSTRAP), hundred).await;
    assert_eq!(results.len(), 100);
    assert!(results
        .iter()
        .all(|r| r["error"]["code"] == "invalid_event"));

    let cases = [
        json!({"title": "Rückwärts", "start": zoned("2026-11-05T18:00:00"),
               "end": zoned("2026-11-05T17:00:00")}),
        json!({"title": "Gemischt", "start": {"date": "2026-11-05"},
               "end": zoned("2026-11-05T17:00:00")}),
        json!({"start": zoned("2026-11-05T18:00:00"), "end": zoned("2026-11-05T19:00:00")}),
    ];
    let mutations: Vec<Value> = cases
        .iter()
        .enumerate()
        .map(|(i, e)| {
            json!({"id": format!("c{i}"), "op": "create_event", "calendar_id": t.calendar_id,
                   "event": e})
        })
        .collect();
    let results = push(&t.s, Auth::Bearer(BOOTSTRAP), mutations).await;
    for r in &results {
        assert_eq!(r["status"], "rejected", "{r}");
        assert_eq!(r["error"]["code"], "invalid_event", "{r}");
    }
    assert!(results[0]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Ende liegt vor dem Beginn"));
    assert!(results[1]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("dieselbe Form"));
    assert!(results[2]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("title fehlt"));

    // A mutation without id breaks the whole request.
    let r =
        t.s.post(
            "/api/sync/push",
            Auth::Bearer(BOOTSTRAP),
            json!({"mutations": [{"op": "create_event"}]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.text().contains("mutations[0].id"));
    // Unauthenticated: 401.
    let r =
        t.s.post("/api/sync/push", Auth::None, json!({"mutations": []}))
            .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn browser_pushes_need_the_client_header() {
    let t = synced().await;
    let session = t.s.session().await;
    let m = vec![create("w1", &t.calendar_id, "Aus dem Browser")];
    let r =
        t.s.post(
            "/api/sync/push",
            Auth::Cookie(&session),
            json!({"mutations": m}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let results = push(&t.s, Auth::Web(&session), m).await;
    assert_eq!(results[0]["status"], "applied", "{results:?}");
}
