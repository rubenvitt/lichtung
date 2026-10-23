//! `RRULE` values: a lossless representation (unknown parts and part order
//! survive edits) and wall-clock expansion via the `rrule` crate.

use chrono::{Month, NaiveDateTime, TimeZone, Weekday};
use std::fmt;

use crate::error::{Error, Result};
use crate::value::{CalTime, TimeZoneRef};

/// Recurrence frequency (`FREQ`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Frequency {
    /// `SECONDLY`
    Secondly,
    /// `MINUTELY`
    Minutely,
    /// `HOURLY`
    Hourly,
    /// `DAILY`
    Daily,
    /// `WEEKLY`
    Weekly,
    /// `MONTHLY`
    Monthly,
    /// `YEARLY`
    Yearly,
}

impl Frequency {
    fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_uppercase().as_str() {
            "SECONDLY" => Self::Secondly,
            "MINUTELY" => Self::Minutely,
            "HOURLY" => Self::Hourly,
            "DAILY" => Self::Daily,
            "WEEKLY" => Self::Weekly,
            "MONTHLY" => Self::Monthly,
            "YEARLY" => Self::Yearly,
            _ => return None,
        })
    }

    /// The iCalendar keyword.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Secondly => "SECONDLY",
            Self::Minutely => "MINUTELY",
            Self::Hourly => "HOURLY",
            Self::Daily => "DAILY",
            Self::Weekly => "WEEKLY",
            Self::Monthly => "MONTHLY",
            Self::Yearly => "YEARLY",
        }
    }

    fn to_rrule(self) -> rrule::Frequency {
        match self {
            Self::Secondly => rrule::Frequency::Secondly,
            Self::Minutely => rrule::Frequency::Minutely,
            Self::Hourly => rrule::Frequency::Hourly,
            Self::Daily => rrule::Frequency::Daily,
            Self::Weekly => rrule::Frequency::Weekly,
            Self::Monthly => rrule::Frequency::Monthly,
            Self::Yearly => rrule::Frequency::Yearly,
        }
    }
}

/// A recurrence rule (`RRULE` value).
///
/// The rule keeps its parts as written (`KEY=VALUE` pairs in source order),
/// so that editing e.g. `UNTIL` leaves everything else byte-identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurRule {
    parts: Vec<(String, String)>,
}

impl RecurRule {
    /// Creates a rule with only a frequency.
    pub fn new(freq: Frequency) -> Self {
        Self {
            parts: vec![("FREQ".into(), freq.as_str().into())],
        }
    }

    /// Parses an `RRULE` value such as `FREQ=WEEKLY;BYDAY=MO,WE`.
    pub fn parse(value: &str) -> Result<Self> {
        let mut parts = Vec::new();
        for item in value.trim().split(';').filter(|s| !s.trim().is_empty()) {
            let (k, v) = item.split_once('=').ok_or_else(|| Error::InvalidValue {
                property: "RRULE".into(),
                value: value.into(),
                reason: format!("Teil „{item}“ ohne „=“"),
            })?;
            parts.push((k.trim().to_string(), v.trim().to_string()));
        }
        let rule = Self { parts };
        if rule.freq().is_none() {
            return Err(Error::InvalidValue {
                property: "RRULE".into(),
                value: value.into(),
                reason: "FREQ fehlt oder ist unbekannt".into(),
            });
        }
        Ok(rule)
    }

    /// Raw value of a part (case-insensitive key).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.parts
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// Sets or removes a part, keeping the position of an existing one.
    pub fn set(&mut self, key: &str, value: Option<String>) {
        match value {
            Some(v) => {
                if let Some(slot) = self
                    .parts
                    .iter_mut()
                    .find(|(k, _)| k.eq_ignore_ascii_case(key))
                {
                    slot.1 = v;
                } else {
                    self.parts.push((key.to_ascii_uppercase(), v));
                }
            }
            None => self.parts.retain(|(k, _)| !k.eq_ignore_ascii_case(key)),
        }
    }

    /// `FREQ`.
    pub fn freq(&self) -> Option<Frequency> {
        self.get("FREQ").and_then(Frequency::parse)
    }

    /// `INTERVAL` (default 1).
    pub fn interval(&self) -> u32 {
        self.get("INTERVAL")
            .and_then(|v| v.parse().ok())
            .filter(|v| *v > 0)
            .unwrap_or(1)
    }

    /// `COUNT`.
    pub fn count(&self) -> Option<u32> {
        self.get("COUNT").and_then(|v| v.parse().ok())
    }

