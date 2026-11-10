//! Blockers (LIC-75) against the invented, stateful CalDAV server: rules
//! through the management API, blockers that follow their source (create,
//! move, delete, series, what does not block), blockers edited elsewhere,
//! rules in both directions, failed writes, deleted rules. All data is
//! made up.

mod common;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use common::dav::{all_day, series, setup_with, single, Setup, CALENDAR, WORK};
use common::{api_token, device_token, Auth, TestResponse};

/// "Now" for the rounds: before the invented events of November 2026.
fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

struct T {
    t: Setup,
    device: String,
    work_id: String,
}

/// "Privat" with an appointment, a writable "Arbeit", a device of the app.
async fn prepared() -> T {
    let t = setup_with(|dav| {
        dav.add_calendar(WORK, "Arbeit", true);
        dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"));
    })
    .await;
    t.sync().await;
    let device = device_token(&t.s, "iPhone von Anna");
    let work_id = t.calendar_named("Arbeit").await;
    T { t, device, work_id }
}

impl T {
    async fn api(&self, method: Method, path: &str, body: Option<Value>) -> TestResponse {
        self.t
            .s
            .request(method, path, Auth::Bearer(&self.device), body)
            .await
    }

    /// Rule Privat → Arbeit; panics unless `201`.
    async fn rule(&self, extra: Value) -> Value {
        let mut body = json!({"source_calendar_id": self.t.calendar_id,
                              "target_calendar_id": self.work_id});
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        let r = self
            .api(Method::POST, "/api/blocker-rules", Some(body))
            .await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
        r.json()
    }

    async fn round(&self) -> lichtung_server::blockers::BlockerReport {
        self.t.s.app.reconcile_blockers(now()).await.unwrap()
    }

    async fn work_events(&self) -> Vec<Value> {
        self.t.s.app.calendar_events(&self.work_id).await.unwrap()
    }

    /// Resources in "Arbeit" at the provider.
    fn at_provider(&self) -> Vec<(String, String)> {
        self.t
            .dav
            .names_in(WORK)
            .into_iter()
            .map(|n| {
                let (_, data) = self.t.dav.get_in(WORK, &n).unwrap();
                (n, data)
            })
            .collect()
    }
}

#[tokio::test]
async fn a_blocker_follows_its_source() {
    let t = prepared().await;
    let rule = t.rule(json!({})).await;
    assert_eq!(rule["title"], "Blockiert");
    assert_eq!(rule["all_day"], false);
    assert_eq!(rule["blocker_count"], 0);

    let report = t.round().await;
    assert_eq!(report.written, 1, "{report:?}");
    let source = t.t.event_by_uid("zahnarzt@example.org").await;
    let blockers = t.work_events().await;
    assert_eq!(blockers.len(), 1);
    let b = &blockers[0];
    assert_eq!(b["title"], "Blockiert");
    assert_eq!(b["availability"], "busy");
    assert_eq!(b["location"], Value::Null, "nothing of the source leaks");
    assert_eq!(b["start"], source["start"]);
    assert_eq!(b["end"], source["end"]);
    assert_eq!(b["blocker_of"], source["id"]);
    assert!(source.get("blocker_of").is_none());
    let provider = t.at_provider();
    assert_eq!(provider.len(), 1);
    assert!(!provider[0].1.contains("Zahnarzt"));
    assert!(!provider[0].1.contains("Praxis"));

    // Nothing changed: nothing written.
    let puts = t.t.dav.put_count();
    assert_eq!(t.round().await, Default::default());
    assert_eq!(t.t.dav.put_count(), puts);
    let listed = t.api(Method::GET, "/api/blocker-rules", None).await.json();
    assert_eq!(listed[0]["blocker_count"], 1);

    // Moved at the provider: the blocker moves along.
    let moved = single("zahnarzt@example.org", "Zahnarzt").replace("T140000", "T160000");
    t.t.dav
        .put("zahnarzt.ics", &moved.replace("T150000", "T170000"));
    t.t.sync().await;
    assert_eq!(t.round().await.written, 1);
    let b = &t.work_events().await[0];
    assert_eq!(b["start"]["date_time"], "2026-11-03T16:00:00");
    assert_eq!(b["end"]["date_time"], "2026-11-03T17:00:00");
    assert_eq!(t.at_provider().len(), 1, "same resource, rewritten");

    // Deleted: the blocker goes too.
    t.t.dav.remove("zahnarzt.ics");
    t.t.sync().await;
    assert_eq!(t.round().await.deleted, 1);
    assert!(t.work_events().await.is_empty());
    assert!(t.at_provider().is_empty());
}

