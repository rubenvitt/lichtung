//! Scoped edits and deletes, always checked through a full roundtrip
//! (parse → edit → serialize → parse → expand). All fixtures are invented.

mod common;

use chrono::NaiveDate;
use common::{berlin, cal, fmt, utc};
use lichtung_ical::{
    delete_event, edit_event, expand, parse_recurrence_key, CalTime, CalendarObject, EditContext,
    EditOutcome, EditScope, Error, EventChanges, RecurRule,
};

/// Weekly Monday 09:00–09:30 Europe/Berlin, endless, with one moved
/// override (19.10. → 10:00), one unmoved override (26.10., other location)
/// and one EXDATE (02.11.).
const SERIES: &str = "BEGIN:VEVENT
    UID:serie@example.org
    DTSTAMP:20261001T080000Z
    SEQUENCE:1
    DTSTART;TZID=Europe/Berlin:20261005T090000
    DTEND;TZID=Europe/Berlin:20261005T093000
    RRULE:FREQ=WEEKLY
    EXDATE;TZID=Europe/Berlin:20261102T090000
    SUMMARY:Team-Sync
    LOCATION:Raum 1
    X-LICHTUNG-TEST:bleibt
    END:VEVENT
    BEGIN:VEVENT
    UID:serie@example.org
    DTSTAMP:20261001T080000Z
    RECURRENCE-ID;TZID=Europe/Berlin:20261019T090000
    DTSTART;TZID=Europe/Berlin:20261019T100000
    DTEND;TZID=Europe/Berlin:20261019T103000
    SUMMARY:Team-Sync (verlegt)
    LOCATION:Raum 1
    END:VEVENT
    BEGIN:VEVENT
    UID:serie@example.org
    DTSTAMP:20261001T080000Z
    RECURRENCE-ID;TZID=Europe/Berlin:20261026T090000
    DTSTART;TZID=Europe/Berlin:20261026T090000
    DTEND;TZID=Europe/Berlin:20261026T093000
    SUMMARY:Team-Sync
    LOCATION:Raum 2
    END:VEVENT";

fn ctx() -> EditContext {
    EditContext::new(utc("2026-10-23 12:00"), chrono_tz::Europe::Berlin)
}

fn berlin_at(day: &str, hm: (u32, u32)) -> CalTime {
    CalTime::zoned(
        NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(hm.0, hm.1, 0)
            .unwrap(),
        "Europe/Berlin",
    )
}

/// Serializes and re-parses an object (the roundtrip every edit must survive).
fn roundtrip(obj: &CalendarObject) -> CalendarObject {
    let text = obj.to_ics();
    let again = CalendarObject::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(
        again.to_ics(),
        text,
        "serialize → parse → serialize ist stabil"
    );
    again
}

/// Occurrences October/November 2026 with summary.
fn occ(obj: &CalendarObject) -> Vec<String> {
    let res = expand(
        obj,
        utc("2026-10-01 00:00"),
        utc("2026-11-15 00:00"),
        &berlin(),
    )
    .unwrap();
    res.occurrences
        .iter()
        .map(|o| {
            let summary = obj
                .event_at(o.component_index)
                .unwrap()
                .summary()
                .unwrap_or_default();
            format!("{} | {summary}", fmt(o))
        })
        .collect()
}

#[test]
fn utc_input_is_converted_into_series_zone() {
    let obj = base();
    let rid = parse_recurrence_key("20261012T070000Z").unwrap();
    let start = parse_recurrence_key("20261012T073000Z").unwrap();
    let out = edit_event(
        &obj,
        Some(&rid),
        EditScope::All,
        &EventChanges {
            start: Some(start),
            ..Default::default()
        },
        &ctx(),
    )
    .unwrap();
    let new = updated(&out);
    assert_eq!(
        new.master().unwrap().start().unwrap(),
        berlin_at("2026-10-05", (9, 30))
    );
    // Still 09:30 local after the switch to winter time.
    assert!(occ(&new)
        .contains(&"2026-11-09 09:30 Europe/Berlin [08:30Z] 30min | Team-Sync".to_string()));
}

