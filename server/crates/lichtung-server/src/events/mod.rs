//! Stored events: one row per calendar object resource (one UID in one
//! calendar) with the iCalendar original, plus the event JSON of the sync
//! API ([`json::event_json`]) and the occurrence query
//! ([`occurrences`], `GET /api/occurrences`).
//!
//! The original (`events.ics`) is the source of truth. Everything else in a
//! row is derived from it with [`derive`] when the row is written and only
//! serves queries (time range, UID); the JSON is built from the original on
//! every read, so improvements of the mapping need no migration.
//!
//! Line endings: CalDAV REPORT answers carry the data inside XML, and XML
//! parsers turn CRLF into LF. To store every object the same way no matter
//! how it was fetched (REPORT or GET), [`normalize_ics`] converts all line
//! endings to CRLF (RFC 5545) and trims surrounding whitespace; nothing
//! else is touched (folding, order, unknown properties stay).

pub mod json;
pub mod occurrences;
pub mod store;

use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use lichtung_ical::{expand, CalendarObject, ExpandOptions, RecurRule};
use serde_json::Value;

use crate::app::App;
use crate::error::{AppError, AppResult};

pub use json::{event_json, EventContext};
pub use store::EventRow;

/// Title of events whose iCalendar data could not be read.
pub const UNREADABLE_TITLE: &str = "(nicht lesbarer Termin)";

/// Upper bound of occurrences scanned to find the end of a finite series.
const RANGE_SCAN_LIMIT: usize = 200_000;

/// The server's zone for floating times and all-day dates where no other
/// zone is known: `TZ` if it names an IANA zone, else UTC.
pub fn default_zone() -> Tz {
    std::env::var("TZ")
        .ok()
        .and_then(|v| Tz::from_str(v.trim().trim_start_matches(':')).ok())
        .unwrap_or(Tz::UTC)
}

/// Normalizes line endings to CRLF and trims surrounding whitespace (see
/// the module docs). Ends with CRLF.
pub fn normalize_ics(raw: &str) -> String {
    let trimmed = raw.trim_matches(|c: char| c == '\r' || c == '\n' || c == ' ' || c == '\t');
    let mut out = String::with_capacity(trimmed.len() + trimmed.len() / 40 + 2);
    let mut chars = trimmed.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\r\n");
            }
            '\n' => out.push_str("\r\n"),
            other => out.push(other),
        }
    }
    out.push_str("\r\n");
    out
}

/// Query columns derived from the original.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Derived {
    pub uid: Option<String>,
    pub title: Option<String>,
    /// Earliest start of any occurrence (Unix ms).
    pub start_utc: Option<i64>,
    /// Latest end of any occurrence (Unix ms); `None` for endless series.
    pub range_end_utc: Option<i64>,
    pub is_recurring: bool,
    pub all_day: bool,
    /// `confirmed`, `tentative`, `cancelled`.
    pub status: Option<String>,
    /// `busy` or `free`.
    pub transparency: Option<String>,
}

/// Result of reading an original: the derived columns, or why it could not
/// be read (German, stored in `parse_error`).
pub type Derivation = Result<Derived, String>;

fn ms(t: DateTime<Utc>) -> i64 {
    t.timestamp_millis()
}

/// Reads the original and derives the query columns. Fails if the data is
/// not iCalendar, holds no `VEVENT`, or the first event has no readable
/// `DTSTART`.
pub fn derive(ics: &str, zone: Tz) -> Derivation {
    let object = CalendarObject::parse(ics).map_err(|e| e.to_string())?;
    let resolver = object.resolver(zone);
    let base = match object.master() {
        Some(m) => m,
        None => object
            .events()
            .next()
            .ok_or_else(|| "Enthält keinen Termin (VEVENT)".to_owned())?,
    };
    let base_start = base.start().map_err(|e| e.to_string())?;
    let mut start = resolver.to_utc(&base_start);
    for ev in object.overrides() {
        if let Ok(s) = ev.start() {
            start = start.min(resolver.to_utc(&s));
        }
    }

    let recurring = object.master().is_some_and(|m| m.is_recurring());
    // Endless: an RRULE without COUNT/UNTIL (or one we cannot read), or a
    // THISANDFUTURE override (shifts every later instance).
    let endless = object.master().is_some_and(|m| {
        m.component()
            .properties_named("RRULE")
            .any(|p| RecurRule::parse(p.value()).map_or(true, |r| r.is_infinite()))
    }) || object.overrides().iter().any(|o| {
        o.recurrence_id()
            .ok()
            .flatten()
            .is_some_and(|r| r.this_and_future)
    });

    let range_end = if endless {
        None
    } else {
        let mut options = ExpandOptions::new(zone);
        options.include_cancelled = true;
        options.max_occurrences = RANGE_SCAN_LIMIT;
        let from = Utc
            .with_ymd_and_hms(1000, 1, 1, 0, 0, 0)
            .single()
            .expect("valid date");
        let to = Utc
            .with_ymd_and_hms(3000, 1, 1, 0, 0, 0)
            .single()
            .expect("valid date");
        match expand(&object, from, to, &options) {
            Ok(x) if !x.truncated => {
                let mut end = start;
                for o in &x.occurrences {
                    start = start.min(o.start);
                    end = end.max(o.end);
                }
                Some(ms(end))
            }
            // Too many instances or unreadable rule: treat as endless, the
            // expansion at query time decides.
            _ => None,
        }
    };

    let status = base
        .status()
        .map(|s| json::status_name(&s).to_owned())
        .unwrap_or_else(|| "confirmed".to_owned());
    let transparency = match base.transparency() {
        lichtung_ical::Transparency::Opaque => "busy",
        lichtung_ical::Transparency::Transparent => "free",
    };
    Ok(Derived {
        uid: object.uid(),
        title: base.summary(),
        start_utc: Some(ms(start)),
        range_end_utc: range_end,
        is_recurring: recurring,
        all_day: base_start.is_date(),
        status: Some(status),
        transparency: Some(transparency.to_owned()),
    })
}