#[tokio::test]
async fn series_and_what_does_not_block() {
    let t = prepared().await;
    t.t.dav.put("team.ics", &series("team-sync@example.org"));
    t.t.dav.put("wandern.ics", &all_day("wandern@example.org"));
    t.t.sync().await;
    t.rule(json!({"title": "Belegt", "all_day": true})).await;
    t.round().await;

    let blockers = t.work_events().await;
    // The all-day event is free (TRANSP:TRANSPARENT): no blocker.
    assert_eq!(blockers.len(), 2, "{blockers:#?}");
    let team = t.t.event_by_uid("team-sync@example.org").await;
    let b = blockers
        .iter()
        .find(|b| b["blocker_of"] == team["id"])
        .unwrap();
    assert_eq!(b["title"], "Belegt");
    assert_eq!(b["recurrence"], team["recurrence"]);
    assert_eq!(b["overrides"].as_array().unwrap().len(), 1);
    assert_eq!(b["overrides"][0]["start"], team["overrides"][0]["start"]);
    assert_eq!(b["overrides"][0]["title"], "Belegt");

    // Marked free at the provider: the blocker is removed.
    let free = single("zahnarzt@example.org", "Zahnarzt")
        .replace("SUMMARY:Zahnarzt", "SUMMARY:Zahnarzt\r\nTRANSP:TRANSPARENT");
    t.t.dav.put("zahnarzt.ics", &free);
    t.t.sync().await;
    assert_eq!(t.round().await.deleted, 1);
    assert_eq!(t.work_events().await.len(), 1);
}

#[tokio::test]
async fn a_blocker_changed_elsewhere_is_put_back() {
    let t = prepared().await;
    t.rule(json!({})).await;
    t.round().await;
    let (name, data) = t.at_provider().remove(0);

    // Renamed by hand at the provider.
    t.t.dav.put_in(
        WORK,
        &name,
        &data.replace("SUMMARY:Blockiert", "SUMMARY:Privat"),
    );
    t.t.sync().await;
    assert_eq!(t.work_events().await[0]["title"], "Privat");
    assert_eq!(t.round().await.written, 1);
    assert_eq!(t.work_events().await[0]["title"], "Blockiert");

    // Deleted by hand: it comes back.
    t.t.dav.with(|s| {
        s.calendars.get_mut(WORK).unwrap().resources.remove(&name);
    });
    t.t.sync().await;
    assert!(t.work_events().await.is_empty());
    assert_eq!(t.round().await.written, 1);
    assert_eq!(t.work_events().await.len(), 1);
}

