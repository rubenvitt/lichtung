//! The `Event` object of the sync API (`docs/api/sync-api.md`, section 3),
//! built from the stored iCalendar original. Pull (LIC-11) and MCP (LIC-12)
//! use [`event_json`]; the occurrence query reuses the building blocks.
//!
//! Mapping rules:
//!
//! - One object per calendar object resource: the series master with
//!   `recurrence` and every override in `overrides`, each complete.
//! - `DateValue`: DATE → `{"date"}`; DATE-TIME → `{"date_time", "time_zone",
//!   "utc"}`. UTC values get `time_zone: "UTC"`, floating ones `null`/`null`.
//!   TZIDs are translated to IANA (IANA, Windows/Outlook names, path
//!   prefixes, fixed offsets; own `VTIMEZONE`s by `X-LIC-LOCATION` or by
//!   comparing their offsets with the IANA zones). An unknown zone gives
//!   `time_zone: null` with `utc` set (contract addition).
//! - Objects without a master (only single occurrences of someone else's
//!   series): the head describes the earliest occurrence at its original
//!   start, `recurrence` is `{"rrule": null, "rdates": [the other
//!   occurrences]}` and `overrides` holds every occurrence.
//! - Unreadable originals become a minimal object titled
//!   [`super::UNREADABLE_TITLE`] with `start`/`end` `null` and
//!   `parse_error` (contract addition).

use std::cell::RefCell;
use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use lichtung_ical::{
    resolve_known_tzid, AlarmTrigger, CalTime, CalendarObject, Component, Event, EventEnd,
    EventStatus, IcalDuration, PartStat, ResolvedBy, TimeZoneRef, Transparency, Zone, ZoneResolver,
};
use serde::Serialize;
use serde_json::{json, Value};

use super::store::EventRow;
use crate::accounts::store::AccountRow;
use crate::app::format_ms;

/// What the JSON of an event depends on besides the event itself.
#[derive(Debug, Clone)]
pub struct EventContext {
    /// The account's own addresses (lower-case, without `mailto:`), for
    /// `is_self` and `self_status`.
    pub self_addresses: Vec<String>,
    /// Zone for floating times and dates where a UTC value is needed.
    pub default_tz: Tz,
}

impl Default for EventContext {
    fn default() -> Self {
        EventContext {
            self_addresses: Vec::new(),
            default_tz: super::default_zone(),
        }
    }
}

impl EventContext {
    /// Addresses from discovery (`calendar-user-address-set`) plus the
    /// login name if it is an e-mail address (Fastmail, iCloud).
    pub fn for_account(account: &AccountRow) -> Self {
        let mut self_addresses: Vec<String> = account
            .user_addresses
            .iter()
            .map(|a| a.trim().to_lowercase())
            .collect();
        let username = account.username.trim().to_lowercase();
        if username.contains('@') && !self_addresses.contains(&username) {
            self_addresses.push(username);
        }
        EventContext {
            self_addresses,
            ..EventContext::default()
        }
    }

    fn is_self(&self, address: &str) -> bool {
        let email = strip_mailto(address).trim().to_lowercase();
        !email.is_empty() && self.self_addresses.contains(&email)
    }
}

// ----- DateValue --------------------------------------------------------------------

/// `DateValue` of the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum DateValue {
    Date {
        date: String,
    },
    DateTime {
        date_time: String,
        time_zone: Option<String>,
        utc: Option<String>,
    },
}

pub(crate) fn fmt_utc(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn fmt_local(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S").to_string()
}

pub(crate) fn fmt_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Zones commonly meant when a custom `VTIMEZONE` matches several IANA
/// zones; tried first.
const PREFERRED_ZONES: &[&str] = &[
    "Europe/Berlin",
    "Europe/London",
    "Europe/Paris",
    "Europe/Zurich",
    "Europe/Vienna",
    "Europe/Lisbon",
    "Europe/Helsinki",
    "Europe/Moscow",
    "America/New_York",
    "America/Chicago",
    "America/Denver",
    "America/Phoenix",
    "America/Los_Angeles",
    "America/Sao_Paulo",
    "Asia/Tokyo",
    "Asia/Shanghai",
    "Asia/Kolkata",
    "Asia/Dubai",
    "Asia/Singapore",
    "Australia/Sydney",
    "Pacific/Auckland",
];

/// Zone handling for one calendar object (its `VTIMEZONE`s), with a cache
/// of TZID translations.
pub(crate) struct Zones<'a> {
    pub resolver: ZoneResolver,
    calendar: &'a Component,
    cache: RefCell<HashMap<String, Option<String>>>,
}

