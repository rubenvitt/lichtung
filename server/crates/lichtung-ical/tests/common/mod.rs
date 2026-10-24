//! Shared helpers for the table tests. All fixtures are invented.
#![allow(dead_code)]

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use lichtung_ical::{expand, CalendarObject, ExpandOptions, Occurrence};

/// Wraps fixture lines (indentation is stripped) into a VCALENDAR with CRLF.
pub fn cal(body: &str) -> String {
    let mut out = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Lichtung Test//DE\r\n");
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("END:VCALENDAR\r\n");
    out
}

/// UTC instant from `YYYY-MM-DD HH:MM`.
pub fn utc(s: &str) -> DateTime<Utc> {
    let n = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").expect("timestamp");
    Utc.from_utc_datetime(&n)
}

/// Date from `YYYY-MM-DD`.
pub fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("date")
}

/// Compact, readable description of an occurrence.
///
/// Timed: `2026-10-19 09:00 Europe/Berlin [07:00Z] 30min`
/// All-day: `2026-10-24..2026-10-26 ganztägig`
pub fn fmt(o: &Occurrence) -> String {
    let mut s = match o.all_day {
        Some(a) => format!("{}..{} ganztägig", a.start, a.end),
        None => format!(
            "{} {} [{}] {}min",
            o.local_start.format("%Y-%m-%d %H:%M"),
            o.time_zone,
            o.start.format("%H:%MZ"),
            (o.end - o.start).num_minutes()
        ),
    };
    if o.is_override {
        s.push_str(" override");
    }
    if o.cancelled {
        s.push_str(" abgesagt");
    }
    s
}

/// Expands `ics` in `[from, to)` and formats the occurrences.
pub fn run(ics: &str, from: &str, to: &str, opts: &ExpandOptions) -> Vec<String> {
    let obj = CalendarObject::parse(ics).unwrap_or_else(|e| panic!("parse: {e}\n{ics}"));
    let res = expand(&obj, utc(from), utc(to), opts).unwrap_or_else(|e| panic!("expand: {e}"));
    res.occurrences.iter().map(fmt).collect()
}

/// Default options with Europe/Berlin as default zone.
pub fn berlin() -> ExpandOptions {
    ExpandOptions::new(chrono_tz::Europe::Berlin)
}

/// A VTIMEZONE for central Europe as Outlook writes it (EU rules).
pub const VTZ_EU: &str = "
    BEGIN:VTIMEZONE
    TZID:W. Europe Standard Time
    BEGIN:STANDARD
    DTSTART:16010101T030000
    TZOFFSETFROM:+0200
    TZOFFSETTO:+0100
    RRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10
    END:STANDARD
    BEGIN:DAYLIGHT
    DTSTART:16010101T020000
    TZOFFSETFROM:+0100
    TZOFFSETTO:+0200
    RRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3
    END:DAYLIGHT
    END:VTIMEZONE
";