fn base() -> CalendarObject {
    CalendarObject::parse(&cal(SERIES)).unwrap()
}

fn updated(out: &EditOutcome) -> CalendarObject {
    roundtrip(out.updated.as_ref().expect("updated object"))
}

#[test]
fn base_series_expands_as_expected() {
    assert_eq!(
        occ(&base()),
        [
            "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min | Team-Sync",
            "2026-10-12 09:00 Europe/Berlin [07:00Z] 30min | Team-Sync",
            "2026-10-19 10:00 Europe/Berlin [08:00Z] 30min override | Team-Sync (verlegt)",
            "2026-10-26 09:00 Europe/Berlin [08:00Z] 30min override | Team-Sync",
            "2026-11-09 09:00 Europe/Berlin [08:00Z] 30min | Team-Sync",
        ]
    );
}

#[test]
fn only_this_creates_override() {
    let obj = base();
    let changes = EventChanges {
        summary: Some(Some("Team-Sync mit Gast".into())),
        start: Some(berlin_at("2026-10-12", (11, 0))),
        end: Some(berlin_at("2026-10-12", (12, 0))),
        ..Default::default()
    };
    let out = edit_event(
        &obj,
        Some(&berlin_at("2026-10-12", (9, 0))),
        EditScope::This,
        &changes,
        &ctx(),
    )
    .unwrap();
    assert!(out.created.is_none());
    let new = updated(&out);
    assert_eq!(
        occ(&new),
        [
            "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min | Team-Sync",
            "2026-10-12 11:00 Europe/Berlin [09:00Z] 60min override | Team-Sync mit Gast",
            "2026-10-19 10:00 Europe/Berlin [08:00Z] 30min override | Team-Sync (verlegt)",
            "2026-10-26 09:00 Europe/Berlin [08:00Z] 30min override | Team-Sync",
            "2026-11-09 09:00 Europe/Berlin [08:00Z] 30min | Team-Sync",
        ]
    );
    // The master is untouched.
    assert_eq!(
        new.master().unwrap().component(),
        obj.master().unwrap().component()
    );
    let ov = new
        .overrides()
        .into_iter()
        .find(|o| o.summary().as_deref() == Some("Team-Sync mit Gast"))
        .unwrap();
    assert_eq!(ov.sequence(), 2);
    assert!(ov.rrule().unwrap().is_none());
    assert!(ov.exdates().unwrap().is_empty());
    assert_eq!(
        ov.recurrence_id().unwrap().unwrap().value,
        berlin_at("2026-10-12", (9, 0))
    );
    assert_eq!(ov.location().as_deref(), Some("Raum 1"));
    assert_eq!(
        ov.component().property("X-LICHTUNG-TEST").unwrap().value(),
        "bleibt"
    );
    assert_eq!(
        ov.component().property("DTSTAMP").unwrap().value(),
        "20261023T120000Z"
    );
}

#[test]
fn only_this_updates_existing_override_via_key() {
    let obj = base();
    let key = parse_recurrence_key("20261019T070000Z").unwrap();
    let changes = EventChanges {
        location: Some(Some("Raum 3".into())),
        ..Default::default()
    };
    let out = edit_event(&obj, Some(&key), EditScope::This, &changes, &ctx()).unwrap();
    let new = updated(&out);
    assert_eq!(new.overrides().len(), 2);
    let ov = new
        .overrides()
        .into_iter()
        .find(|o| o.location().as_deref() == Some("Raum 3"))
        .unwrap();
    assert_eq!(ov.summary().as_deref(), Some("Team-Sync (verlegt)"));
    assert_eq!(ov.sequence(), 1);
    // Time unchanged.
    assert_eq!(ov.start().unwrap(), berlin_at("2026-10-19", (10, 0)));
}

