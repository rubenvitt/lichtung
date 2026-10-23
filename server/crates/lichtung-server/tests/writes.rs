//! The write path (LIC-10) against the invented, stateful CalDAV server:
//! create, update in all three scopes, delete, respond, conflicts (412,
//! stale `base_version`, gone at the provider), provider errors, rights and
//! moving between calendars. All data is made up.

mod common;

use lichtung_server::writes::{Origin, WriteOutcome};
use serde_json::{json, Value};

use common::dav::{
    invitation, setup, setup_with, single, FakeDav, Setup, CALENDAR, HOLIDAYS, USER, WORK,
};

fn app_origin() -> Origin {
    Origin::App("iPhone von Anna".into())
}

impl Setup {
    async fn mutate_as(&self, origin: &Origin, m: Value) -> (WriteOutcome, Value) {
        let outcome = self.s.app.apply_mutation(origin, &m).await.unwrap();
        let json = outcome.to_json(m["id"].as_str().unwrap_or("m"));
        (outcome, json)
    }

    async fn mutate(&self, m: Value) -> Value {
        self.mutate_as(&app_origin(), m).await.1
    }
}

fn unfold(ics: &str) -> String {
    ics.replace("\r\n ", "")
}

fn zoned(local: &str) -> Value {
    json!({"date_time": local, "time_zone": "Europe/Berlin"})
}

/// A synced setup plus the IDs of its two events.
async fn synced() -> (Setup, String, String) {
    let t = setup().await;
    t.sync().await;
    let single = t.event_by_uid("zahnarzt@example.org").await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let series = t.event_by_uid("team-sync@example.org").await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (t, single, series)
}

