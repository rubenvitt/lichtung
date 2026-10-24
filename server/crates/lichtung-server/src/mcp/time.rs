//! Times in MCP tool arguments and results.
//!
//! - Arguments: an ISO date (`2026-11-03`) or an RFC 3339 instant **with
//!   offset** (`2026-11-03T14:00:00+01:00`, `…Z`). A wall-clock time without
//!   offset is refused with an explanation, because it is ambiguous.
//! - Results: instants as RFC 3339 in the requested output zone, dates as
//!   ISO dates.

use std::str::FromStr;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Offset, Utc};
use chrono_tz::Tz;
use lichtung_ical::Zone;
use serde_json::{json, Value};

use super::args::{invalid, ArgResult};
use crate::events::default_zone;
use crate::events::occurrences::MAX_RANGE_DAYS;

/// A time argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeArg {
    /// All-day (ISO date).
    Date(NaiveDate),
    /// An instant with the offset it was given in.
    At(DateTime<FixedOffset>),
}

fn example_hint() -> &'static str {
    "Richtig ist ein Zeitpunkt mit Offset, z. B. „2026-11-03T14:00:00+01:00“ (14 Uhr Winterzeit \
     in Deutschland), „2026-07-03T14:00:00+02:00“ (Sommerzeit) oder „2026-11-03T13:00:00Z“ (UTC), \
     oder für ganztägige Termine nur das Datum „2026-11-03“."
}

/// Parses a date or an RFC 3339 instant with offset.
pub fn parse_time(key: &str, raw: &str) -> ArgResult<TimeArg> {
    let raw = raw.trim();
    if let Ok(d) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Ok(TimeArg::Date(d));
    }
    if let Ok(t) = DateTime::parse_from_rfc3339(raw) {
        return Ok(TimeArg::At(t));
    }
    // RFC 3339 without seconds ("2026-11-03T14:00+01:00").
    if let Ok(t) = DateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M%:z") {
        return Ok(TimeArg::At(t));
    }
    let without_offset = [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ]
    .iter()
    .any(|f| NaiveDateTime::parse_from_str(raw, f).is_ok());
    if without_offset {
        return Err(invalid(format!(
            "„{key}“ hat keine Zeitzone: „{raw}“. Lichtung nimmt Uhrzeiten nur mit Offset an, damit \
             kein Termin in der falschen Zeitzone landet. {}",
            example_hint()
        )));
    }
    Err(invalid(format!(
        "„{key}“ ist kein gültiger Zeitpunkt: „{raw}“. {}",
        example_hint()
    )))
}

/// An IANA zone (`time_zone`).
pub fn parse_zone(key: &str, raw: &str) -> ArgResult<Tz> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("utc") || raw == "Z" {
        return Ok(Tz::UTC);
    }
    Tz::from_str(raw).map_err(|_| {
        invalid(format!(
            "„{key}“ muss eine IANA-Zeitzone sein, z. B. „Europe/Berlin“ oder „UTC“, nicht „{raw}“."
        ))
    })
}

/// `hh:mm` (24 hours).
pub fn parse_clock(key: &str, raw: &str) -> ArgResult<NaiveTime> {
    NaiveTime::parse_from_str(raw.trim(), "%H:%M").map_err(|_| {
        invalid(format!(
            "„{key}“ muss eine Uhrzeit hh:mm sein (24 Stunden), z. B. „09:00“, nicht „{raw}“."
        ))
    })
}

/// Midnight at the start of `date` in `tz`.
pub fn midnight(date: NaiveDate, tz: Tz) -> DateTime<Utc> {
    Zone::Iana(tz).to_utc(date.and_time(NaiveTime::MIN))
}

/// A wall-clock time in `tz` as an instant.
pub fn local_to_utc(local: NaiveDateTime, tz: Tz) -> DateTime<Utc> {
    Zone::Iana(tz).to_utc(local)
}

/// RFC 3339 in `tz` (seconds, `Z` for UTC).
pub fn fmt_at(t: DateTime<Utc>, tz: Tz) -> String {
    t.with_timezone(&tz)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn fmt_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// A query range from `start`/`end`: a date as `start` is midnight in
/// `tz`, a date as `end` includes that whole day; instants are taken as
/// given (`end` exclusive). At most [`MAX_RANGE_DAYS`].
pub fn range(start: TimeArg, end: TimeArg, tz: Tz) -> ArgResult<(DateTime<Utc>, DateTime<Utc>)> {
    let from = match start {
        TimeArg::Date(d) => midnight(d, tz),
        TimeArg::At(t) => t.with_timezone(&Utc),
    };
    let to = match end {
        TimeArg::Date(d) => midnight(d + Duration::days(1), tz),
        TimeArg::At(t) => t.with_timezone(&Utc),
    };
    if to <= from {
        return Err(invalid(
            "end liegt nicht nach start. Ein Datum als end zählt ganz mit: start „2026-11-03“ und \
             end „2026-11-03“ ist der ganze 3. November.",
        ));
    }
    if (to - from).num_days() > MAX_RANGE_DAYS {
        return Err(invalid(format!(
            "Der Zeitraum ist zu lang (höchstens {MAX_RANGE_DAYS} Tage). Bitte in mehrere Abfragen \
             teilen."
        )));
    }
    Ok((from, to))
}

/// A time of a stored event or occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Date(NaiveDate),
    At(DateTime<Utc>),
}