/// Best-effort `UID` of data that could not be parsed.
pub fn sniff_uid(ics: &str) -> Option<String> {
    ics.lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("UID")
                .then(|| value.trim().to_owned())
        })
        .filter(|u| !u.is_empty())
}

impl App {
    /// One event in its JSON form (sync API contract, section 3).
    pub async fn event(&self, id: &str) -> AppResult<Value> {
        let id = id.to_owned();
        self.with_reader(move |conn| {
            let row = store::event(conn, &id)?
                .ok_or_else(|| AppError::NotFound("Termin nicht gefunden".into()))?;
            let ctx = store::context_for_calendar(conn, &row.calendar_id)?;
            Ok(event_json(&row, &ctx))
        })
        .await
    }

    /// All events of a calendar in their JSON form, ordered by start.
    pub async fn calendar_events(&self, calendar_id: &str) -> AppResult<Vec<Value>> {
        let calendar_id = calendar_id.to_owned();
        self.with_reader(move |conn| {
            let ctx = store::context_for_calendar(conn, &calendar_id)?;
            Ok(store::events_of_calendar(conn, &calendar_id)?
                .iter()
                .map(|row| event_json(row, &ctx))
                .collect())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_become_crlf() {
        assert_eq!(normalize_ics("A\nB\n"), "A\r\nB\r\n");
        assert_eq!(normalize_ics("\n  A\r\nB\rC"), "A\r\nB\r\nC\r\n");
        assert_eq!(normalize_ics("A\r\n DESC\r\n"), "A\r\n DESC\r\n");
    }

    fn wrap(body: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//DE\r\n{body}END:VCALENDAR\r\n"
        )
    }

    #[test]
    fn derives_range_of_single_and_series() {
        let single = wrap(
            "BEGIN:VEVENT\r\nUID:a@example.org\r\nDTSTART;TZID=Europe/Berlin:20261103T140000\r\n\
             DTEND;TZID=Europe/Berlin:20261103T150000\r\nSUMMARY:Zahnarzt\r\nEND:VEVENT\r\n",
        );
        let d = derive(&single, Tz::UTC).unwrap();
        assert_eq!(d.uid.as_deref(), Some("a@example.org"));
        assert_eq!(d.title.as_deref(), Some("Zahnarzt"));
        assert_eq!(d.start_utc, Some(1_793_710_800_000)); // 13:00Z
        assert_eq!(d.range_end_utc, Some(1_793_714_400_000));
        assert!(!d.is_recurring && !d.all_day);
        assert_eq!(d.status.as_deref(), Some("confirmed"));
        assert_eq!(d.transparency.as_deref(), Some("busy"));

        let finite = wrap(
            "BEGIN:VEVENT\r\nUID:b@example.org\r\nDTSTART;VALUE=DATE:20261102\r\n\
             RRULE:FREQ=DAILY;COUNT=3\r\nTRANSP:TRANSPARENT\r\nSUMMARY:Urlaub\r\nEND:VEVENT\r\n",
        );
        let d = derive(&finite, Tz::UTC).unwrap();
        assert!(d.is_recurring && d.all_day);
        // Last instance 4 Nov, ends 5 Nov 00:00 UTC.
        assert_eq!(d.range_end_utc, Some(1_793_836_800_000));
        assert_eq!(d.transparency.as_deref(), Some("free"));

        let endless = wrap(
            "BEGIN:VEVENT\r\nUID:c@example.org\r\nDTSTART:20261102T080000Z\r\n\
             RRULE:FREQ=WEEKLY\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:c@example.org\r\nRECURRENCE-ID:20261109T080000Z\r\n\
             DTSTART:20261001T080000Z\r\nEND:VEVENT\r\n",
        );
        let d = derive(&endless, Tz::UTC).unwrap();
        assert_eq!(d.range_end_utc, None);
        // An override moved before the master start widens the range.
        assert_eq!(d.start_utc, Some(1_790_841_600_000));
    }

    #[test]
    fn unreadable_data_is_reported() {
        assert!(derive("kein Kalender", Tz::UTC).is_err());
        let todo = wrap("BEGIN:VTODO\r\nUID:t@example.org\r\nEND:VTODO\r\n");
        assert_eq!(
            derive(&todo, Tz::UTC).unwrap_err(),
            "Enthält keinen Termin (VEVENT)"
        );
        let no_start = wrap("BEGIN:VEVENT\r\nUID:x@example.org\r\nEND:VEVENT\r\n");
        assert!(derive(&no_start, Tz::UTC).is_err());
        assert_eq!(
            sniff_uid("BEGIN:VEVENT\nUID: x@example.org \nkaputt").as_deref(),
            Some("x@example.org")
        );
    }
}