#[tokio::test]
async fn create_writes_a_new_object_and_stores_it() {
    let t = setup().await;
    t.sync().await;
    let r = t
        .mutate(json!({
            "id": "m-create", "op": "create_event", "calendar_id": t.calendar_id,
            "event": {
                "title": "Lauftreff", "location": "Stadtpark",
                "start": zoned("2026-11-05T18:00:00"), "end": zoned("2026-11-05T19:00:00"),
                "alarms": [{"minutes_before": 30}],
                "uid": "lauftreff@example.org"
            }
        }))
        .await;
    assert_eq!(r["status"], "applied", "{r}");
    let ev = &r["events"][0];
    assert_eq!(ev["title"], "Lauftreff");
    assert_eq!(ev["version"], 1);
    assert_eq!(ev["uid"], "lauftreff@example.org");
    assert_eq!(ev["alarms"], json!([{"minutes_before": 30}]));
    assert_eq!(r["deleted_event_ids"], json!([]));

    let (name, etag, data) = t.dav.find_uid(CALENDAR, "lauftreff@example.org").unwrap();
    assert!(name.ends_with(".ics"), "{name}");
    let data = unfold(&data);
    assert!(data.contains("SEQUENCE:0"));
    assert!(data.contains("PRODID:-//Lichtung//Lichtung-Server//DE"));
    assert!(data.contains("DTSTAMP:"));
    let stored_etag: Option<String> = t
        .db()
        .query_row(
            "SELECT etag FROM events WHERE id = ?1",
            [ev["id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored_etag.as_deref(), Some(etag.as_str()));

    // The next sync sees nothing new.
    let report = t.sync().await;
    assert_eq!(
        (report.events.added, report.events.updated),
        (0, 0),
        "{report:?}"
    );

    // Same UID again: refused before anything is written.
    let puts = t.dav.put_count();
    let r = t
        .mutate(json!({
            "id": "m-create-2", "op": "create_event", "calendar_id": t.calendar_id,
            "event": {"title": "Doppelt", "uid": "lauftreff@example.org",
                      "start": {"date": "2026-11-06"}, "end": {"date": "2026-11-07"}}
        }))
        .await;
    assert_eq!(r["status"], "rejected");
    assert_eq!(r["error"]["code"], "invalid_event");
    assert_eq!(t.dav.put_count(), puts);
}

#[tokio::test]
async fn create_refetches_when_the_server_changes_the_data() {
    let t = setup_with(|dav| dav.with(|s| s.etag_in_put_response = false)).await;
    let r = t
        .mutate(json!({
            "id": "m1", "op": "create_event", "calendar_id": t.calendar_id,
            "event": {"title": "Ohne ETag", "start": {"date": "2026-11-06"}, "end": {"date": "2026-11-07"}}
        }))
        .await;
    assert_eq!(r["status"], "applied", "{r}");
    let id = r["events"][0]["id"].as_str().unwrap();
    let stored = t.stored_ics(id);
    assert!(stored.contains("X-BEISPIEL-GESEHEN:1"), "{stored}");
    assert!(r["events"][0]["uid"]
        .as_str()
        .unwrap()
        .ends_with("@lichtung"));
    // Stored as the provider has it: the next sync changes nothing.
    let report = t.sync().await;
    assert_eq!(report.events.updated, 0, "{report:?}");
}

#[tokio::test]
async fn validation_refuses_bad_events() {
    let t = setup().await;
    let cases = [
        (
            json!({"title": "Rückwärts", "start": zoned("2026-11-05T18:00:00"),
                   "end": zoned("2026-11-05T17:00:00")}),
            "invalid_event",
        ),
        (
            json!({"title": "Gemischt", "start": {"date": "2026-11-05"},
                   "end": zoned("2026-11-05T17:00:00")}),
            "invalid_event",
        ),
        (
            json!({"start": zoned("2026-11-05T18:00:00"), "end": zoned("2026-11-05T19:00:00")}),
            "invalid_event",
        ),
        (
            json!({"title": "Ohne Ende", "start": {"date": "2026-11-05"}}),
            "invalid_event",
        ),
        (
            json!({"title": "Tippfehler", "begin": {"date": "2026-11-05"}}),
            "invalid_request",
        ),
    ];
    let puts = t.dav.put_count();
    for (event, code) in cases {
        let r = t
            .mutate(
                json!({"id": "m", "op": "create_event", "calendar_id": t.calendar_id,
                           "event": event}),
            )
            .await;
        assert_eq!(r["status"], "rejected", "{event}: {r}");
        assert_eq!(r["error"]["code"], code, "{event}: {r}");
    }
    let r = t.mutate(json!({"id": "m", "op": "explode"})).await;
    assert_eq!(r["error"]["code"], "invalid_request");
    assert_eq!(t.dav.put_count(), puts, "nothing written");
}

#[tokio::test]
async fn update_all_keeps_unknown_properties() {
    let t = setup_with(|dav| {
        dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"));
        dav.put("sitzung.ics", &invitation("sitzung@example.org"));
    })
    .await;
    t.sync().await;
    let ev = t.event_by_uid("zahnarzt@example.org").await;
    let r = t
        .mutate(json!({
            "id": "m-up", "op": "update_event", "event_id": ev["id"], "base_version": 1,
            "changes": {"title": "Zahnarzt (Kontrolle)", "start": zoned("2026-11-03T15:00:00"),
                        "location": null, "notes": "Bonusheft"}
        }))
        .await;
    assert_eq!(r["status"], "applied", "{r}");
    let new = &r["events"][0];
    assert_eq!(new["version"], 2);
    assert_eq!(new["title"], "Zahnarzt (Kontrolle)");
    assert_eq!(new["location"], Value::Null);
    assert_eq!(new["notes"], "Bonusheft");
    // The end moves with the start (one hour).
    assert_eq!(new["end"]["date_time"], "2026-11-03T16:00:00");
    let (_, _, data) = t.dav.find_uid(CALENDAR, "zahnarzt@example.org").unwrap();
    assert!(
        data.contains("DTSTART;TZID=Europe/Berlin:20261103T150000"),
        "{data}"
    );
    assert!(!data.contains("LOCATION"));

    // Unknown properties of the original survive an edit.
    let sitzung = t.event_by_uid("sitzung@example.org").await;
    let r = t
        .mutate(
            json!({"id": "m-up2", "op": "update_event", "event_id": sitzung["id"],
                       "base_version": 1, "changes": {"notes": "Protokoll"}}),
        )
        .await;
    assert_eq!(r["status"], "applied", "{r}");
    let (_, _, data) = t.dav.find_uid(CALENDAR, "sitzung@example.org").unwrap();
    assert!(data.contains("X-BEISPIEL-UNBEKANNT:bleibt erhalten"));
}

#[tokio::test]
async fn update_this_occurrence_creates_an_override() {
    let (t, _, series) = synced().await;
    let r = t
        .mutate(json!({
            "id": "m-this", "op": "update_event", "event_id": series, "base_version": 1,
            "scope": "this", "occurrence": zoned("2026-11-23T09:00:00"),
            "changes": {"start": zoned("2026-11-23T11:00:00"), "end": zoned("2026-11-23T11:30:00"),
                        "title": "Team-Sync (spät)"}
        }))
        .await;
    assert_eq!(r["status"], "applied", "{r}");
    let ev = &r["events"][0];
    assert_eq!(ev["title"], "Team-Sync", "series head unchanged");
    let overrides = ev["overrides"].as_array().unwrap();
    let moved = overrides
        .iter()
        .find(|o| o["recurrence_id"]["date_time"] == "2026-11-23T09:00:00")
        .unwrap_or_else(|| panic!("{ev}"));
    assert_eq!(moved["title"], "Team-Sync (spät)");
    assert_eq!(moved["start"]["date_time"], "2026-11-23T11:00:00");
    let (_, _, data) = t.dav.find_uid(CALENDAR, "team-sync@example.org").unwrap();
    assert!(data.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261123T090000"));

    // An occurrence that does not exist.
    let r = t
        .mutate(json!({
            "id": "m-this-2", "op": "update_event", "event_id": series, "base_version": 2,
            "scope": "this", "occurrence": zoned("2026-11-24T09:00:00"),
            "changes": {"title": "Nie"}
        }))
        .await;
    assert_eq!(r["error"]["code"], "occurrence_not_found", "{r}");
    // Recurrence changes need scope all or this_and_following.
    let r = t
        .mutate(json!({
            "id": "m-this-3", "op": "update_event", "event_id": series, "base_version": 2,
            "scope": "this", "occurrence": zoned("2026-11-02T09:00:00"),
            "changes": {"recurrence": null}
        }))
        .await;
    assert_eq!(r["error"]["code"], "scope_not_supported", "{r}");
}

#[tokio::test]
async fn update_this_and_following_splits_the_series() {
    let (t, _, series) = synced().await;
    let r = t
        .mutate(json!({
            "id": "m-split", "op": "update_event", "event_id": series, "base_version": 1,
            "scope": "this_and_following", "occurrence": zoned("2026-11-23T09:00:00"),
            "changes": {"title": "Team-Sync neu", "alarms": [{"minutes_before": 5}]}
        }))
        .await;
    assert_eq!(r["status"], "applied", "{r}");
    let events = r["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "{r}");
    assert_eq!(events[0]["id"], series);
    assert_eq!(events[0]["title"], "Team-Sync");
    assert_eq!(events[0]["alarms"], json!([]));
    assert_eq!(events[1]["title"], "Team-Sync neu");
    assert_eq!(events[1]["start"]["date_time"], "2026-11-23T09:00:00");
    assert_eq!(events[1]["alarms"], json!([{"minutes_before": 5}]));
    assert_ne!(events[1]["uid"], events[0]["uid"]);
    assert_eq!(t.dav.names_in(CALENDAR).len(), 3, "a second object");
    let (_, _, old) = t.dav.find_uid(CALENDAR, "team-sync@example.org").unwrap();
    assert!(!old.contains("COUNT=4"), "the old series ends: {old}");
}

#[tokio::test]
async fn delete_whole_event_and_single_occurrence() {
    let (t, single, series) = synced().await;
    let r = t
        .mutate(json!({"id": "m-del", "op": "delete_event", "event_id": single, "base_version": 1}))
        .await;
    assert_eq!(r["status"], "applied", "{r}");
    assert_eq!(r["deleted_event_ids"], json!([single]));
    assert!(t.dav.find_uid(CALENDAR, "zahnarzt@example.org").is_none());
    assert!(t.event_tombstones().contains(&single));

    let r = t
        .mutate(
            json!({"id": "m-del-2", "op": "delete_event", "event_id": series,
                       "base_version": 1, "scope": "this",
                       "occurrence": zoned("2026-11-23T09:00:00")}),
        )
        .await;
    assert_eq!(r["status"], "applied", "{r}");
    let exdates = r["events"][0]["recurrence"]["exdates"].as_array().unwrap();
    assert!(exdates
        .iter()
        .any(|d| d["date_time"] == "2026-11-23T09:00:00"));
    // Deleting the already deleted event again: never known as such → it
    // was deleted, so the change is reported as a conflict.
    let r = t
        .mutate(
            json!({"id": "m-del-3", "op": "update_event", "event_id": single,
                       "base_version": 1, "changes": {"title": "zu spät"}}),
        )
        .await;
    assert_eq!(r["status"], "conflict", "{r}");
    assert_eq!(r["conflict"]["event_id"], Value::Null);
    assert_eq!(r["events"], json!([]));
    let r = t
        .mutate(
            json!({"id": "m-del-4", "op": "delete_event", "event_id": "ev_unbekannt",
                       "base_version": 1}),
        )
        .await;
    assert_eq!(r["error"]["code"], "event_not_found");
}

#[tokio::test]
async fn respond_sets_the_own_partstat() {
    let t = setup_with(|dav| dav.put("sitzung.ics", &invitation("sitzung@example.org"))).await;
    t.sync().await;
    let ev = t.event_by_uid("sitzung@example.org").await;
    assert_eq!(ev["self_status"], "needs_action");
    let r = t
        .mutate(
            json!({"id": "m-rsvp", "op": "respond", "event_id": ev["id"],
                       "status": "accepted", "comment": null}),
        )
        .await;
    assert_eq!(r["status"], "applied", "{r}");
    assert_eq!(r["events"][0]["self_status"], "accepted");
    let (_, _, data) = t.dav.find_uid(CALENDAR, "sitzung@example.org").unwrap();
    let data = unfold(&data);
    assert!(
        data.contains(&format!("ATTENDEE;PARTSTAT=ACCEPTED:mailto:{USER}")),
        "{data}"
    );
    assert!(data.contains("SEQUENCE:2"), "an attendee keeps SEQUENCE");
    assert!(data.contains("X-BEISPIEL-UNBEKANNT:bleibt erhalten"));

    // One occurrence only.
    let r = t
        .mutate(
            json!({"id": "m-rsvp-2", "op": "respond", "event_id": ev["id"],
                       "occurrence": zoned("2026-11-11T10:00:00"), "status": "declined",
                       "comment": "Bin verreist"}),
        )
        .await;
    assert_eq!(r["status"], "applied", "{r}");
    let ov = &r["events"][0]["overrides"][0];
    assert_eq!(ov["recurrence_id"]["date_time"], "2026-11-11T10:00:00");
    assert_eq!(ov["self_status"], "declined");
    assert_eq!(r["events"][0]["self_status"], "accepted");
    let (_, _, data) = t.dav.find_uid(CALENDAR, "sitzung@example.org").unwrap();
    assert!(data.contains("COMMENT:Bin verreist"));

    // Not invited: refused.
    let t2 = setup().await;
    t2.sync().await;
    let ev = t2.event_by_uid("zahnarzt@example.org").await;
    let r = t2
        .mutate(
            json!({"id": "m-rsvp-3", "op": "respond", "event_id": ev["id"], "status": "tentative"}),
        )
        .await;
    assert_eq!(r["error"]["code"], "invalid_event", "{r}");
}

#[tokio::test]
async fn precondition_failed_is_a_conflict_and_the_provider_wins() {
    let (t, ev_id, _) = synced().await;
    // Changed at the provider behind our back.
    t.dav.put(
        "zahnarzt.ics",
        &single("zahnarzt@example.org", "Zahnarzt (vom Anbieter)"),
    );
    let r = t
        .mutate(
            json!({"id": "m-412", "op": "update_event", "event_id": ev_id, "base_version": 1,
                       "changes": {"start": zoned("2026-11-03T10:00:00"),
                                   "end": zoned("2026-11-03T11:00:00")}}),
        )
        .await;
    assert_eq!(r["status"], "conflict", "{r}");
    let conflict = &r["conflict"];
    assert!(conflict["id"].as_str().unwrap().starts_with("cf_"));
    assert_eq!(conflict["event_id"], ev_id);
    assert_eq!(conflict["calendar_id"], t.calendar_id);
    assert_eq!(conflict["origin"], "app:iPhone von Anna");
    let message = conflict["message"].as_str().unwrap();
    assert!(
        message.contains("„Zahnarzt“ wurde inzwischen auf dem CalDAV-Server geändert")
            && message.contains("Beginn 3.11.2026, 10:00")
            && message.contains("nicht übernommen"),
        "{message}"
    );
    assert_eq!(conflict["mutation"]["id"], "m-412");
    assert_eq!(
        conflict["mutation"]["changes"]["start"]["date_time"],
        "2026-11-03T10:00:00"
    );
    // The provider's version is stored at once (new version).
    assert_eq!(r["events"][0]["title"], "Zahnarzt (vom Anbieter)");
    assert_eq!(r["events"][0]["version"], 2);
    let (_, _, data) = t.dav.find_uid(CALENDAR, "zahnarzt@example.org").unwrap();
    assert!(data.contains("T140000"), "provider data untouched");

    let listed = t.s.app.list_conflicts().await.unwrap();
    assert_eq!(listed.len(), 1);
    let id = listed[0].id.clone();
    t.s.app.dismiss_conflict(&id).await.unwrap();
    assert!(t.s.app.list_conflicts().await.unwrap().is_empty());
    assert_eq!(t.tombstones_of("conflict"), std::slice::from_ref(&id));
    assert_eq!(
        t.s.app.dismiss_conflict(&id).await.unwrap_err().code(),
        "not_found"
    );
}

#[tokio::test]
async fn stale_base_version_is_a_conflict_without_writing() {
    let (t, ev_id, _) = synced().await;
    t.dav.put(
        "zahnarzt.ics",
        &single("zahnarzt@example.org", "Zahnarzt (neu)"),
    );
    t.sync().await; // version 2
    let puts = t.dav.put_count();
    let r = t
        .mutate(json!({"id": "m-old", "op": "delete_event", "event_id": ev_id, "base_version": 1}))
        .await;
    assert_eq!(r["status"], "conflict", "{r}");
    assert!(r["conflict"]["message"]
        .as_str()
        .unwrap()
        .contains("Deine Löschung wurde nicht übernommen"));
    assert_eq!(r["events"][0]["version"], 2);
    assert_eq!(t.dav.put_count(), puts);
    assert!(t.dav.find_uid(CALENDAR, "zahnarzt@example.org").is_some());
}

#[tokio::test]
async fn gone_at_the_provider_is_a_conflict() {
    let (t, ev_id, _) = synced().await;
    t.dav.remove("zahnarzt.ics");
    let r = t
        .mutate(
            json!({"id": "m-404", "op": "update_event", "event_id": ev_id, "base_version": 1,
                       "changes": {"title": "Zahnarzt?"}}),
        )
        .await;
    assert_eq!(r["status"], "conflict", "{r}");
    assert_eq!(r["conflict"]["event_id"], Value::Null);
    assert!(r["conflict"]["message"]
        .as_str()
        .unwrap()
        .contains("gelöscht"));
    assert_eq!(r["events"], json!([]));
    assert!(t.event_tombstones().contains(&ev_id));
}

#[tokio::test]
async fn deleting_the_event_detaches_its_conflicts() {
    let (t, ev_id, _) = synced().await;
    t.dav.put(
        "zahnarzt.ics",
        &single("zahnarzt@example.org", "Zahnarzt (geändert)"),
    );
    let r = t
        .mutate(
            json!({"id": "m-a", "op": "update_event", "event_id": ev_id, "base_version": 1,
                       "changes": {"title": "x"}}),
        )
        .await;
    assert_eq!(r["status"], "conflict");
    t.dav.remove("zahnarzt.ics");
    t.sync().await;
    let listed = t.s.app.list_conflicts().await.unwrap();
    assert_eq!(listed[0].event_id, None);
}

#[tokio::test]
async fn provider_errors_become_retry_or_rejected() {
    let (t, ev_id, _) = synced().await;
    let update = |id: &str| {
        json!({"id": id, "op": "update_event", "event_id": ev_id, "base_version": 1,
               "changes": {"title": "Neu"}})
    };
    t.dav.with(|s| s.put_status = Some(503));
    let r = t.mutate(update("m-503")).await;
    assert_eq!(r["status"], "retry", "{r}");
    assert_eq!(r["error"]["code"], "provider_unavailable");
    assert_eq!(r["retry_after_seconds"], 60);

    t.dav.with(|s| s.put_status = Some(403));
    let r = t.mutate(update("m-403")).await;
    assert_eq!(r["status"], "rejected", "{r}");
    assert_eq!(r["error"]["code"], "provider_rejected");
    assert!(r["error"]["message"]
        .as_str()
        .unwrap()
        .contains("abgelehnt"));

    t.dav.with(|s| s.put_status = Some(429));
    let r = t.mutate(update("m-429")).await;
    assert_eq!(r["status"], "retry", "{r}");

    t.dav.with(|s| {
        s.put_status = None;
        s.unauthorized = true;
    });
    let r = t.mutate(update("m-401")).await;
    assert_eq!(r["status"], "retry", "{r}");
    let (state, _) = t.status().await;
    assert_eq!(state, "auth_failed");
    // Nothing changed locally.
    assert_eq!(t.event_by_uid("zahnarzt@example.org").await["version"], 1);
}

#[tokio::test]
async fn read_only_calendars_and_the_mcp_guard() {
    let t = setup_with(|dav| {
        dav.add_calendar(HOLIDAYS, "Feiertage", false);
        dav.put_in(
            HOLIDAYS,
            "neujahr.ics",
            &single("neujahr@example.org", "Neujahr"),
        );
        dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"));
    })
    .await;
    t.sync().await;
    let holidays = t.calendar_named("Feiertage").await;
    let r = t
        .mutate(
            json!({"id": "m-ro", "op": "create_event", "calendar_id": holidays,
                       "event": {"title": "Brückentag", "start": {"date": "2026-12-28"},
                                 "end": {"date": "2026-12-29"}}}),
        )
        .await;
    assert_eq!(r["status"], "rejected");
    assert_eq!(r["error"]["code"], "calendar_read_only");
    assert_eq!(
        r["error"]["message"],
        "Der Kalender „Feiertage“ ist nur lesbar."
    );

    // mcp_write only guards MCP.
    t.s.app
        .update_calendar(
            &t.calendar_id,
            serde_json::from_value(json!({"mcp_write": false})).unwrap(),
        )
        .await
        .unwrap();
    let ev = t.event_by_uid("zahnarzt@example.org").await;
    let change = |id: &str| {
        json!({"id": id, "op": "update_event", "event_id": ev["id"], "base_version": 1,
               "changes": {"title": "Von Claude"}})
    };
    let (_, r) = t
        .mutate_as(&Origin::Mcp("claude.ai".into()), change("m-mcp"))
        .await;
    assert_eq!(r["error"]["code"], "calendar_read_only", "{r}");
    assert!(r["error"]["message"]
        .as_str()
        .unwrap()
        .contains("mcp_write"));
    let (_, r) = t.mutate_as(&Origin::Web, change("m-web")).await;
    assert_eq!(r["status"], "applied", "{r}");
}

async fn with_work_calendar(prepare: impl FnOnce(&FakeDav)) -> (Setup, String, String) {
    let t = setup_with(|dav| {
        dav.add_calendar(WORK, "Arbeit", true);
        dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"));
        prepare(dav);
    })
    .await;
    t.sync().await;
    let work = t.calendar_named("Arbeit").await;
    let ev = t.event_by_uid("zahnarzt@example.org").await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (t, work, ev)
}

#[tokio::test]
async fn moving_to_another_calendar() {
    for uid_conflict in [false, true] {
        let (t, work, ev) =
            with_work_calendar(|dav| dav.with(|s| s.uid_conflict_across = uid_conflict)).await;
        let r = t
            .mutate(
                json!({"id": "m-move", "op": "update_event", "event_id": ev, "base_version": 1,
                           "changes": {"calendar_id": work, "title": "Zahnarzt (Arbeit)"}}),
            )
            .await;
        assert_eq!(r["status"], "applied", "{uid_conflict}: {r}");
        assert_eq!(r["deleted_event_ids"], json!([ev]));
        assert_eq!(r["events"][0]["calendar_id"], work);
        assert_eq!(r["events"][0]["title"], "Zahnarzt (Arbeit)");
        assert!(t.dav.find_uid(WORK, "zahnarzt@example.org").is_some());
        assert!(t.dav.find_uid(CALENDAR, "zahnarzt@example.org").is_none());
        assert!(t.event_tombstones().contains(&ev));
    }

    // Moving needs scope all.
    let (t, work, ev) = with_work_calendar(|_| {}).await;
    let r = t
        .mutate(
            json!({"id": "m-move-2", "op": "update_event", "event_id": ev, "base_version": 1,
                       "scope": "this", "occurrence": zoned("2026-11-03T14:00:00"),
                       "changes": {"calendar_id": work}}),
        )
        .await;
    assert_eq!(r["error"]["code"], "scope_not_supported", "{r}");
}

#[tokio::test]
async fn moving_reports_a_failed_deletion_as_conflict() {
    let (t, work, ev) = with_work_calendar(|dav| dav.with(|s| s.delete_status = Some(503))).await;
    let r = t
        .mutate(
            json!({"id": "m-move", "op": "update_event", "event_id": ev, "base_version": 1,
                       "changes": {"calendar_id": work}}),
        )
        .await;
    assert_eq!(r["status"], "conflict", "{r}");
    assert!(r["conflict"]["message"]
        .as_str()
        .unwrap()
        .contains("im Kalender „Privat“ aber nicht gelöscht"));
    let events = r["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "both copies: {r}");
    assert!(t.dav.find_uid(WORK, "zahnarzt@example.org").is_some());
    assert!(t.dav.find_uid(CALENDAR, "zahnarzt@example.org").is_some());
}
