//! The iCalendar object of a blocker, built from its source event.
//!
//! A blocker carries only the times of its source: `DTSTART`, `DTEND` or
//! `DURATION`, and for series `RRULE`, `RDATE`, `EXDATE` and the moved
//! occurrences (`RECURRENCE-ID`), plus the zones they name. Title,
//! location, notes, attendees and alarms of the source never leave its
//! calendar. The blocker is busy (`TRANSP:OPAQUE`), has the rule's title
//! and names its source in `X-LICHTUNG-BLOCKER-OF` (the event ID), which
//! the event JSON shows as `blocker_of`.
//!
//! **What blocks.** A source (or one occurrence of it) blocks unless it is
//! cancelled, marked free (`TRANSP:TRANSPARENT`), an invitation the user
//! declined, or all-day while the rule leaves all-day events out. A moved
//! occurrence that does not block becomes an `EXDATE` of the blocker. If
//! the series itself (the master) does not block, the whole source gets no
//! blocker.

use chrono::{DateTime, Utc};
use lichtung_ical::{CalendarObject, Component, Event, PartStat, Property, Transparency};

use crate::events::EventContext;

/// Property naming the source event of a blocker (its event ID).
pub const BLOCKER_OF: &str = "X-LICHTUNG-BLOCKER-OF";

/// Properties copied from the source, per `VEVENT`.
const TIME_PROPERTIES: [&str; 7] = [
    "DTSTART",
    "DTEND",
    "DURATION",
    "RRULE",
    "RDATE",
    "EXDATE",
    "RECURRENCE-ID",
];

/// What a blocker needs from its rule.
#[derive(Debug, Clone)]
pub struct BlockerSpec<'a> {
    pub uid: &'a str,
    pub title: &'a str,
    /// Also block all-day events.
    pub all_day: bool,
    /// Event ID of the source (written as [`BLOCKER_OF`]).
    pub source_event_id: &'a str,
}

/// Whether one `VEVENT` of the source blocks time.
fn blocks(ev: &Event<&Component>, ctx: &EventContext, all_day: bool) -> bool {
    if ev.is_cancelled() || ev.transparency() == Transparency::Transparent {
        return false;
    }
    if !all_day && ev.is_all_day() {
        return false;
    }
    let declined = ev
        .attendees()
        .iter()
        .any(|a| ctx.is_self(&a.address) && a.partstat == Some(PartStat::Declined));
    !declined
}

/// The source names a blocker (it is one itself): never mirrored again,
/// so two rules in opposite directions do not feed each other.
pub fn is_blocker(object: &CalendarObject) -> bool {
    object
        .events()
        .any(|e| e.component().property(BLOCKER_OF).is_some())
}

/// The `VEVENT` of a blocker: the time properties of `source`, the rest
/// from `spec`.
fn blocker_event(source: &Component, spec: &BlockerSpec<'_>, now: DateTime<Utc>) -> Component {
    let mut out = Component::new("VEVENT");
    out.insert_property(Property::new("UID", spec.uid));
    out.insert_property(Property::new(
        "DTSTAMP",
        now.format("%Y%m%dT%H%M%SZ").to_string(),
    ));
    for p in source.properties() {
        if TIME_PROPERTIES.iter().any(|n| p.is(n)) {
            out.insert_property(p.clone());
        }
    }
    out.insert_property(Property::new_text("SUMMARY", spec.title));
    out.insert_property(Property::new("TRANSP", "OPAQUE"));
    out.insert_property(Property::new(BLOCKER_OF, spec.source_event_id));
    out
}