#[tokio::test]
async fn title_change_rewrites_and_rule_deletion_removes() {
    let t = prepared().await;
    let rule = t.rule(json!({})).await;
    t.round().await;
    let id = rule["id"].as_str().unwrap();

    let r = t
        .api(
            Method::PATCH,
            &format!("/api/blocker-rules/{id}"),
            Some(json!({"title": "Termin"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["title"], "Termin");
    assert_eq!(t.round().await.written, 1);
    assert_eq!(t.work_events().await[0]["title"], "Termin");

    let r = t
        .api(Method::DELETE, &format!("/api/blocker-rules/{id}"), None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(t.round().await.deleted, 1);
    assert!(t.at_provider().is_empty());
    let r = t
        .api(Method::GET, &format!("/api/blocker-rules/{id}"), None)
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    // The source stays untouched.
    assert_eq!(
        t.t.event_by_uid("zahnarzt@example.org").await["title"],
        "Zahnarzt"
    );
}

#[tokio::test]
async fn rules_in_both_directions_do_not_feed_each_other() {
    let t = prepared().await;
    t.t.dav.put_in(
        WORK,
        "standup.ics",
        &single("standup@example.org", "Standup"),
    );
    t.t.sync().await;
    t.rule(json!({})).await;
    let r = t
        .api(
            Method::POST,
            "/api/blocker-rules",
            Some(json!({"source_calendar_id": t.work_id,
                        "target_calendar_id": t.t.calendar_id})),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert_eq!(t.round().await.written, 2);
    assert_eq!(t.round().await, Default::default());
    // Each calendar: its own event and one blocker.
    assert_eq!(t.work_events().await.len(), 2);
    assert_eq!(t.t.events().await.len(), 2);
    assert_eq!(t.t.dav.names_in(CALENDAR).len(), 2);
}

#[tokio::test]
async fn failed_writes_are_shown_and_retried() {
    let t = prepared().await;
    t.rule(json!({})).await;
    t.t.dav.with(|s| s.put_status = Some(503));
    let report = t.round().await;
    assert_eq!(report.failed, 1, "{report:?}");
    let rule = &t.api(Method::GET, "/api/blocker-rules", None).await.json()[0];
    assert_eq!(rule["failed_count"], 1);
    assert!(rule["last_error"]
        .as_str()
        .unwrap()
        .contains("nicht erreichbar"));

    t.t.dav.with(|s| s.put_status = None);
    assert_eq!(t.round().await.written, 1);
    let rule = &t.api(Method::GET, "/api/blocker-rules", None).await.json()[0];
    assert_eq!(rule["failed_count"], 0);
    assert_eq!(rule["blocker_count"], 1);
    assert_eq!(t.at_provider().len(), 1);
}

#[tokio::test]
async fn a_lost_answer_does_not_duplicate() {
    let t = prepared().await;
    t.rule(json!({})).await;
    // Written at the provider, but the answer got lost.
    t.t.dav.with(|s| s.put_status_after_write = Some(504));
    assert_eq!(t.round().await.failed, 1);
    t.t.dav.with(|s| s.put_status_after_write = None);
    assert_eq!(t.at_provider().len(), 1);
    assert_eq!(t.round().await.written, 1);
    assert_eq!(t.at_provider().len(), 1, "same UID, overwritten");
    t.t.sync().await;
    assert_eq!(t.work_events().await.len(), 1);
}

#[tokio::test]
async fn rule_checks() {
    let t = setup_with(|dav| dav.add_calendar(WORK, "Arbeit", false)).await;
    t.sync().await;
    let device = device_token(&t.s, "iPhone von Anna");
    let work = t.calendar_named("Arbeit").await;
    let post = |body: Value| {
        let device = device.clone();
        let t = &t;
        async move {
            t.s.request(
                Method::POST,
                "/api/blocker-rules",
                Auth::Bearer(&device),
                Some(body),
            )
            .await
        }
    };

    let r = post(json!({"source_calendar_id": t.calendar_id, "target_calendar_id": work})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.error_code(), "calendar_read_only");
    let r = post(json!({"source_calendar_id": t.calendar_id,
                        "target_calendar_id": t.calendar_id}))
    .await;
    assert_eq!(r.error_code(), "invalid_request");
    let r = post(json!({"source_calendar_id": "cal_gibtsnicht",
                        "target_calendar_id": t.calendar_id}))
    .await;
    assert_eq!(r.error_code(), "calendar_not_found");

    // The read-only calendar as source is fine, once.
    let ok = json!({"source_calendar_id": work, "target_calendar_id": t.calendar_id});
    assert_eq!(post(ok.clone()).await.status, StatusCode::CREATED);
    assert_eq!(post(ok.clone()).await.status, StatusCode::CONFLICT);
    let r = post(
        json!({"source_calendar_id": work, "target_calendar_id": t.calendar_id,
                        "title": "x".repeat(201)}),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // API tokens may read rules, not change them.
    let token = api_token(&t.s, "Skript").await;
    let r =
        t.s.request(
            Method::POST,
            "/api/blocker-rules",
            Auth::Bearer(&token),
            Some(ok),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = t.s.get("/api/blocker-rules", Auth::Bearer(&token)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json().as_array().unwrap().len(), 1);
}