impl<'a> Zones<'a> {
    pub fn new(object: &'a CalendarObject, default: Tz) -> Self {
        Zones {
            resolver: object.resolver(default),
            calendar: object.calendar(),
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// IANA name for a TZID, or `None` if it cannot be translated.
    pub fn iana(&self, tzid: &str, near: NaiveDateTime) -> Option<String> {
        if let Some(hit) = self.cache.borrow().get(tzid) {
            return hit.clone();
        }
        let (zone, how) = self.resolver.resolve(tzid);
        let name = match (how, zone) {
            (ResolvedBy::Fallback, _) => None,
            (_, Zone::Iana(tz)) => Some(tz.name().to_owned()),
            (_, Zone::Custom(_)) => {
                let custom = self.resolver.resolve(tzid).0;
                let vtz = self.calendar.components_named("VTIMEZONE").find(|c| {
                    c.property("TZID")
                        .is_some_and(|p| p.value().trim() == tzid.trim().trim_matches('"'))
                });
                iana_for_custom(&custom, tzid, vtz, near.year())
            }
        };
        self.cache
            .borrow_mut()
            .insert(tzid.to_owned(), name.clone());
        name
    }

    /// Label of the zone a DATE-TIME is in: `(time_zone, floating)`.
    pub fn label(&self, t: &CalTime) -> (Option<String>, bool) {
        match t.zone() {
            Some(TimeZoneRef::Utc) => (Some("UTC".to_owned()), false),
            Some(TimeZoneRef::Floating) | None => (None, true),
            Some(TimeZoneRef::Tzid(id)) => (self.iana(id, t.naive()), false),
        }
    }

    pub fn date_value(&self, t: &CalTime) -> DateValue {
        match t {
            CalTime::Date(d) => DateValue::Date { date: fmt_date(*d) },
            CalTime::DateTime { local, .. } => {
                let (time_zone, floating) = self.label(t);
                DateValue::DateTime {
                    date_time: fmt_local(*local),
                    time_zone,
                    utc: (!floating).then(|| fmt_utc(self.resolver.to_utc(t))),
                }
            }
        }
    }
}

/// Best-effort IANA zone for a custom `VTIMEZONE`: its `X-LIC-LOCATION`,
/// else an IANA zone with the same offsets over the year of the event and
/// the next (preferring one whose city appears in the TZID).
fn iana_for_custom(zone: &Zone, tzid: &str, vtz: Option<&Component>, year: i32) -> Option<String> {
    if let Some(loc) = vtz.and_then(|v| v.property("X-LIC-LOCATION")) {
        if let Some((tz, _)) = resolve_known_tzid(loc.value()) {
            return Some(tz.name().to_owned());
        }
    }
    let samples: Vec<NaiveDateTime> = [year, year + 1]
        .iter()
        .flat_map(|&y| {
            (1..=12).flat_map(move |m| {
                [1, 15]
                    .into_iter()
                    .filter_map(move |d| NaiveDate::from_ymd_opt(y, m, d))
            })
        })
        .filter_map(|d| d.and_hms_opt(12, 0, 0))
        .collect();
    let wanted: Vec<i32> = samples.iter().map(|s| zone.offset_at(*s)).collect();
    let matches = |tz: &Tz| {
        let z = Zone::Iana(*tz);
        samples
            .iter()
            .zip(&wanted)
            .all(|(s, w)| z.offset_at(*s) == *w)
    };
    let candidates: Vec<Tz> = chrono_tz::TZ_VARIANTS
        .iter()
        .filter(|tz| tz.name().contains('/') && !tz.name().starts_with("Etc/"))
        .filter(|tz| matches(tz))
        .copied()
        .collect();
    let text = tzid.to_lowercase();
    candidates
        .iter()
        .find(|tz| {
            tz.name()
                .rsplit('/')
                .next()
                .is_some_and(|city| text.contains(&city.replace('_', " ").to_lowercase()))
        })
        .or_else(|| {
            PREFERRED_ZONES
                .iter()
                .find_map(|p| candidates.iter().find(|tz| tz.name() == *p))
        })
        .or_else(|| candidates.first())
        .map(|tz| tz.name().to_owned())
}

/// `start + duration` in the zone of `start` (days nominal, time exact).
pub(crate) fn add_duration(start: &CalTime, d: &IcalDuration, resolver: &ZoneResolver) -> CalTime {
    let days = Duration::days(d.nominal_days());
    let exact = Duration::seconds(d.exact_seconds());
    match start {
        CalTime::Date(date) => {
            // Time parts on a date are rounded up to whole days.
            let extra = if d.exact_seconds() > 0 {
                (d.exact_seconds() + 86_399) / 86_400
            } else {
                0
            };
            CalTime::Date(*date + days + Duration::days(extra))
        }
        CalTime::DateTime { local, zone } => {
            let wall = *local + days;
            match zone {
                TimeZoneRef::Floating => CalTime::floating(wall + exact),
                _ => {
                    let z = resolver.zone_of(start);
                    let utc = z.to_utc(wall) + exact;
                    let local = match zone {
                        TimeZoneRef::Utc => utc.naive_utc(),
                        _ => z.to_local(utc),
                    };
                    CalTime::DateTime {
                        local,
                        zone: zone.clone(),
                    }
                }
            }
        }
    }
}

/// The length from `start` to `end` as a duration (whole days for dates).
fn span_between(start: &CalTime, end: &CalTime, resolver: &ZoneResolver) -> IcalDuration {
    if start.is_date() && end.is_date() {
        IcalDuration::from_parts((end.date() - start.date()).num_days().max(0), 0)
    } else {
        let secs = (resolver.to_utc(end) - resolver.to_utc(start))
            .num_seconds()
            .max(0);
        IcalDuration::from_seconds(secs)
    }
}

/// The end of an event as a value (DTEND, DTSTART + DURATION, or the
/// implicit end: one day for dates, zero length otherwise).
pub(crate) fn end_of(ev: &Event<&Component>, start: &CalTime, resolver: &ZoneResolver) -> CalTime {
    match ev.end() {
        Ok(EventEnd::End(t)) => t,
        Ok(EventEnd::Duration(d)) => add_duration(start, &d, resolver),
        Ok(EventEnd::Implicit) | Err(_) => {
            if start.is_date() {
                add_duration(start, &IcalDuration::from_parts(1, 0), resolver)
            } else {
                start.clone()
            }
        }
    }
}

// ----- fields -----------------------------------------------------------------------

/// `confirmed`, `tentative` or `cancelled` (unknown values: `confirmed`).
pub(crate) fn status_name(status: &EventStatus) -> &'static str {
    match status {
        EventStatus::Tentative => "tentative",
        EventStatus::Cancelled => "cancelled",
        EventStatus::Confirmed | EventStatus::Other(_) => "confirmed",
    }
}

/// `busy` or `free`: `TRANSP`, else Outlook's
/// `X-MICROSOFT-CDO-BUSYSTATUS` (`FREE` and `WORKINGELSEWHERE` are free;
/// `TENTATIVE`, `BUSY`, `OOF` block time), else busy.
pub(crate) fn availability(ev: &Event<&Component>) -> &'static str {
    if ev.component().property("TRANSP").is_none() {
        if let Some(p) = ev.component().property("X-MICROSOFT-CDO-BUSYSTATUS") {
            let value = p.value().trim();
            return if value.eq_ignore_ascii_case("FREE")
                || value.eq_ignore_ascii_case("WORKINGELSEWHERE")
            {
                "free"
            } else {
                "busy"
            };
        }
    }
    match ev.transparency() {
        Transparency::Opaque => "busy",
        Transparency::Transparent => "free",
    }
}