#[test]
fn all_moves_series_and_keeps_overrides_consistent() {
    let obj = base();
    let changes = EventChanges {
        summary: Some(Some("Jour fixe".into())),
        start: Some(berlin_at("2026-10-12", (9, 30))),
        ..Default::default()
    };
    let out = edit_event(
        &obj,
        Some(&berlin_at("2026-10-12", (9, 0))),
        EditScope::All,
        &changes,
        &ctx(),
    )
    .unwrap();
    let new = updated(&out);
    assert_eq!(
        occ(&new),
        [
            "2026-10-05 09:30 Europe/Berlin [07:30Z] 30min | Jour fixe",
            "2026-10-12 09:30 Europe/Berlin [07:30Z] 30min | Jour fixe",
            "2026-10-19 10:00 Europe/Berlin [08:00Z] 30min override | Team-Sync (verlegt)",
            "2026-10-26 09:30 Europe/Berlin [08:30Z] 30min override | Jour fixe",
            "2026-11-09 09:30 Europe/Berlin [08:30Z] 30min | Jour fixe",
        ]
    );
    let m = new.master().unwrap();
    assert_eq!(m.sequence(), 2);
    assert_eq!(m.exdates().unwrap(), [berlin_at("2026-11-02", (9, 30))]);
    let rids: Vec<CalTime> = new
        .overrides()
        .iter()
        .map(|o| o.recurrence_id().unwrap().unwrap().value)
        .collect();
    assert_eq!(
        rids,
        [
            berlin_at("2026-10-19", (9, 30)),
            berlin_at("2026-10-26", (9, 30))
        ]
    );
    // Location of the second override was its own and stays.
    assert_eq!(new.overrides()[1].location().as_deref(), Some("Raum 2"));
}

#[test]
fn all_can_switch_to_all_day() {
    let obj = base();
    let changes = EventChanges {
        start: Some(CalTime::Date(common::date("2026-10-05"))),
        ..Default::default()
    };
    let out = edit_event(&obj, None, EditScope::All, &changes, &ctx()).unwrap();
    let new = updated(&out);
    assert_eq!(
        occ(&new),
        [
            "2026-10-05..2026-10-06 ganztägig | Team-Sync",
            "2026-10-12..2026-10-13 ganztägig | Team-Sync",
            "2026-10-19 10:00 Europe/Berlin [08:00Z] 30min override | Team-Sync (verlegt)",
            "2026-10-26..2026-10-27 ganztägig override | Team-Sync",
            "2026-11-09..2026-11-10 ganztägig | Team-Sync",
        ]
    );
    assert_eq!(
        new.master().unwrap().exdates().unwrap(),
        [CalTime::Date(common::date("2026-11-02"))]
    );
}

#[test]
fn this_and_future_splits_series() {
    let obj = base();
    let changes = EventChanges {
        summary: Some(Some("Neues Format".into())),
        ..Default::default()
    };
    let out = edit_event(
        &obj,
        Some(&berlin_at("2026-10-19", (9, 0))),
        EditScope::ThisAndFuture,
        &changes,
        &ctx().with_new_uid("serie-neu@example.org"),
    )
    .unwrap();
    let old = updated(&out);
    let new = roundtrip(out.created.as_ref().unwrap());

    assert_eq!(
        old.master().unwrap().rrule().unwrap().unwrap().to_string(),
        "FREQ=WEEKLY;UNTIL=20261019T065959Z"
    );
    assert!(old.overrides().is_empty());
    assert!(old.master().unwrap().exdates().unwrap().is_empty());
    assert_eq!(
        occ(&old),
        [
            "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min | Team-Sync",
            "2026-10-12 09:00 Europe/Berlin [07:00Z] 30min | Team-Sync",
        ]
    );

    assert_eq!(new.uid().as_deref(), Some("serie-neu@example.org"));
    assert!(new
        .events()
        .all(|e| e.uid().as_deref() == Some("serie-neu@example.org")));
    let m = new.master().unwrap();
    assert_eq!(m.start().unwrap(), berlin_at("2026-10-19", (9, 0)));
    assert_eq!(m.rrule().unwrap().unwrap().to_string(), "FREQ=WEEKLY");
    assert_eq!(m.sequence(), 0);
    assert_eq!(
        occ(&new),
        [
            "2026-10-19 10:00 Europe/Berlin [08:00Z] 30min override | Team-Sync (verlegt)",
            "2026-10-26 09:00 Europe/Berlin [08:00Z] 30min override | Neues Format",
            "2026-11-09 09:00 Europe/Berlin [08:00Z] 30min | Neues Format",
        ]
    );
}