    /// Sets `COUNT` (and removes `UNTIL`, which must not coexist).
    pub fn set_count(&mut self, count: Option<u32>) {
        if count.is_some() {
            self.set("UNTIL", None);
        }
        self.set("COUNT", count.map(|c| c.to_string()));
    }

    /// `UNTIL` as DATE, UTC date-time or (non-conforming) floating date-time.
    pub fn until(&self) -> Option<CalTime> {
        self.get("UNTIL")
            .and_then(|v| CalTime::parse(v, None, None).ok())
    }

    /// Sets `UNTIL` (and removes `COUNT`). A zoned value is not allowed by
    /// RFC 5545; pass UTC or DATE values.
    pub fn set_until(&mut self, until: Option<CalTime>) {
        if until.is_some() {
            self.set("COUNT", None);
        }
        self.set("UNTIL", until.map(|u| u.format_value()));
    }

    /// Whether the rule is unbounded (no `COUNT`, no `UNTIL`).
    pub fn is_infinite(&self) -> bool {
        self.get("COUNT").is_none() && self.get("UNTIL").is_none()
    }

    /// Builds the `rrule` crate representation without COUNT/UNTIL (those are
    /// applied by [`WallClockIter`], which needs time zone knowledge).
    fn to_rrule(&self) -> Result<rrule::RRule<rrule::Unvalidated>> {
        let bad = |reason: String| Error::InvalidValue {
            property: "RRULE".into(),
            value: self.to_string(),
            reason,
        };
        let freq = self
            .freq()
            .ok_or_else(|| bad("FREQ fehlt".into()))?
            .to_rrule();
        let mut r = rrule::RRule::new(freq).interval(self.interval().min(u16::MAX as u32) as u16);
        for (k, v) in &self.parts {
            let key = k.to_ascii_uppercase();
            let list = || v.split(',').map(str::trim).filter(|s| !s.is_empty());
            match key.as_str() {
                "FREQ" | "INTERVAL" | "COUNT" | "UNTIL" => {}
                "BYSETPOS" => r = r.by_set_pos(parse_list(list(), &key, &bad)?),
                "BYMONTH" => {
                    let months: Vec<u8> = parse_list(list(), &key, &bad)?;
                    let months: Vec<Month> = months
                        .into_iter()
                        .map(|m| Month::try_from(m).map_err(|_| bad(format!("BYMONTH={m}"))))
                        .collect::<Result<_>>()?;
                    r = r.by_month(&months);
                }
                "BYMONTHDAY" => r = r.by_month_day(parse_list(list(), &key, &bad)?),
                "BYYEARDAY" => r = r.by_year_day(parse_list(list(), &key, &bad)?),
                "BYWEEKNO" => r = r.by_week_no(parse_list(list(), &key, &bad)?),
                "BYHOUR" => r = r.by_hour(parse_list(list(), &key, &bad)?),
                "BYMINUTE" => r = r.by_minute(parse_list(list(), &key, &bad)?),
                "BYSECOND" => r = r.by_second(parse_list(list(), &key, &bad)?),
                "BYDAY" => {
                    let days = list()
                        .map(|d| parse_nweekday(d).ok_or_else(|| bad(format!("BYDAY={d}"))))
                        .collect::<Result<Vec<_>>>()?;
                    r = r.by_weekday(days);
                }
                "WKST" => {
                    let wd = parse_weekday(v).ok_or_else(|| bad(format!("WKST={v}")))?;
                    r = r.week_start(wd);
                }
                // Unknown or unsupported parts (RSCALE, SKIP, X-…) are kept in the
                // text but ignored for expansion.
                _ => {}
            }
        }
        Ok(r)
    }

    /// Expands the rule in pure wall-clock time starting at `dtstart`.
    ///
    /// `COUNT` is applied (DTSTART always counts as the first instance, as
    /// RFC 5545 requires, even if it does not match the rule); `UNTIL` is
    /// *not* applied because comparing it needs time zone information.
    pub fn wall_clock_iter(&self, dtstart: NaiveDateTime) -> Result<WallClockIter> {
        let start = rrule::Tz::UTC.from_utc_datetime(&dtstart);
        let set = self
            .to_rrule()?
            .build(start)
            .map_err(|e| Error::InvalidValue {
                property: "RRULE".into(),
                value: self.to_string(),
                reason: e.to_string(),
            })?;
        // `limit()` makes the crate give up after 100 000 empty iterations,
        // which protects against rules that never match (e.g. 30 February).
        Ok(WallClockIter {
            inner: set.limit().into_iter(),
            dtstart,
            emitted_start: false,
            remaining: self.count(),
            pending: None,
        })
    }
}