fn partstat_name(p: Option<&PartStat>) -> &'static str {
    match p {
        Some(PartStat::Accepted) => "accepted",
        Some(PartStat::Declined) => "declined",
        Some(PartStat::Tentative) => "tentative",
        Some(PartStat::Delegated) => "delegated",
        Some(PartStat::NeedsAction) | Some(PartStat::Other(_)) | None => "needs_action",
    }
}

fn role_name(role: Option<&str>) -> &'static str {
    match role.map(|r| r.trim().to_ascii_uppercase()).as_deref() {
        Some("CHAIR") => "chair",
        Some("OPT-PARTICIPANT") => "optional",
        Some("NON-PARTICIPANT") => "non_participant",
        _ => "required",
    }
}

fn strip_mailto(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 7 && s[..7].eq_ignore_ascii_case("mailto:") {
        &s[7..]
    } else {
        s
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Person {
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AttendeeJson {
    pub email: String,
    pub name: Option<String>,
    pub role: &'static str,
    pub status: &'static str,
    pub is_self: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(crate) enum AlarmJson {
    Relative { minutes_before: i64 },
    Absolute { at: String },
}

/// The fields shared by the event head, overrides and occurrences.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct EventFields {
    pub title: String,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub url: Option<String>,
    pub start: DateValue,
    pub end: DateValue,
    pub all_day: bool,
    pub status: &'static str,
    pub availability: &'static str,
    pub organizer: Option<Person>,
    pub attendees: Vec<AttendeeJson>,
    pub self_status: Option<&'static str>,
    pub alarms: Vec<AlarmJson>,
    pub travel_time_minutes: Option<i64>,
}

/// Start and end of an event; `fallback_start` (an override's recurrence
/// id) stands in for a missing `DTSTART`.
pub(crate) fn start_end(
    ev: &Event<&Component>,
    fallback_start: Option<&CalTime>,
    resolver: &ZoneResolver,
) -> Result<(CalTime, CalTime), String> {
    let start = match (ev.start(), fallback_start) {
        (Ok(s), _) => s,
        (Err(_), Some(f)) => f.clone(),
        (Err(e), None) => return Err(e.to_string()),
    };
    let end = end_of(ev, &start, resolver);
    Ok((start, end))
}

/// Builds the shared fields of one `VEVENT`.
pub(crate) fn event_fields(
    ev: &Event<&Component>,
    start: &CalTime,
    end: &CalTime,
    zones: &Zones<'_>,
    ctx: &EventContext,
) -> EventFields {
    let organizer = ev.organizer().map(|o| Person {
        email: strip_mailto(&o.address).to_owned(),
        name: o.cn.clone(),
    });
    let organizer_is_self = organizer.as_ref().is_some_and(|o| ctx.is_self(&o.email));
    let attendees: Vec<AttendeeJson> = ev
        .attendees()
        .iter()
        .map(|a| AttendeeJson {
            email: a.email().trim().to_owned(),
            name: a.cn.clone(),
            role: role_name(a.role.as_deref()),
            status: partstat_name(a.partstat.as_ref()),
            is_self: ctx.is_self(&a.address),
        })
        .collect();
    let self_status = if organizer_is_self {
        None
    } else {
        attendees.iter().find(|a| a.is_self).map(|a| a.status)
    };
    let duration_secs = (zones.resolver.to_utc(end) - zones.resolver.to_utc(start)).num_seconds();
    let alarms = ev
        .alarms()
        .into_iter()
        .filter(|a| {
            !a.action
                .as_deref()
                .is_some_and(|x| x.eq_ignore_ascii_case("NONE"))
        })
        .filter_map(|a| match a.trigger? {
            AlarmTrigger::Relative {
                offset,
                related_to_end,
            } => {
                let secs = offset.approx_seconds() + if related_to_end { duration_secs } else { 0 };
                Some(AlarmJson::Relative {
                    minutes_before: -(secs / 60),
                })
            }
            AlarmTrigger::Absolute(t) => Some(AlarmJson::Absolute {
                at: fmt_utc(zones.resolver.to_utc(&t)),
            }),
        })
        .collect();
    EventFields {
        title: ev.summary().unwrap_or_default(),
        location: ev.location(),
        notes: ev.description(),
        url: ev.url(),
        start: zones.date_value(start),
        end: zones.date_value(end),
        all_day: start.is_date(),
        status: ev.status().map_or("confirmed", |s| status_name(&s)),
        availability: availability(ev),
        organizer,
        attendees,
        self_status,
        alarms,
        travel_time_minutes: ev.travel_duration().map(|d| d.approx_seconds() / 60),
    }
}

// ----- event ------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct RecurrenceJson {
    rrule: Option<String>,
    rdates: Vec<DateValue>,
    exdates: Vec<DateValue>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    incomplete: bool,
}

#[derive(Debug, Serialize)]
struct OverrideJson {
    recurrence_id: DateValue,
    #[serde(flatten)]
    fields: EventFields,
}

#[derive(Debug, Serialize)]
struct EventJson {
    id: String,
    calendar_id: String,
    uid: Option<String>,
    version: i64,
    #[serde(flatten)]
    fields: EventFields,
    recurrence: Option<RecurrenceJson>,
    overrides: Vec<OverrideJson>,
    created_at: String,
    updated_at: String,
}

fn recurrence_of(master: &Event<&Component>, zones: &Zones<'_>) -> RecurrenceJson {
    let c = master.component();
    let rrules: Vec<String> = c
        .properties_named("RRULE")
        .map(|p| p.value().trim().to_owned())
        .collect();
    let mut incomplete = rrules.len() > 1 || c.property("EXRULE").is_some();
    let rdates = match master.rdates() {
        Ok(list) => list.iter().map(|r| zones.date_value(r.start())).collect(),
        Err(_) => {
            incomplete = true;
            Vec::new()
        }
    };
    let exdates = match master.exdates() {
        Ok(list) => list.iter().map(|t| zones.date_value(t)).collect(),
        Err(_) => {
            incomplete = true;
            Vec::new()
        }
    };
    RecurrenceJson {
        rrule: rrules.into_iter().next(),
        rdates,
        exdates,
        incomplete,
    }
}

fn build(row: &EventRow, ctx: &EventContext) -> Result<Value, String> {
    let object = CalendarObject::parse(&row.ics).map_err(|e| e.to_string())?;
    let zones = Zones::new(&object, ctx.default_tz);
    let resolver = &zones.resolver;

    // Overrides with a readable RECURRENCE-ID, by original start.
    let mut overrides: Vec<(DateTime<Utc>, CalTime, Event<&Component>)> = Vec::new();
    let mut skipped = false;
    for ev in object.overrides() {
        match ev.recurrence_id() {
            Ok(Some(rid)) => overrides.push((resolver.to_utc(&rid.value), rid.value, ev)),
            _ => skipped = true,
        }
    }
    overrides.sort_by_key(|(at, _, _)| *at);
    let mut override_json = Vec::new();
    for (_, rid, ev) in &overrides {
        match start_end(ev, Some(rid), resolver) {
            Ok((start, end)) => override_json.push(OverrideJson {
                recurrence_id: zones.date_value(rid),
                fields: event_fields(ev, &start, &end, &zones, ctx),
            }),
            Err(_) => skipped = true,
        }
    }

    let (fields, recurrence) = match object.master() {
        Some(master) => {
            let (start, end) = start_end(&master, None, resolver)?;
            let fields = event_fields(&master, &start, &end, &zones, ctx);
            let recurrence = master.is_recurring().then(|| {
                let mut r = recurrence_of(&master, &zones);
                r.incomplete |= skipped;
                r
            });
            (fields, recurrence)
        }
        None => {
            let (_, rid, first) = overrides
                .first()
                .ok_or_else(|| "Enthält keinen Termin (VEVENT)".to_owned())?;
            let (start, end) = start_end(first, Some(rid), resolver)?;
            let span = span_between(&start, &end, resolver);
            let head_end = add_duration(rid, &span, resolver);
            let fields = event_fields(first, rid, &head_end, &zones, ctx);
            let recurrence = RecurrenceJson {
                rrule: None,
                rdates: overrides[1..]
                    .iter()
                    .map(|(_, r, _)| zones.date_value(r))
                    .collect(),
                exdates: Vec::new(),
                incomplete: skipped,
            };
            (fields, Some(recurrence))
        }
    };

    let event = EventJson {
        id: row.id.clone(),
        calendar_id: row.calendar_id.clone(),
        uid: row.uid.clone().or_else(|| object.uid()),
        version: row.version,
        fields,
        recurrence,
        overrides: override_json,
        created_at: format_ms(row.created_at),
        updated_at: format_ms(row.updated_at),
    };
    serde_json::to_value(event).map_err(|e| e.to_string())
}

/// The minimal object for an original that cannot be read.
fn unreadable(row: &EventRow, error: &str) -> Value {
    json!({
        "id": row.id,
        "calendar_id": row.calendar_id,
        "uid": row.uid,
        "version": row.version,
        "title": super::UNREADABLE_TITLE,
        "location": null,
        "notes": null,
        "url": null,
        "start": null,
        "end": null,
        "all_day": false,
        "status": "confirmed",
        "availability": "free",
        "recurrence": null,
        "overrides": [],
        "organizer": null,
        "attendees": [],
        "self_status": null,
        "alarms": [],
        "travel_time_minutes": null,
        "created_at": format_ms(row.created_at),
        "updated_at": format_ms(row.updated_at),
        "parse_error": error,
    })
}

/// The `Event` object of the sync API for a stored event.
pub fn event_json(row: &EventRow, ctx: &EventContext) -> Value {
    if let Some(error) = &row.parse_error {
        return unreadable(row, error);
    }
    match build(row, ctx) {
        Ok(v) => v,
        Err(e) => unreadable(row, &e),
    }
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

    fn row(ics: &str) -> EventRow {
        EventRow {
            id: "ev_1".into(),
            calendar_id: "cal_1".into(),
            href: "https://dav.example.org/cal/1.ics".into(),
            etag: Some("\"1\"".into()),
            uid: None,
            ics: super::super::normalize_ics(ics),
            parse_error: None,
            version: 3,
            created_at: 1_792_756_800_000,
            updated_at: 1_792_756_800_000,
            seq: 7,
        }
    }

    fn vcal(body: &str) -> String {
        format!("BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Beispiel//DE\n{body}END:VCALENDAR\n")
    }

    fn ev(body: &str) -> Value {
        event_json(&row(&vcal(body)), &ctx())
    }

    fn dt(local: &str, zone: Option<&str>, utc: Option<&str>) -> Value {
        json!({"date_time": local, "time_zone": zone, "utc": utc})
    }

    #[test]
    fn single_event_with_people_alarms_and_travel_time() {
        let v = ev("BEGIN:VEVENT\nUID:termin-1@example.org\n\
            DTSTART;TZID=Europe/Berlin:20261102T090000\nDTEND;TZID=Europe/Berlin:20261102T093000\n\
            SUMMARY:Team-Sync\nLOCATION:Raum 2\nDESCRIPTION:Agenda im Dokument\nURL:https://example.org/agenda\n\
            STATUS:TENTATIVE\nTRANSP:OPAQUE\nX-APPLE-TRAVEL-DURATION;VALUE=DURATION:PT20M\n\
            ORGANIZER;CN=Ben Beispiel:mailto:ben@example.org\n\
            ATTENDEE;CN=Ben Beispiel;ROLE=CHAIR;PARTSTAT=ACCEPTED:mailto:ben@example.org\n\
            ATTENDEE;CN=Anna;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:Anna@Example.org\n\
            ATTENDEE;ROLE=OPT-PARTICIPANT;PARTSTAT=DECLINED:mailto:carla@example.org\n\
            ATTENDEE;ROLE=NON-PARTICIPANT;PARTSTAT=TENTATIVE:mailto:dora@example.org\n\
            BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT10M\nEND:VALARM\n\
            BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER;RELATED=END:PT5M\nEND:VALARM\n\
            BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER;VALUE=DATE-TIME:20261102T074500Z\nEND:VALARM\n\
            BEGIN:VALARM\nACTION:NONE\nTRIGGER;VALUE=DATE-TIME:19760401T005545Z\nEND:VALARM\n\
            END:VEVENT\n");
        assert_eq!(
            v,
            json!({
                "id": "ev_1", "calendar_id": "cal_1", "uid": "termin-1@example.org", "version": 3,
                "title": "Team-Sync", "location": "Raum 2", "notes": "Agenda im Dokument",
                "url": "https://example.org/agenda",
                "start": dt("2026-11-02T09:00:00", Some("Europe/Berlin"), Some("2026-11-02T08:00:00Z")),
                "end": dt("2026-11-02T09:30:00", Some("Europe/Berlin"), Some("2026-11-02T08:30:00Z")),
                "all_day": false, "status": "tentative", "availability": "busy",
                "recurrence": null, "overrides": [],
                "organizer": {"email": "ben@example.org", "name": "Ben Beispiel"},
                "attendees": [
                    {"email": "ben@example.org", "name": "Ben Beispiel", "role": "chair", "status": "accepted", "is_self": false},
                    {"email": "Anna@Example.org", "name": "Anna", "role": "required", "status": "needs_action", "is_self": true},
                    {"email": "carla@example.org", "name": null, "role": "optional", "status": "declined", "is_self": false},
                    {"email": "dora@example.org", "name": null, "role": "non_participant", "status": "tentative", "is_self": false}
                ],
                "self_status": "needs_action",
                "alarms": [{"minutes_before": 10}, {"minutes_before": -35}, {"at": "2026-11-02T07:45:00Z"}],
                "travel_time_minutes": 20,
                "created_at": "2026-10-23T12:00:00Z", "updated_at": "2026-10-23T12:00:00Z"
            })
        );
    }

    #[test]
    fn organizer_has_no_self_status() {
        let v = ev(
            "BEGIN:VEVENT\nUID:x@example.org\nDTSTART:20261102T080000Z\n\
            ORGANIZER:mailto:anna@example.org\n\
            ATTENDEE;PARTSTAT=ACCEPTED:mailto:anna@example.org\n\
            ATTENDEE:mailto:ben@example.org\nEND:VEVENT\n",
        );
        assert_eq!(v["self_status"], Value::Null);
        assert_eq!(v["attendees"][1]["status"], "needs_action");
        assert_eq!(v["title"], "");
        // No DTEND: zero length; UTC values name the zone UTC.
        assert_eq!(v["end"], v["start"]);
        assert_eq!(
            v["start"],
            dt(
                "2026-11-02T08:00:00",
                Some("UTC"),
                Some("2026-11-02T08:00:00Z")
            )
        );
    }

    #[test]
    fn series_with_override_exdate_and_rdate() {
        let v = ev("BEGIN:VEVENT\nUID:serie@example.org\n\
            DTSTART;TZID=Europe/Berlin:20261102T090000\nDURATION:PT30M\n\
            RRULE:FREQ=WEEKLY;BYDAY=MO;UNTIL=20270201T075959Z\n\
            EXDATE;TZID=Europe/Berlin:20261116T090000\nRDATE;VALUE=DATE-TIME:20261104T120000Z\n\
            SUMMARY:Team-Sync\nEND:VEVENT\n\
            BEGIN:VEVENT\nUID:serie@example.org\nRECURRENCE-ID;TZID=Europe/Berlin:20261109T090000\n\
            DTSTART;TZID=Europe/Berlin:20261109T100000\nDTEND;TZID=Europe/Berlin:20261109T103000\n\
            SUMMARY:Team-Sync (verschoben)\nSTATUS:CANCELLED\nEND:VEVENT\n");
        assert_eq!(
            v["end"],
            dt(
                "2026-11-02T09:30:00",
                Some("Europe/Berlin"),
                Some("2026-11-02T08:30:00Z")
            )
        );
        assert_eq!(
            v["recurrence"],
            json!({
                "rrule": "FREQ=WEEKLY;BYDAY=MO;UNTIL=20270201T075959Z",
                "rdates": [dt("2026-11-04T12:00:00", Some("UTC"), Some("2026-11-04T12:00:00Z"))],
                "exdates": [dt("2026-11-16T09:00:00", Some("Europe/Berlin"), Some("2026-11-16T08:00:00Z"))]
            })
        );
        let o = &v["overrides"][0];
        assert_eq!(
            o["recurrence_id"],
            dt(
                "2026-11-09T09:00:00",
                Some("Europe/Berlin"),
                Some("2026-11-09T08:00:00Z")
            )
        );
        assert_eq!(o["title"], "Team-Sync (verschoben)");
        assert_eq!(o["status"], "cancelled");
        assert_eq!(
            o["start"],
            dt(
                "2026-11-09T10:00:00",
                Some("Europe/Berlin"),
                Some("2026-11-09T09:00:00Z")
            )
        );
        assert!(o.get("id").is_none() && o.get("recurrence").is_none());
    }

    #[test]
    fn several_rrules_are_incomplete() {
        let v = ev(
            "BEGIN:VEVENT\nUID:r@example.org\nDTSTART:20261102T080000Z\n\
            RRULE:FREQ=DAILY;COUNT=2\nRRULE:FREQ=WEEKLY;COUNT=2\nEND:VEVENT\n",
        );
        assert_eq!(v["recurrence"]["rrule"], "FREQ=DAILY;COUNT=2");
        assert_eq!(v["recurrence"]["incomplete"], true);
    }

    #[test]
    fn all_day_and_floating_values() {
        let v = ev(
            "BEGIN:VEVENT\nUID:a@example.org\nDTSTART;VALUE=DATE:20261103\nDURATION:P2D\n\
            TRANSP:TRANSPARENT\nEND:VEVENT\n",
        );
        assert_eq!(v["start"], json!({"date": "2026-11-03"}));
        assert_eq!(v["end"], json!({"date": "2026-11-05"}));
        assert_eq!(v["all_day"], true);
        assert_eq!(v["availability"], "free");

        let v = ev("BEGIN:VEVENT\nUID:b@example.org\nDTSTART;VALUE=DATE:20261103\nEND:VEVENT\n");
        assert_eq!(
            v["end"],
            json!({"date": "2026-11-04"}),
            "implicit end: one day"
        );

        let v = ev("BEGIN:VEVENT\nUID:c@example.org\nDTSTART:20261103T070000\n\
            DTEND:20261103T073000\nEND:VEVENT\n");
        assert_eq!(v["start"], dt("2026-11-03T07:00:00", None, None));
    }

    const CENTRAL_EUROPE: &str = "BEGIN:STANDARD\nDTSTART:19701025T030000\nTZOFFSETFROM:+0200\n\
        TZOFFSETTO:+0100\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\nEND:STANDARD\n\
        BEGIN:DAYLIGHT\nDTSTART:19700329T020000\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0200\n\
        RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\nEND:DAYLIGHT\n";

    #[test]
    fn zones_are_translated_to_iana() {
        let cases = [
            // Windows name.
            ("W. Europe Standard Time", String::new(), Some("Europe/Berlin")),
            // Path prefix.
            (
                "/mozilla.org/20050126_1/Europe/Berlin",
                String::new(),
                Some("Europe/Berlin"),
            ),
            // Own VTIMEZONE with X-LIC-LOCATION.
            (
                "Eigene Zone",
                format!(
                    "BEGIN:VTIMEZONE\nTZID:Eigene Zone\nX-LIC-LOCATION:Europe/Vienna\n{CENTRAL_EUROPE}END:VTIMEZONE\n"
                ),
                Some("Europe/Vienna"),
            ),
            // Own VTIMEZONE matched by its offsets (central Europe).
            (
                "Mitteleuropäische Zeit",
                format!("BEGIN:VTIMEZONE\nTZID:Mitteleuropäische Zeit\n{CENTRAL_EUROPE}END:VTIMEZONE\n"),
                Some("Europe/Berlin"),
            ),
            // Same offsets, but the name names a city.
            (
                "Ortszeit Madrid",
                format!("BEGIN:VTIMEZONE\nTZID:Ortszeit Madrid\n{CENTRAL_EUROPE}END:VTIMEZONE\n"),
                Some("Europe/Madrid"),
            ),
            // Unknown name without definition: no zone, but an instant.
            ("Irgendwo/Nirgendwo", String::new(), None),
        ];
        for (tzid, vtz, want) in cases {
            let v = ev(&format!(
                "{vtz}BEGIN:VEVENT\nUID:z@example.org\nDTSTART;TZID=\"{tzid}\":20261102T090000\nEND:VEVENT\n"
            ));
            let start = &v["start"];
            assert_eq!(start["time_zone"], json!(want), "{tzid}: {start}");
            assert_eq!(start["utc"], "2026-11-02T08:00:00Z", "{tzid}: {start}");
        }
    }

    #[test]
    fn object_without_master_lists_its_occurrences() {
        let v = ev("BEGIN:VEVENT\nUID:fremd@example.org\nRECURRENCE-ID:20261116T080000Z\n\
            DTSTART:20261116T090000Z\nDTEND:20261116T100000Z\nSUMMARY:Einzeln eingeladen (2)\nEND:VEVENT\n\
            BEGIN:VEVENT\nUID:fremd@example.org\nRECURRENCE-ID:20261109T080000Z\n\
            DTSTART:20261109T083000Z\nDTEND:20261109T093000Z\nSUMMARY:Einzeln eingeladen\nEND:VEVENT\n");
        assert_eq!(v["title"], "Einzeln eingeladen");
        assert_eq!(
            v["start"],
            dt(
                "2026-11-09T08:00:00",
                Some("UTC"),
                Some("2026-11-09T08:00:00Z")
            )
        );
        assert_eq!(
            v["end"],
            dt(
                "2026-11-09T09:00:00",
                Some("UTC"),
                Some("2026-11-09T09:00:00Z")
            )
        );
        assert_eq!(v["recurrence"]["rrule"], Value::Null);
        assert_eq!(
            v["recurrence"]["rdates"],
            json!([dt(
                "2026-11-16T08:00:00",
                Some("UTC"),
                Some("2026-11-16T08:00:00Z")
            )])
        );
        let overrides = v["overrides"].as_array().unwrap();
        assert_eq!(overrides.len(), 2);
        assert_eq!(overrides[0]["start"]["utc"], "2026-11-09T08:30:00Z");
        assert_eq!(overrides[1]["title"], "Einzeln eingeladen (2)");
    }

    #[test]
    fn unreadable_objects_are_marked() {
        let mut r = row("kaputt");
        r.parse_error = Some("iCalendar-Fehler in Zeile 1: kaputt".into());
        r.uid = Some("k@example.org".into());
        let v = event_json(&r, &ctx());
        assert_eq!(v["title"], "(nicht lesbarer Termin)");
        assert_eq!(v["uid"], "k@example.org");
        assert_eq!(v["start"], Value::Null);
        assert_eq!(v["availability"], "free");
        assert_eq!(v["parse_error"], "iCalendar-Fehler in Zeile 1: kaputt");
        assert_eq!(v["version"], 3);

        // No usable start: same form.
        let v = ev("BEGIN:VEVENT\nUID:y@example.org\nSUMMARY:ohne Beginn\nEND:VEVENT\n");
        assert_eq!(v["title"], "(nicht lesbarer Termin)");
        assert!(v["parse_error"].is_string());
    }

    #[test]
    fn context_takes_addresses_and_login() {
        let account = AccountRow {
            id: "acc_1".into(),
            kind: crate::accounts::AccountKind::Fastmail,
            name: "Fastmail".into(),
            username: "Anna@Example.org".into(),
            base_url: "https://dav.example.org/".into(),
            principal_url: None,
            home_url: None,
            auto_schedule: true,
            user_addresses: vec!["anna.alt@example.org".into()],
            credentials: None,
            status_state: crate::accounts::StatusState::Ok,
            status_message: None,
            last_sync_at: None,
            created_at: 0,
            updated_at: 0,
            seq: 0,
        };
        let ctx = EventContext::for_account(&account);
        assert_eq!(
            ctx.self_addresses,
            ["anna.alt@example.org", "anna@example.org"]
        );
        assert!(ctx.is_self("MAILTO:anna@example.org"));
        assert!(!ctx.is_self("mailto:ben@example.org"));
    }
}