/// The blocker for `source_ics`, or `None` if the source blocks no time
/// (module docs) or cannot be read.
pub fn build_blocker(
    source_ics: &str,
    ctx: &EventContext,
    spec: &BlockerSpec<'_>,
    now: DateTime<Utc>,
) -> Option<String> {
    let source = CalendarObject::parse(source_ics).ok()?;
    if is_blocker(&source) {
        return None;
    }
    let mut events: Vec<Component> = Vec::new();
    match source.master() {
        Some(master) => {
            if !blocks(&master, ctx, spec.all_day) {
                return None;
            }
            let mut head = blocker_event(master.component(), spec, now);
            let mut moved = Vec::new();
            for ov in source.overrides() {
                if blocks(&ov, ctx, spec.all_day) {
                    moved.push(blocker_event(ov.component(), spec, now));
                } else if let Some(rid) = ov.component().property("RECURRENCE-ID") {
                    // The occurrence stays out of the blocker.
                    let mut ex = Property::new("EXDATE", rid.value());
                    for param in rid.params() {
                        if !param.name().eq_ignore_ascii_case("RANGE") {
                            ex.set_param(param.name(), param.value());
                        }
                    }
                    head.insert_property(ex);
                }
            }
            events.push(head);
            events.extend(moved);
        }
        None => {
            // Only single occurrences (an invitation to one date of a
            // series): each one that blocks.
            for ov in source.overrides() {
                if blocks(&ov, ctx, spec.all_day) {
                    events.push(blocker_event(ov.component(), spec, now));
                }
            }
            if events.is_empty() {
                return None;
            }
        }
    }
    let mut calendar = Component::new("VCALENDAR");
    calendar.insert_property(Property::new("VERSION", "2.0"));
    calendar.insert_property(Property::new("PRODID", "-//Lichtung//Blocker//DE"));
    let zones = source
        .calendar()
        .components()
        .iter()
        .filter(|c| c.is("VTIMEZONE"))
        .cloned();
    calendar.components_mut().extend(zones);
    calendar.components_mut().extend(events);
    let mut object = CalendarObject::from_component(calendar).ok()?;
    lichtung_ical::ensure_vtimezones(&mut object);
    Some(object.to_ics())
}