impl fmt::Display for RecurRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self
            .parts
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(";");
        f.write_str(&s)
    }
}

/// Iterator over wall-clock occurrence starts (see
/// [`RecurRule::wall_clock_iter`]).
pub struct WallClockIter {
    inner: rrule::RRuleSetIter,
    dtstart: NaiveDateTime,
    emitted_start: bool,
    remaining: Option<u32>,
    pending: Option<NaiveDateTime>,
}

impl Iterator for WallClockIter {
    type Item = NaiveDateTime;

    fn next(&mut self) -> Option<NaiveDateTime> {
        if self.remaining == Some(0) {
            return None;
        }
        let next = if !self.emitted_start {
            self.emitted_start = true;
            let first = self.inner.next().map(|d| d.naive_utc());
            if first != Some(self.dtstart) {
                self.pending = first;
            }
            Some(self.dtstart)
        } else if let Some(p) = self.pending.take() {
            Some(p)
        } else {
            self.inner.next().map(|d| d.naive_utc())
        };
        if next.is_some() {
            if let Some(r) = self.remaining.as_mut() {
                *r -= 1;
            }
        }
        next
    }
}

fn parse_list<'a, T: std::str::FromStr>(
    items: impl Iterator<Item = &'a str>,
    key: &str,
    bad: &dyn Fn(String) -> Error,
) -> Result<Vec<T>> {
    items
        .map(|s| {
            s.trim_start_matches('+')
                .parse::<T>()
                .map_err(|_| bad(format!("{key}={s}")))
        })
        .collect()
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    Some(match s.trim().to_ascii_uppercase().as_str() {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

fn parse_nweekday(s: &str) -> Option<rrule::NWeekday> {
    let s = s.trim();
    if s.len() < 2 {
        return None;
    }
    let (num, day) = s.split_at(s.len() - 2);
    let wd = parse_weekday(day)?;
    if num.is_empty() {
        Some(rrule::NWeekday::Every(wd))
    } else {
        let n: i16 = num.trim_start_matches('+').parse().ok()?;
        if n == 0 {
            return None;
        }
        Some(rrule::NWeekday::Nth(n, wd))
    }
}

/// Converts an `UNTIL` value that should be compared against a zoned series
/// into a predicate on wall-clock values, given a function that maps wall
/// clock to UTC wall clock. Used by expansion and editing.
pub(crate) fn until_allows(
    until: &CalTime,
    candidate_wall: NaiveDateTime,
    to_utc: &dyn Fn(NaiveDateTime) -> NaiveDateTime,
    series_is_date: bool,
) -> bool {
    match until {
        CalTime::Date(d) => candidate_wall.date() <= *d,
        CalTime::DateTime { local, zone } => {
            if series_is_date {
                // UNTIL as date-time on an all-day series: compare dates.
                return candidate_wall.date() <= local.date();
            }
            match zone {
                TimeZoneRef::Utc => to_utc(candidate_wall) <= *local,
                // Non-conforming floating/zoned UNTIL: compare wall clock.
                _ => candidate_wall <= *local,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn dt(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    #[test]
    fn keeps_unknown_parts_and_order() {
        let mut r =
            RecurRule::parse("FREQ=WEEKLY;X-FOO=bar;BYDAY=MO;UNTIL=20261231T230000Z").unwrap();
        r.set_until(Some(CalTime::utc(dt(2026, 11, 30, 23))));
        assert_eq!(
            r.to_string(),
            "FREQ=WEEKLY;X-FOO=bar;BYDAY=MO;UNTIL=20261130T230000Z"
        );
        r.set_count(Some(4));
        assert_eq!(r.to_string(), "FREQ=WEEKLY;X-FOO=bar;BYDAY=MO;COUNT=4");
    }

    #[test]
    fn dtstart_counts_even_if_unsynchronized() {
        // 2026-01-31 is a Saturday; the rule only yields Mondays.
        let r = RecurRule::parse("FREQ=WEEKLY;BYDAY=MO;COUNT=3").unwrap();
        let v: Vec<_> = r.wall_clock_iter(dt(2026, 1, 31, 9)).unwrap().collect();
        assert_eq!(
            v,
            vec![dt(2026, 1, 31, 9), dt(2026, 2, 2, 9), dt(2026, 2, 9, 9)]
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(RecurRule::parse("BYDAY=MO").is_err());
        assert!(RecurRule::parse("FREQ=DAILY;COUNT").is_err());
        let r = RecurRule::parse("FREQ=MONTHLY;BYDAY=XX").unwrap();
        assert!(r.wall_clock_iter(dt(2026, 1, 1, 9)).is_err());
    }
}