#[test]
fn this_and_future_without_changes_preserves_all_times() {
    let obj = base();
    let out = edit_event(
        &obj,
        Some(&berlin_at("2026-10-12", (9, 0))),
        EditScope::ThisAndFuture,
        &EventChanges::default(),
        &ctx().with_new_uid("serie-neu@example.org"),
    )
    .unwrap();
    let old = updated(&out);
    let new = roundtrip(out.created.as_ref().unwrap());
    let strip = |v: Vec<String>| -> Vec<String> {
        v.into_iter()
            .map(|s| s.split(" | ").next().unwrap().to_string())
            .collect()
    };
    let mut union = strip(occ(&old));
    union.extend(strip(occ(&new)));
    assert_eq!(union, strip(occ(&obj)));
}

#[test]
fn this_and_future_recomputes_count_and_moves_time() {
    let obj = CalendarObject::parse(&cal("BEGIN:VEVENT
         UID:count@example.org
         DTSTAMP:20261001T080000Z
         DTSTART;TZID=Europe/Berlin:20261005T090000
         DTEND;TZID=Europe/Berlin:20261005T093000
         RRULE:FREQ=WEEKLY;COUNT=6
         SUMMARY:Kurs
         END:VEVENT"))
    .unwrap();
    let changes = EventChanges {
        start: Some(berlin_at("2026-10-26", (10, 0))),
        ..Default::default()
    };
    let out = edit_event(
        &obj,
        Some(&berlin_at("2026-10-26", (9, 0))),
        EditScope::ThisAndFuture,
        &changes,
        &ctx().with_new_uid("count-neu@example.org"),
    )
    .unwrap();
    let old = updated(&out);
    let new = roundtrip(out.created.as_ref().unwrap());
    assert_eq!(
        old.master().unwrap().rrule().unwrap().unwrap().to_string(),
        "FREQ=WEEKLY;COUNT=3"
    );
    assert_eq!(
        new.master().unwrap().rrule().unwrap().unwrap().to_string(),
        "FREQ=WEEKLY;COUNT=3"
    );
    assert_eq!(
        occ(&new),
        [
            "2026-10-26 10:00 Europe/Berlin [09:00Z] 30min | Kurs",
            "2026-11-02 10:00 Europe/Berlin [09:00Z] 30min | Kurs",
            "2026-11-09 10:00 Europe/Berlin [09:00Z] 30min | Kurs",
        ]
    );
}

#[test]
fn this_and_future_on_all_day_series_uses_date_until() {
    let obj = CalendarObject::parse(&cal("BEGIN:VEVENT
         UID:markt@example.org
         DTSTAMP:20261001T080000Z
         DTSTART;VALUE=DATE:20261024
         DTEND;VALUE=DATE:20261025
         RRULE:FREQ=WEEKLY
         SUMMARY:Markttag
         END:VEVENT"))
    .unwrap();
    let out = edit_event(
        &obj,
        Some(&CalTime::Date(common::date("2026-11-07"))),
        EditScope::ThisAndFuture,
        &EventChanges {
            location: Some(Some("Marktplatz".into())),
            ..Default::default()
        },
        &ctx().with_new_uid("markt-neu@example.org"),
    )
    .unwrap();
    let old = updated(&out);
    let new = roundtrip(out.created.as_ref().unwrap());
    assert_eq!(
        old.master().unwrap().rrule().unwrap().unwrap().to_string(),
        "FREQ=WEEKLY;UNTIL=20261106"
    );
    assert_eq!(
        new.master().unwrap().start().unwrap(),
        CalTime::Date(common::date("2026-11-07"))
    );
    assert_eq!(
        occ(&old),
        [
            "2026-10-24..2026-10-25 ganztägig | Markttag",
            "2026-10-31..2026-11-01 ganztägig | Markttag",
        ]
    );
    assert_eq!(
        occ(&new),
        [
            "2026-11-07..2026-11-08 ganztägig | Markttag",
            "2026-11-14..2026-11-15 ganztägig | Markttag",
        ]
    );
    assert_eq!(
        new.master().unwrap().location().as_deref(),
        Some("Marktplatz")
    );
}

#[test]
fn this_and_future_at_first_occurrence_is_all() {
    let obj = base();
    let out = edit_event(
        &obj,
        Some(&berlin_at("2026-10-05", (9, 0))),
        EditScope::ThisAndFuture,
        &EventChanges {
            summary: Some(Some("Weekly".into())),
            ..Default::default()
        },
        &ctx().with_new_uid("unbenutzt@example.org"),
    )
    .unwrap();
    assert!(out.created.is_none());
    let new = updated(&out);
    assert_eq!(new.master().unwrap().summary().as_deref(), Some("Weekly"));
    assert_eq!(new.overrides().len(), 2);
}

#[test]
fn delete_only_this() {
    let obj = base();
    // Plain occurrence.
    let out = delete_event(
        &obj,
        Some(&berlin_at("2026-10-12", (9, 0))),
        EditScope::This,
        &ctx(),
    )
    .unwrap();
    let new = updated(&out);
    assert_eq!(
        new.master().unwrap().exdates().unwrap(),
        [
            berlin_at("2026-11-02", (9, 0)),
            berlin_at("2026-10-12", (9, 0))
        ]
    );
    // Overridden occurrence: override removed as well.
    let out = delete_event(
        &new,
        Some(&berlin_at("2026-10-19", (9, 0))),
        EditScope::This,
        &ctx(),
    )
    .unwrap();
    let new = updated(&out);
    assert_eq!(new.overrides().len(), 1);
    assert_eq!(
        occ(&new),
        [
            "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min | Team-Sync",
            "2026-10-26 09:00 Europe/Berlin [08:00Z] 30min override | Team-Sync",
            "2026-11-09 09:00 Europe/Berlin [08:00Z] 30min | Team-Sync",
        ]
    );
    // Untouched lines keep their bytes.
    assert!(new.to_ics().contains("X-LICHTUNG-TEST:bleibt\r\n"));
}

#[test]
fn delete_this_and_future() {
    let obj = base();
    let out = delete_event(
        &obj,
        Some(&berlin_at("2026-10-26", (9, 0))),
        EditScope::ThisAndFuture,
        &ctx(),
    )
    .unwrap();
    assert!(out.created.is_none());
    let new = updated(&out);
    assert_eq!(
        new.master().unwrap().rrule().unwrap().unwrap().to_string(),
        "FREQ=WEEKLY;UNTIL=20261026T075959Z"
    );
    assert!(new.master().unwrap().exdates().unwrap().is_empty());
    assert_eq!(new.overrides().len(), 1);
    assert_eq!(
        occ(&new),
        [
            "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min | Team-Sync",
            "2026-10-12 09:00 Europe/Berlin [07:00Z] 30min | Team-Sync",
            "2026-10-19 10:00 Europe/Berlin [08:00Z] 30min override | Team-Sync (verlegt)",
        ]
    );
    // From the first occurrence on, everything goes.
    let out = delete_event(
        &obj,
        Some(&berlin_at("2026-10-05", (9, 0))),
        EditScope::ThisAndFuture,
        &ctx(),
    )
    .unwrap();
    assert!(out.updated.is_none());
}

#[test]
fn delete_all_and_single_events() {
    let obj = base();
    let out = delete_event(
        &obj,
        Some(&berlin_at("2026-10-12", (9, 0))),
        EditScope::All,
        &ctx(),
    )
    .unwrap();
    assert!(out.updated.is_none() && out.created.is_none());

    let single = CalendarObject::parse(&cal("BEGIN:VEVENT
         UID:zahnarzt@example.org
         DTSTAMP:20261001T080000Z
         DTSTART;TZID=Europe/Berlin:20261027T140000
         DTEND;TZID=Europe/Berlin:20261027T150000
         SUMMARY:Zahnarzt
         END:VEVENT"))
    .unwrap();
    for scope in [EditScope::This, EditScope::ThisAndFuture, EditScope::All] {
        let out = delete_event(&single, None, scope, &ctx()).unwrap();
        assert!(out.updated.is_none(), "{scope:?}");
    }
}

#[test]
fn single_event_edit_keeps_length() {
    let single = CalendarObject::parse(&cal("BEGIN:VEVENT
         UID:zahnarzt@example.org
         DTSTAMP:20261001T080000Z
         SEQUENCE:0
         DTSTART;TZID=Europe/Berlin:20261027T140000
         DTEND;TZID=Europe/Berlin:20261027T150000
         SUMMARY:Zahnarzt
         END:VEVENT"))
    .unwrap();
    let out = edit_event(
        &single,
        None,
        EditScope::This,
        &EventChanges {
            start: Some(berlin_at("2026-10-28", (8, 30))),
            ..Default::default()
        },
        &ctx(),
    )
    .unwrap();
    let new = updated(&out);
    assert_eq!(
        occ(&new),
        ["2026-10-28 08:30 Europe/Berlin [07:30Z] 60min | Zahnarzt"]
    );
    assert_eq!(new.master().unwrap().sequence(), 1);
}

#[test]
fn errors() {
    let obj = base();
    // Not an occurrence of the series.
    let err = edit_event(
        &obj,
        Some(&berlin_at("2026-10-13", (9, 0))),
        EditScope::This,
        &EventChanges::default(),
        &ctx(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::UnknownOccurrence(_)), "{err}");
    // Split without new UID.
    let err = edit_event(
        &obj,
        Some(&berlin_at("2026-10-12", (9, 0))),
        EditScope::ThisAndFuture,
        &EventChanges::default(),
        &ctx(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::InvalidEdit(_)), "{err}");
    // Rule change on a single occurrence.
    let err = edit_event(
        &obj,
        Some(&berlin_at("2026-10-12", (9, 0))),
        EditScope::This,
        &EventChanges {
            rrule: Some(Some(RecurRule::parse("FREQ=DAILY").unwrap())),
            ..Default::default()
        },
        &ctx(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::InvalidEdit(_)), "{err}");
    assert!(err.to_string().contains("Wiederholung"));
}

#[test]
fn all_can_change_and_remove_rule() {
    let obj = base();
    let out = edit_event(
        &obj,
        None,
        EditScope::All,
        &EventChanges {
            rrule: Some(Some(RecurRule::parse("FREQ=WEEKLY;INTERVAL=2").unwrap())),
            ..Default::default()
        },
        &ctx(),
    )
    .unwrap();
    let new = updated(&out);
    assert_eq!(
        occ(&new)[..2],
        [
            "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min | Team-Sync".to_string(),
            "2026-10-19 10:00 Europe/Berlin [08:00Z] 30min override | Team-Sync (verlegt)"
                .to_string(),
        ]
    );

    let out = edit_event(
        &obj,
        None,
        EditScope::All,
        &EventChanges {
            rrule: Some(None),
            ..Default::default()
        },
        &ctx(),
    )
    .unwrap();
    let new = updated(&out);
    assert!(new.overrides().is_empty());
    assert_eq!(
        occ(&new),
        ["2026-10-05 09:00 Europe/Berlin [07:00Z] 30min | Team-Sync"]
    );
}