/// The source event ID a stored object names as blocker, if any.
pub fn blocker_of(object: &CalendarObject) -> Option<String> {
    object.events().find_map(|e| {
        e.component()
            .property(BLOCKER_OF)
            .map(|p| p.value().trim().to_owned())
            .filter(|v| !v.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> EventContext {
        EventContext {
            self_addresses: vec!["anna@example.org".into()],
            default_tz: chrono_tz::Europe::Berlin,
        }
    }

    fn vcal(body: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//DE\r\n{body}END:VCALENDAR\r\n"
        )
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
    }

    fn spec(all_day: bool) -> BlockerSpec<'static> {
        BlockerSpec {
            uid: "blk-1@lichtung",
            title: "Blockiert",
            all_day,
            source_event_id: "ev_quelle",
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn build(body: &str) -> Option<String> {
        build_blocker(&vcal(body), &ctx(), &spec(false), now())
    }

    #[test]
    fn single_event_keeps_only_times() {
        let out = build(
            "BEGIN:VEVENT\nUID:arzt@example.org\nDTSTAMP:20261020T070000Z\n\
             DTSTART;TZID=Europe/Berlin:20261113T140000\nDTEND;TZID=Europe/Berlin:20261113T150000\n\
             SUMMARY:Zahnarzt\nLOCATION:Praxis am Markt\nDESCRIPTION:Geheim\n\
             ATTENDEE;PARTSTAT=ACCEPTED:mailto:bert@example.org\n\
             BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT10M\nEND:VALARM\nEND:VEVENT\n",
        )
        .unwrap();
        assert!(out.contains("UID:blk-1@lichtung\r\n"), "{out}");
        assert!(out.contains("DTSTART;TZID=Europe/Berlin:20261113T140000\r\n"));
        assert!(out.contains("DTEND;TZID=Europe/Berlin:20261113T150000\r\n"));
        assert!(out.contains("SUMMARY:Blockiert\r\n"));
        assert!(out.contains("TRANSP:OPAQUE\r\n"));
        assert!(out.contains("X-LICHTUNG-BLOCKER-OF:ev_quelle\r\n"));
        assert!(out.contains("BEGIN:VTIMEZONE"), "zone added: {out}");
        for gone in ["Zahnarzt", "Praxis", "Geheim", "bert@", "VALARM"] {
            assert!(!out.contains(gone), "{gone} leaked: {out}");
        }
        let object = CalendarObject::parse(&out).unwrap();
        assert_eq!(blocker_of(&object).as_deref(), Some("ev_quelle"));
        assert!(is_blocker(&object));
    }

    #[test]
    fn series_with_moved_and_cancelled_occurrences() {
        let out = build(
            "BEGIN:VEVENT\nUID:team@example.org\nDTSTART;TZID=Europe/Berlin:20261102T090000\n\
             DTEND;TZID=Europe/Berlin:20261102T093000\nRRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=6\n\
             EXDATE;TZID=Europe/Berlin:20261116T090000\nSUMMARY:Team-Sync\nEND:VEVENT\n\
             BEGIN:VEVENT\nUID:team@example.org\nRECURRENCE-ID;TZID=Europe/Berlin:20261109T090000\n\
             DTSTART;TZID=Europe/Berlin:20261109T100000\nDTEND;TZID=Europe/Berlin:20261109T103000\n\
             SUMMARY:Team-Sync (verschoben)\nEND:VEVENT\n\
             BEGIN:VEVENT\nUID:team@example.org\nRECURRENCE-ID;TZID=Europe/Berlin:20261123T090000\n\
             DTSTART;TZID=Europe/Berlin:20261123T090000\nDTEND;TZID=Europe/Berlin:20261123T093000\n\
             STATUS:CANCELLED\nSUMMARY:Team-Sync\nEND:VEVENT\n\
             BEGIN:VEVENT\nUID:team@example.org\nRECURRENCE-ID;TZID=Europe/Berlin:20261130T090000\n\
             DTSTART;TZID=Europe/Berlin:20261130T090000\nDTEND;TZID=Europe/Berlin:20261130T093000\n\
             TRANSP:TRANSPARENT\nSUMMARY:Team-Sync\nEND:VEVENT\n",
        )
        .unwrap();
        let object = CalendarObject::parse(&out).unwrap();
        let master = object.master().unwrap();
        assert!(master.component().property("RRULE").is_some());
        let exdates: Vec<&str> = master
            .component()
            .properties_named("EXDATE")
            .map(|p| p.value())
            .collect();
        assert_eq!(
            exdates,
            ["20261116T090000", "20261123T090000", "20261130T090000"]
        );
        let moved = object.overrides();
        assert_eq!(moved.len(), 1);
        assert_eq!(
            moved[0].component().property("DTSTART").unwrap().value(),
            "20261109T100000"
        );
        assert!(!out.contains("Team-Sync"));
        assert!(object
            .events()
            .all(|e| e.uid().as_deref() == Some("blk-1@lichtung")));
    }

    #[test]
    fn what_does_not_block() {
        let event = |extra: &str| {
            format!(
                "BEGIN:VEVENT\nUID:x@example.org\nDTSTART:20261113T140000Z\nDTEND:20261113T150000Z\n\
                 SUMMARY:Etwas\n{extra}END:VEVENT\n"
            )
        };
        assert!(build(&event("STATUS:CANCELLED\n")).is_none());
        assert!(build(&event("TRANSP:TRANSPARENT\n")).is_none());
        assert!(build(&event(
            "ATTENDEE;PARTSTAT=DECLINED:mailto:Anna@Example.org\n"
        ))
        .is_none());
        assert!(build(&event(
            "ATTENDEE;PARTSTAT=DECLINED:mailto:bert@example.org\n"
        ))
        .is_some());
        assert!(build(&event(
            "ATTENDEE;PARTSTAT=TENTATIVE:mailto:anna@example.org\n"
        ))
        .is_some());
        // A blocker is never mirrored again.
        assert!(build(&event("X-LICHTUNG-BLOCKER-OF:ev_anderer\n")).is_none());
        assert!(build_blocker("kaputt", &ctx(), &spec(false), now()).is_none());
    }

    #[test]
    fn all_day_only_when_the_rule_says_so() {
        let body = "BEGIN:VEVENT\nUID:urlaub@example.org\nDTSTART;VALUE=DATE:20261120\n\
                    DTEND;VALUE=DATE:20261122\nSUMMARY:Urlaub\nEND:VEVENT\n";
        assert!(build(body).is_none());
        let out = build_blocker(&vcal(body), &ctx(), &spec(true), now()).unwrap();
        assert!(out.contains("DTSTART;VALUE=DATE:20261120\r\n"), "{out}");
        assert!(out.contains("DTEND;VALUE=DATE:20261122\r\n"));
    }

    #[test]
    fn single_occurrences_without_master() {
        let out = build(
            "BEGIN:VEVENT\nUID:verein@example.org\nRECURRENCE-ID:20261104T090000Z\n\
             DTSTART:20261104T090000Z\nDTEND:20261104T100000Z\nSUMMARY:Verein\n\
             ATTENDEE;PARTSTAT=ACCEPTED:mailto:anna@example.org\nEND:VEVENT\n\
             BEGIN:VEVENT\nUID:verein@example.org\nRECURRENCE-ID:20261111T090000Z\n\
             DTSTART:20261111T090000Z\nDTEND:20261111T100000Z\nSUMMARY:Verein\n\
             ATTENDEE;PARTSTAT=DECLINED:mailto:anna@example.org\nEND:VEVENT\n",
        )
        .unwrap();
        let object = CalendarObject::parse(&out).unwrap();
        assert!(object.master().is_none());
        assert_eq!(object.overrides().len(), 1);
        assert!(out.contains("RECURRENCE-ID:20261104T090000Z"));
        assert!(!out.contains("20261111T090000Z"));
    }
}
