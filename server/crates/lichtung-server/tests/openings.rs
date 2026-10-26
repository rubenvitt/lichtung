//! Openings (LIC-31) and their placeholders (LIC-32) against the invented,
//! stateful CalDAV server: management API, target calendar rules, slots,
//! placeholders (busy for other links, not for their own), booking,
//! deletion, past suggestions, provider failures, pull. All data is made
//! up.

mod common;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};

use common::dav::{setup_with, single, Setup, CALENDAR, WORK};
use common::{api_token, device_token, Auth, TestResponse, BOOTSTRAP};

/// A day a week ahead, 00:00 UTC (suggestions must lie in the future).
fn day() -> DateTime<Utc> {
    let d = (Utc::now() + Duration::days(7)).date_naive();
    d.and_hms_opt(0, 0, 0).unwrap().and_utc()
}

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    day() + Duration::hours(hour.into()) + Duration::minutes(minute.into())
}

fn rfc(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A setup with the shared, read-only calendar "Arbeit" and a device of
/// the app.
async fn prepared() -> (Setup, String) {
    let t = setup_with(|dav| {
        dav.add_calendar(WORK, "Arbeit", false);
        // Long before the test day, so it does not touch any slot.
        let old = single("zahnarzt@example.org", "Zahnarzt").replace("20261103T", "20200103T");
        dav.put("zahnarzt.ics", &old);
    })
    .await;
    t.sync().await;
    let device = device_token(&t.s, "iPhone von Anna");
    (t, device)
}

impl Setup {
    async fn api(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> TestResponse {
        self.s
            .request(method, path, Auth::Bearer(token), body)
            .await
    }

    /// Creates a link as the app; panics unless `201`.
    async fn create_opening(&self, device: &str, body: Value) -> Value {
        let r = self
            .api(Method::POST, "/api/openings", device, Some(body))
            .await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
        r.json()
    }

    /// A template on all days 08:00–18:00 UTC, 30 minutes, no notice.
    async fn template(&self, device: &str) -> Value {
        let windows: Vec<Value> = ["mo", "tu", "we", "th", "fr", "sa", "su"]
            .iter()
            .map(|d| json!({"weekday": d, "start": "08:00", "end": "18:00"}))
            .collect();
        self.create_opening(
            device,
            json!({"kind": "template", "title": "30 Minuten Kennenlernen", "duration_minutes": 30,
                   "calendar_id": self.calendar_id, "time_zone": "UTC", "windows": windows,
                   "min_notice_minutes": 0, "video_url": "https://meet.example.org/kennenlernen"}),
        )
        .await
    }

    /// A one-off link with suggestions at 10:00 and 14:00 of [`day`].
    async fn once_with_suggestions(&self, device: &str) -> Value {
        self.create_opening(
            device,
            json!({"kind": "once", "title": "Abstimmung", "duration_minutes": 30,
                   "calendar_id": self.calendar_id, "time_zone": "UTC",
                   "suggestions": [{"start": rfc(at(14, 0))},
                                   {"start": rfc(at(10, 0)), "end": rfc(at(10, 30))}]}),
        )
        .await
    }

    async fn slot_starts(&self, opening: &Value) -> Vec<String> {
        self.s
            .app
            .opening_slots(
                opening["id"].as_str().unwrap(),
                at(0, 0),
                at(24, 0),
                at(0, 0) - Duration::days(1),
            )
            .await
            .unwrap()
            .iter()
            .map(|s| s.start.format("%H:%M").to_string())
            .collect()
    }

    fn placeholders(&self) -> Vec<String> {
        self.dav
            .with(|s| {
                s.calendars[CALENDAR]
                    .resources
                    .values()
                    .map(|(_, d)| d.clone())
                    .collect::<Vec<_>>()
            })
            .into_iter()
            .filter(|d| d.contains("SUMMARY:Reserviert: "))
            .collect()
    }

    fn queued_removals(&self) -> i64 {
        self.db()
            .query_row("SELECT COUNT(*) FROM placeholder_removals", [], |r| {
                r.get(0)
            })
            .unwrap()
    }
}

#[tokio::test]
async fn templates_are_managed_by_the_app() {
    let (t, device) = prepared().await;
    let o = t.template(&device).await;
    let id = o["id"].as_str().unwrap();
    assert!(id.starts_with("opn_"), "{o}");
    assert_eq!(o["kind"], "template");
    let url = o["url"].as_str().unwrap();
    let token = url.strip_prefix("http://localhost:8080/o/").expect(url);
    assert_eq!(token.len(), 43, "256 bits, base64url: {url}");
    assert_eq!(o["duration_minutes"], 30);
    assert_eq!(o["buffer_before_minutes"], 0);
    assert_eq!(o["horizon_days"], 21, "default");
    assert_eq!(o["video_url"], "https://meet.example.org/kennenlernen");
    assert_eq!(o["suggestions"], json!([]));
    assert_eq!(o["suggestions_only"], false);
    assert_eq!(o["booked_at"], Value::Null);
    assert!(o.get("token").is_none());

    // Reading is open to every token, changing only to the app.
    let lt = api_token(&t.s, "werkzeug").await;
    let r = t.api(Method::GET, "/api/openings", &lt, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()[0]["id"], id);
    let r = t
        .api(Method::GET, &format!("/api/openings/{id}"), BOOTSTRAP, None)
        .await;
    assert_eq!(r.json()["title"], "30 Minuten Kennenlernen");
    for (method, path, body) in [
        (
            Method::POST,
            "/api/openings".to_owned(),
            Some(json!({"kind": "once", "duration_minutes": 30, "calendar_id": t.calendar_id})),
        ),
        (
            Method::PATCH,
            format!("/api/openings/{id}"),
            Some(json!({"title": "x"})),
        ),
        (Method::DELETE, format!("/api/openings/{id}"), None),
    ] {
        let r = t.api(method, &path, &lt, body).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{path}");
    }

    // PATCH changes the rules; null removes the video link.
    let r = t
        .api(
            Method::PATCH,
            &format!("/api/openings/{id}"),
            &device,
            Some(
                json!({"title": "Kennenlernen", "buffer_after_minutes": 15, "video_url": null,
                        "windows": [{"weekday": "TU", "start": "9:00", "end": "12:00"}]}),
            ),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let o = r.json();
    assert_eq!(o["title"], "Kennenlernen");
    assert_eq!(o["buffer_after_minutes"], 15);
    assert_eq!(o["video_url"], Value::Null);
    assert_eq!(
        o["windows"],
        json!([{"weekday": "tu", "start": "09:00", "end": "12:00"}])
    );
    assert_eq!(o["url"], url, "the link stays");
    let r = t
        .api(
            Method::PATCH,
            &format!("/api/openings/{id}"),
            &device,
            Some(json!({"horizon_days": 0})),
        )
        .await;
    assert_eq!(r.error_code(), "invalid_request");
    let r = t
        .api(
            Method::PATCH,
            &format!("/api/openings/{id}"),
            &device,
            Some(json!({"kind": "once"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // The booking page finds it by its token.
    let found = t.s.app.opening_by_token(token).await.unwrap().unwrap();
    assert_eq!(found.id, id);
    assert!(t
        .s
        .app
        .opening_by_token("unbekannt")
        .await
        .unwrap()
        .is_none());

    let r = t
        .api(
            Method::DELETE,
            &format!("/api/openings/{id}"),
            &device,
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = t
        .api(Method::GET, &format!("/api/openings/{id}"), &device, None)
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(t.tombstones_of("opening"), [id]);
}

#[tokio::test]
async fn target_calendar_must_take_invitations() {
    let (t, device) = prepared().await;
    let work = t.calendar_named("Arbeit").await;
    let body =
        |calendar: &str| json!({"kind": "once", "duration_minutes": 30, "calendar_id": calendar});
    let r = t
        .api(
            Method::POST,
            "/api/openings",
            &device,
            Some(body("cal_unbekannt")),
        )
        .await;
    assert_eq!(
        (r.status, r.error_code().as_str()),
        (StatusCode::BAD_REQUEST, "calendar_not_found")
    );
    let r = t
        .api(Method::POST, "/api/openings", &device, Some(body(&work)))
        .await;
    assert_eq!(r.error_code(), "calendar_read_only", "{}", r.text());
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Arbeit"));
    t.db()
        .execute(
            "UPDATE calendars SET can_invite = 0 WHERE id = ?1",
            [&t.calendar_id],
        )
        .unwrap();
    let r = t
        .api(
            Method::POST,
            "/api/openings",
            &device,
            Some(body(&t.calendar_id)),
        )
        .await;
    assert_eq!(r.error_code(), "calendar_cannot_invite", "{}", r.text());

    // Templates need a title; suggestions only for one-off links.
    t.db()
        .execute(
            "UPDATE calendars SET can_invite = 1 WHERE id = ?1",
            [&t.calendar_id],
        )
        .unwrap();
    let r = t
        .api(
            Method::POST,
            "/api/openings",
            &device,
            Some(json!({"kind": "template", "duration_minutes": 30, "calendar_id": t.calendar_id})),
        )
        .await;
    assert_eq!(r.error_code(), "invalid_request");
    let r = t
        .api(
            Method::POST,
            "/api/openings",
            &device,
            Some(
                json!({"kind": "template", "title": "x", "duration_minutes": 30,
                        "calendar_id": t.calendar_id,
                        "suggestions": [{"start": rfc(at(10, 0))}]}),
            ),
        )
        .await;
    assert_eq!(r.error_code(), "invalid_request");

    // A one-off link without suggestions gets the defaults.
    let o = t.create_opening(&device, body(&t.calendar_id)).await;
    assert_eq!(o["title"], "Termin (30 Minuten)");
    assert_eq!(o["min_notice_minutes"], 240);
    assert_eq!(o["windows"].as_array().unwrap().len(), 5);
    assert_eq!(
        o["windows"][0],
        json!({"weekday": "mo", "start": "09:00", "end": "17:00"})
    );
    assert!(t.placeholders().is_empty());
}

#[tokio::test]
async fn placeholders_block_other_links_but_not_their_own() {
    let (t, device) = prepared().await;
    let once = t.once_with_suggestions(&device).await;
    let suggestions = once["suggestions"].as_array().unwrap();
    assert_eq!(suggestions.len(), 2);
    assert_eq!(suggestions[0]["start"], rfc(at(10, 0)));
    assert_eq!(suggestions[1]["end"], rfc(at(14, 30)));
    assert_eq!(once["suggestions_only"], true);
    for s in suggestions {
        assert_eq!(s["placeholder_status"], "reserved", "{s}");
        assert!(s["placeholder_event_id"]
            .as_str()
            .unwrap()
            .starts_with("ev_"));
    }
    // Written to the provider: busy, without attendees.
    let placeholders = t.placeholders();
    assert_eq!(placeholders.len(), 2);
    assert!(
        placeholders.iter().all(|d| !d.contains("ATTENDEE")),
        "{placeholders:?}"
    );
    assert!(placeholders
        .iter()
        .all(|d| !d.contains("TRANSP:TRANSPARENT")));

    // The own link offers its suggestions; another link sees them busy.
    assert_eq!(t.slot_starts(&once).await, ["10:00", "14:00"]);
    let template = t.template(&device).await;
    let starts = t.slot_starts(&template).await;
    assert_eq!(starts.len(), 20 - 2, "{starts:?}");
    assert!(!starts.contains(&"10:00".to_owned()) && !starts.contains(&"14:00".to_owned()));

    // find_free_times sees them too.
    let busy =
        t.s.app
            .busy_intervals(&[], at(0, 0), at(24, 0), chrono_tz::Tz::UTC, None)
            .await
            .unwrap();
    let mut intervals = busy.intervals.clone();
    intervals.sort();
    assert_eq!(
        intervals,
        [(at(10, 0), at(10, 30)), (at(14, 0), at(14, 30))]
    );

    // Re-check right before booking.
    let id = once["id"].as_str().unwrap();
    let now = at(0, 0) - Duration::days(1);
    let app = &t.s.app;
    let _guard = app.opening_booking_guard().await;
    let slot = app
        .check_opening_slot(id, at(10, 0), at(10, 30), now)
        .await
        .unwrap();
    assert_eq!(
        slot.unwrap().suggestion_id.as_deref(),
        suggestions[0]["id"].as_str()
    );
    assert!(app
        .check_opening_slot(id, at(11, 0), at(11, 30), now)
        .await
        .unwrap()
        .is_none());
    assert!(app
        .check_opening_slot(id, at(10, 0), at(11, 0), now)
        .await
        .unwrap()
        .is_none());
    // Past by then: not bookable.
    assert!(app
        .check_opening_slot(id, at(10, 0), at(10, 30), at(10, 0))
        .await
        .unwrap()
        .is_none());

    // Booking: the link is used up, all placeholders go.
    app.complete_opening_booking(id, "ev_gebucht", now)
        .await
        .unwrap();
    assert!(t.placeholders().is_empty());
    let o = t
        .api(Method::GET, &format!("/api/openings/{id}"), &device, None)
        .await
        .json();
    assert_eq!(o["booked_event_id"], "ev_gebucht");
    assert!(o["booked_at"].is_string());
    assert_eq!(o["suggestions"], json!([]));
    assert!(t.slot_starts(&o).await.is_empty());
    let e = app
        .complete_opening_booking(id, "ev_zweiter", now)
        .await
        .unwrap_err();
    assert_eq!(e.code(), "conflict");
    assert_eq!(
        t.slot_starts(&template).await.len(),
        20,
        "times are free again"
    );

    // Templates stay bookable.
    let tid = template["id"].as_str().unwrap();
    app.complete_opening_booking(tid, "ev_vorlage", now)
        .await
        .unwrap();
    app.complete_opening_booking(tid, "ev_vorlage_2", now)
        .await
        .unwrap();
    let o = t
        .api(Method::GET, &format!("/api/openings/{tid}"), &device, None)
        .await
        .json();
    assert_eq!(o["booked_at"], Value::Null);
    assert_eq!(t.queued_removals(), 0);
}

#[tokio::test]
async fn deleting_a_link_removes_its_placeholders() {
    let (t, device) = prepared().await;
    let once = t.once_with_suggestions(&device).await;
    let id = once["id"].as_str().unwrap();
    assert_eq!(t.placeholders().len(), 2);

    // The provider is down: the link goes, the removal waits.
    t.dav.with(|s| s.delete_status = Some(503));
    let r = t
        .api(
            Method::DELETE,
            &format!("/api/openings/{id}"),
            &device,
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(t.placeholders().len(), 2);
    assert_eq!(t.queued_removals(), 2);
    let attempts: i64 = t
        .db()
        .query_row("SELECT MIN(attempts) FROM placeholder_removals", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(attempts, 1);

    // The next cleanup removes them.
    t.dav.with(|s| s.delete_status = None);
    let report = t.s.app.clean_up_openings(Utc::now()).await.unwrap();
    assert_eq!((report.removed, report.pending), (2, 0));
    assert!(t.placeholders().is_empty());
    assert_eq!(t.queued_removals(), 0);
}

#[tokio::test]
async fn past_suggestions_drop_out() {
    let (t, device) = prepared().await;
    let once = t.once_with_suggestions(&device).await;
    let id = once["id"].as_str().unwrap();

    // 12:00: the 10:00 suggestion is past.
    let report = t.s.app.clean_up_openings(at(12, 0)).await.unwrap();
    assert_eq!(report.removed, 1);
    let left = t.placeholders();
    assert_eq!(left.len(), 1);
    let o = t
        .api(Method::GET, &format!("/api/openings/{id}"), &device, None)
        .await
        .json();
    let suggestions = o["suggestions"].as_array().unwrap();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0]["start"], rfc(at(14, 0)));

    // All past: the link offers nothing, even without falling back to rules.
    t.s.app.clean_up_openings(at(15, 0)).await.unwrap();
    assert!(t.placeholders().is_empty());
    let o = t
        .api(Method::GET, &format!("/api/openings/{id}"), &device, None)
        .await
        .json();
    assert_eq!(o["suggestions"], json!([]));
    assert_eq!(o["suggestions_only"], true);
    let slots =
        t.s.app
            .opening_slots(id, at(0, 0), at(24, 0) + Duration::days(3), at(0, 0))
            .await
            .unwrap();
    assert!(slots.is_empty(), "{slots:?}");
}

#[tokio::test]
async fn placeholder_deleted_at_the_provider_is_reported_missing() {
    let (t, device) = prepared().await;
    let once = t.once_with_suggestions(&device).await;
    let id = once["id"].as_str().unwrap();
    let name = t
        .dav
        .with(|s| {
            s.calendars[CALENDAR]
                .resources
                .iter()
                .find(|(_, (_, d))| {
                    d.contains("SUMMARY:Reserviert: ")
                        && d.lines()
                            .any(|l| l.starts_with("DTSTART") && l.contains("T100000"))
                })
                .map(|(n, _)| n.clone())
        })
        .expect("placeholder at 10:00");
    t.dav.remove(&name);
    let report = t.sync().await;
    assert_eq!(report.events.deleted, 1, "{report:?}");
    let o = t
        .api(Method::GET, &format!("/api/openings/{id}"), &device, None)
        .await
        .json();
    let status: Vec<&str> = o["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["placeholder_status"].as_str().unwrap())
        .collect();
    assert_eq!(status, ["missing", "reserved"]);
    let r = t
        .api(
            Method::DELETE,
            &format!("/api/openings/{id}"),
            &device,
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert!(t.placeholders().is_empty());
    assert_eq!(t.queued_removals(), 0);
}

#[tokio::test]
async fn a_failed_placeholder_leaves_no_link() {
    let (t, device) = prepared().await;
    t.dav.with(|s| s.put_status = Some(503));
    let r = t
        .api(
            Method::POST,
            "/api/openings",
            &device,
            Some(
                json!({"kind": "once", "duration_minutes": 30, "calendar_id": t.calendar_id,
                        "suggestions": [{"start": rfc(at(10, 0))}]}),
            ),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_GATEWAY, "{}", r.text());
    assert_eq!(r.error_code(), "provider_unavailable");
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Platzhalter"));
    let r = t.api(Method::GET, "/api/openings", &device, None).await;
    assert_eq!(r.json(), json!([]));
    assert!(t.placeholders().is_empty());
}

#[tokio::test]
async fn openings_come_with_the_pull() {
    let (t, device) = prepared().await;
    let o = t.template(&device).await;
    let r = t
        .api(Method::GET, "/api/sync/pull?since=0", &device, None)
        .await;
    let changes = r.json()["changes"].as_array().unwrap().clone();
    let pulled = changes
        .iter()
        .find(|c| c["type"] == "opening")
        .expect("opening in the pull");
    assert_eq!(pulled["op"], "upsert");
    assert_eq!(pulled["data"], o);
    let cursor = r.json()["cursor"].as_i64().unwrap();
    let id = o["id"].as_str().unwrap();
    t.api(
        Method::DELETE,
        &format!("/api/openings/{id}"),
        &device,
        None,
    )
    .await;
    let r = t
        .api(
            Method::GET,
            &format!("/api/sync/pull?since={cursor}"),
            &device,
            None,
        )
        .await;
    assert_eq!(
        r.json()["changes"],
        json!([{"type": "opening", "op": "delete", "id": id}])
    );
}