impl When {
    /// The instant this starts at, all-day dates at midnight in `tz`.
    pub fn instant(self, tz: Tz) -> DateTime<Utc> {
        match self {
            When::Date(d) => midnight(d, tz),
            When::At(t) => t,
        }
    }
}

/// Reads a `DateValue` of the event JSON; floating times count in `tz`.
pub fn when_of(v: &Value, tz: Tz) -> Option<When> {
    if let Some(d) = v.get("date").and_then(Value::as_str) {
        return NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .map(When::Date);
    }
    if let Some(u) = v.get("utc").and_then(Value::as_str) {
        return DateTime::parse_from_rfc3339(u)
            .ok()
            .map(|t| When::At(t.with_timezone(&Utc)));
    }
    let local = v.get("date_time").and_then(Value::as_str)?;
    NaiveDateTime::parse_from_str(local, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|l| When::At(local_to_utc(l, tz)))
}

/// The zone a written time is stored in: the explicit `time_zone`, else
/// `existing` (the zone the event already has), else the server's zone if
/// its offset at that instant matches the given offset, else UTC.
pub fn pick_zone(explicit: Option<Tz>, existing: Option<Tz>, at: DateTime<FixedOffset>) -> Tz {
    if let Some(tz) = explicit.or(existing) {
        return tz;
    }
    let zone = default_zone();
    let utc = at.with_timezone(&Utc);
    if utc.with_timezone(&zone).offset().fix() == *at.offset() {
        zone
    } else {
        Tz::UTC
    }
}

/// A `DateValue` input of the write path for an instant in `zone`.
pub fn date_time_input(at: DateTime<FixedOffset>, zone: Tz) -> Value {
    let local = at.with_timezone(&zone).naive_local();
    json!({
        "date_time": local.format("%Y-%m-%dT%H:%M:%S").to_string(),
        "time_zone": if zone == Tz::UTC { "UTC" } else { zone.name() },
    })
}

pub fn date_input(d: NaiveDate) -> Value {
    json!({"date": fmt_date(d)})
}

/// The recurrence id of an occurrence as the write path takes it (UTC
/// instant or date).
pub fn occurrence_input(t: TimeArg) -> Value {
    match t {
        TimeArg::Date(d) => date_input(d),
        TimeArg::At(t) => date_time_input(t, Tz::UTC),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_need_an_offset() {
        assert!(matches!(
            parse_time("start", "2026-11-03").unwrap(),
            TimeArg::Date(_)
        ));
        let TimeArg::At(t) = parse_time("start", "2026-11-03T14:00:00+01:00").unwrap() else {
            panic!()
        };
        assert_eq!(
            fmt_at(t.with_timezone(&Utc), Tz::UTC),
            "2026-11-03T13:00:00Z"
        );
        assert!(parse_time("start", "2026-11-03T14:00+01:00").is_ok());
        let e = parse_time("start", "2026-11-03T14:00:00").unwrap_err();
        assert!(e.message.contains("keine Zeitzone"), "{}", e.message);
        assert!(e.message.contains("+01:00"), "{}", e.message);
        let e = parse_time("end", "morgen").unwrap_err();
        assert!(
            e.message.contains("kein gültiger Zeitpunkt"),
            "{}",
            e.message
        );
    }

    #[test]
    fn ranges_include_the_end_date() {
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        let d = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let (from, to) = range(TimeArg::Date(d), TimeArg::Date(d), berlin).unwrap();
        assert_eq!(fmt_at(from, Tz::UTC), "2026-11-02T23:00:00Z");
        assert_eq!(fmt_at(to, Tz::UTC), "2026-11-03T23:00:00Z");
        let late = NaiveDate::from_ymd_opt(2028, 1, 1).unwrap();
        assert!(range(TimeArg::Date(d), TimeArg::Date(late), berlin).is_err());
    }

    #[test]
    fn zones_follow_the_explicit_choice() {
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        let at = DateTime::parse_from_rfc3339("2026-11-03T14:00:00+01:00").unwrap();
        assert_eq!(pick_zone(Some(berlin), None, at), berlin);
        let v = date_time_input(at, berlin);
        assert_eq!(v["date_time"], "2026-11-03T14:00:00");
        assert_eq!(v["time_zone"], "Europe/Berlin");
        let v = date_time_input(at, Tz::UTC);
        assert_eq!(v["date_time"], "2026-11-03T13:00:00");
        assert_eq!(v["time_zone"], "UTC");
    }
}
