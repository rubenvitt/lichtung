//! Typed iCalendar values: DATE / DATE-TIME (with zone reference), DURATION
//! and PERIOD.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use std::fmt;

use crate::error::{Error, Result};
use crate::parser::Property;

/// How a DATE-TIME value is anchored in time.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TimeZoneRef {
    /// UTC (`…Z` suffix).
    Utc,
    /// Floating time: no zone; interpreted in a caller-supplied default zone.
    Floating,
    /// Local time in the zone named by the `TZID` parameter (raw TZID text).
    Tzid(String),
}

/// A DATE or DATE-TIME value as it appears in `DTSTART`, `DTEND`, `EXDATE`,
/// `RDATE` or `RECURRENCE-ID`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CalTime {
    /// A calendar date (`VALUE=DATE`), used for all-day events.
    Date(NaiveDate),
    /// A wall-clock date and time plus its anchoring.
    DateTime {
        /// Wall-clock value as written in the property.
        local: NaiveDateTime,
        /// Zone reference.
        zone: TimeZoneRef,
    },
}

impl CalTime {
    /// A UTC date-time.
    pub fn utc(local: NaiveDateTime) -> Self {
        CalTime::DateTime {
            local,
            zone: TimeZoneRef::Utc,
        }
    }

    /// A floating date-time.
    pub fn floating(local: NaiveDateTime) -> Self {
        CalTime::DateTime {
            local,
            zone: TimeZoneRef::Floating,
        }
    }

    /// A date-time in a named zone.
    pub fn zoned(local: NaiveDateTime, tzid: impl Into<String>) -> Self {
        CalTime::DateTime {
            local,
            zone: TimeZoneRef::Tzid(tzid.into()),
        }
    }

    /// Whether this is a DATE value (all-day).
    pub fn is_date(&self) -> bool {
        matches!(self, CalTime::Date(_))
    }

    /// The calendar date part.
    pub fn date(&self) -> NaiveDate {
        match self {
            CalTime::Date(d) => *d,
            CalTime::DateTime { local, .. } => local.date(),
        }
    }

    /// The wall-clock value (midnight for DATE values).
    pub fn naive(&self) -> NaiveDateTime {
        match self {
            CalTime::Date(d) => d.and_time(NaiveTime::MIN),
            CalTime::DateTime { local, .. } => *local,
        }
    }

    /// The zone reference (`None` for DATE values).
    pub fn zone(&self) -> Option<&TimeZoneRef> {
        match self {
            CalTime::Date(_) => None,
            CalTime::DateTime { zone, .. } => Some(zone),
        }
    }

    /// The TZID if this is a zoned date-time.
    pub fn tzid(&self) -> Option<&str> {
        match self.zone() {
            Some(TimeZoneRef::Tzid(t)) => Some(t),
            _ => None,
        }
    }

    /// Returns a copy with another wall-clock value but the same kind/zone.
    /// For DATE values only the date part is used.
    pub fn with_naive(&self, naive: NaiveDateTime) -> Self {
        match self {
            CalTime::Date(_) => CalTime::Date(naive.date()),
            CalTime::DateTime { zone, .. } => CalTime::DateTime {
                local: naive,
                zone: zone.clone(),
            },
        }
    }

    /// Parses a single value. `tzid` is the `TZID` parameter, `value_type`
    /// the `VALUE` parameter (if any).
    pub fn parse(value: &str, tzid: Option<&str>, value_type: Option<&str>) -> Result<Self> {
        let v = value.trim();
        let invalid = |reason: &str| Error::InvalidValue {
            property: "DATE/DATE-TIME".into(),
            value: value.into(),
            reason: reason.into(),
        };
        let is_date_type = value_type.is_some_and(|t| t.eq_ignore_ascii_case("DATE"));
        if is_date_type || (v.len() == 8 && !v.contains('T')) {
            let d = NaiveDate::parse_from_str(&v[..v.len().min(8)], "%Y%m%d")
                .map_err(|_| invalid("Datum im Format JJJJMMTT erwartet"))?;
            return Ok(CalTime::Date(d));
        }
        let (body, utc) = match v.strip_suffix('Z').or_else(|| v.strip_suffix('z')) {
            Some(b) => (b, true),
            None => (v, false),
        };
        let local = NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S")
            .or_else(|_| NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M"))
            .map_err(|_| invalid("Zeitpunkt im Format JJJJMMTTThhmmss erwartet"))?;
        let zone = if utc {
            TimeZoneRef::Utc
        } else {
            match tzid {
                Some(t) if !t.trim().is_empty() => TimeZoneRef::Tzid(t.to_string()),
                _ => TimeZoneRef::Floating,
            }
        };
        Ok(CalTime::DateTime { local, zone })
    }

    /// Parses the (single) value of a property such as `DTSTART`.
    pub fn from_property(p: &Property) -> Result<Self> {
        let first = p.value().split(',').next().unwrap_or("");
        CalTime::parse(first, p.param("TZID"), p.param("VALUE")).map_err(|e| rename(e, p))
    }

    /// Parses a comma-separated list value (e.g. `EXDATE`).
    pub fn list_from_property(p: &Property) -> Result<Vec<Self>> {
        p.value()
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| CalTime::parse(s, p.param("TZID"), p.param("VALUE")).map_err(|e| rename(e, p)))
            .collect()
    }

    /// The value text without parameters (`20261024`, `20261024T090000`,
    /// `20261024T070000Z`).
    pub fn format_value(&self) -> String {
        match self {
            CalTime::Date(d) => d.format("%Y%m%d").to_string(),
            CalTime::DateTime { local, zone } => {
                let mut s = local.format("%Y%m%dT%H%M%S").to_string();
                if *zone == TimeZoneRef::Utc {
                    s.push('Z');
                }
                s
            }
        }
    }

    /// Builds a property with the right `VALUE`/`TZID` parameters.
    pub fn to_property(&self, name: &str) -> Property {
        Self::list_to_property(name, std::slice::from_ref(self))
    }

    /// Builds a multi-valued property. All values must share kind and zone;
    /// the first value decides the parameters.
    pub fn list_to_property(name: &str, values: &[CalTime]) -> Property {
        let joined = values
            .iter()
            .map(CalTime::format_value)
            .collect::<Vec<_>>()
            .join(",");
        let mut p = Property::new(name, joined);
        match values.first() {
            Some(CalTime::Date(_)) => p.set_param("VALUE", "DATE"),
            Some(CalTime::DateTime {
                zone: TimeZoneRef::Tzid(t),
                ..
            }) => p.set_param("TZID", t.clone()),
            _ => {}
        }
        p
    }
}

impl fmt::Display for CalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.tzid() {
            Some(t) => write!(f, "{} ({t})", self.format_value()),
            None => f.write_str(&self.format_value()),
        }
    }
}

fn rename(e: Error, p: &Property) -> Error {
    match e {
        Error::InvalidValue { value, reason, .. } => Error::InvalidValue {
            property: p.name().to_ascii_uppercase(),
            value,
            reason,
        },
        other => other,
    }
}

/// An iCalendar DURATION (RFC 5545 §3.3.6).
///
/// Weeks and days are *nominal* (added in wall-clock time, a day may have
/// 23 or 25 hours), hours/minutes/seconds are *exact*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct IcalDuration {
    /// Negative duration (`-P…`), e.g. alarms before the start.
    pub negative: bool,
    /// Weeks.
    pub weeks: u32,
    /// Days.
    pub days: u32,
    /// Hours.
    pub hours: u32,
    /// Minutes.
    pub minutes: u32,
    /// Seconds.
    pub seconds: u32,
}

impl IcalDuration {
    /// Builds a duration from a signed number of nominal days and exact seconds.
    pub fn from_parts(days: i64, seconds: i64) -> Self {
        let negative = days < 0 || (days == 0 && seconds < 0);
        let days = days.unsigned_abs();
        let secs = seconds.unsigned_abs();
        Self {
            negative,
            weeks: 0,
            days: days as u32,
            hours: (secs / 3600) as u32,
            minutes: ((secs % 3600) / 60) as u32,
            seconds: (secs % 60) as u32,
        }
    }

    /// Builds an exact duration from seconds.
    pub fn from_seconds(seconds: i64) -> Self {
        Self::from_parts(0, seconds)
    }

    /// Signed nominal days (weeks × 7 + days).
    pub fn nominal_days(&self) -> i64 {
        let d = i64::from(self.weeks) * 7 + i64::from(self.days);
        if self.negative {
            -d
        } else {
            d
        }
    }

    /// Signed exact seconds (hours, minutes, seconds).
    pub fn exact_seconds(&self) -> i64 {
        let s =
            i64::from(self.hours) * 3600 + i64::from(self.minutes) * 60 + i64::from(self.seconds);
        if self.negative {
            -s
        } else {
            s
        }
    }

    /// Total length in seconds, counting a nominal day as 86 400 s.
    pub fn approx_seconds(&self) -> i64 {
        self.nominal_days() * 86_400 + self.exact_seconds()
    }

    /// Parses `P1W`, `-PT15M`, `P1DT2H`, …
    pub fn parse(value: &str) -> Result<Self> {
        let invalid = || Error::InvalidValue {
            property: "DURATION".into(),
            value: value.into(),
            reason: "Dauer im Format P[n]DT[n]H[n]M[n]S erwartet".into(),
        };
        let mut s = value.trim();
        let mut d = IcalDuration::default();
        if let Some(rest) = s.strip_prefix('-') {
            d.negative = true;
            s = rest;
        } else if let Some(rest) = s.strip_prefix('+') {
            s = rest;
        }
        let s = s
            .strip_prefix('P')
            .or_else(|| s.strip_prefix('p'))
            .ok_or_else(invalid)?;
        let mut in_time = false;
        let mut num = String::new();
        let mut any = false;
        for c in s.chars() {
            match c.to_ascii_uppercase() {
                '0'..='9' => num.push(c),
                'T' => in_time = true,
                unit => {
                    let n: u32 = num.parse().map_err(|_| invalid())?;
                    num.clear();
                    any = true;
                    match (unit, in_time) {
                        ('W', false) => d.weeks = n,
                        ('D', false) => d.days = n,
                        ('H', true) => d.hours = n,
                        ('M', true) => d.minutes = n,
                        ('S', true) => d.seconds = n,
                        _ => return Err(invalid()),
                    }
                }
            }
        }
        if !any || !num.is_empty() {
            return Err(invalid());
        }
        Ok(d)
    }
}

impl fmt::Display for IcalDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negative {
            f.write_str("-")?;
        }
        f.write_str("P")?;
        let has_time = self.hours > 0 || self.minutes > 0 || self.seconds > 0;
        if self.weeks > 0 && self.days == 0 && !has_time {
            return write!(f, "{}W", self.weeks);
        }
        let days = self.weeks * 7 + self.days;
        if days > 0 {
            write!(f, "{days}D")?;
        }
        if has_time || days == 0 {
            f.write_str("T")?;
            if self.hours > 0 {
                write!(f, "{}H", self.hours)?;
            }
            if self.minutes > 0 {
                write!(f, "{}M", self.minutes)?;
            }
            if self.seconds > 0 || !has_time {
                write!(f, "{}S", self.seconds)?;
            }
        }
        Ok(())
    }
}

/// End of a PERIOD value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeriodEnd {
    /// Explicit end.
    End(CalTime),
    /// Duration from the start.
    Duration(IcalDuration),
}

/// A PERIOD value (`start/end` or `start/duration`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Period {
    /// Start.
    pub start: CalTime,
    /// End or duration.
    pub end: PeriodEnd,
}

/// One entry of an `RDATE` property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RDate {
    /// A DATE or DATE-TIME.
    Time(CalTime),
    /// A PERIOD; its length replaces the event's default duration.
    Period(Period),
}

impl RDate {
    /// The start of this recurrence date.
    pub fn start(&self) -> &CalTime {
        match self {
            RDate::Time(t) => t,
            RDate::Period(p) => &p.start,
        }
    }

    /// Parses all values of an `RDATE` property.
    pub fn list_from_property(p: &Property) -> Result<Vec<RDate>> {
        let tzid = p.param("TZID");
        let is_period = p
            .param("VALUE")
            .is_some_and(|v| v.eq_ignore_ascii_case("PERIOD"));
        let mut out = Vec::new();
        for item in p.value().split(',').filter(|s| !s.trim().is_empty()) {
            if is_period || item.contains('/') {
                let (a, b) = item.split_once('/').ok_or_else(|| Error::InvalidValue {
                    property: "RDATE".into(),
                    value: item.into(),
                    reason: "Zeitraum im Format Beginn/Ende erwartet".into(),
                })?;
                let start = CalTime::parse(a, tzid, None)?;
                let b = b.trim();
                let end = if b.starts_with(['P', 'p', '-', '+']) {
                    PeriodEnd::Duration(IcalDuration::parse(b)?)
                } else {
                    PeriodEnd::End(CalTime::parse(b, tzid, None)?)
                };
                out.push(RDate::Period(Period { start, end }));
            } else {
                out.push(RDate::Time(CalTime::parse(item, tzid, p.param("VALUE"))?));
            }
        }
        Ok(out)
    }

    /// Formats the value text of this entry (without parameters).
    pub fn format_value(&self) -> String {
        match self {
            RDate::Time(t) => t.format_value(),
            RDate::Period(p) => match &p.end {
                PeriodEnd::End(e) => format!("{}/{}", p.start.format_value(), e.format_value()),
                PeriodEnd::Duration(d) => format!("{}/{}", p.start.format_value(), d),
            },
        }
    }

    /// Builds an `RDATE` property from entries sharing kind and zone.
    pub fn list_to_property(values: &[RDate]) -> Property {
        let joined = values
            .iter()
            .map(RDate::format_value)
            .collect::<Vec<_>>()
            .join(",");
        let mut p = Property::new("RDATE", joined);
        if let Some(first) = values.first() {
            if matches!(first, RDate::Period(_)) {
                p.set_param("VALUE", "PERIOD");
            } else if first.start().is_date() {
                p.set_param("VALUE", "DATE");
            }
            if let Some(t) = first.start().tzid() {
                p.set_param("TZID", t.to_string());
            }
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        for (input, days, secs, out) in [
            ("PT1H30M", 0, 5400, "PT1H30M"),
            ("-PT15M", 0, -900, "-PT15M"),
            ("P1W", 7, 0, "P1W"),
            ("P1DT12H", 1, 43200, "P1DT12H"),
            ("PT0S", 0, 0, "PT0S"),
            ("P2D", 2, 0, "P2D"),
        ] {
            let d = IcalDuration::parse(input).unwrap();
            assert_eq!(d.nominal_days(), days, "{input}");
            assert_eq!(d.exact_seconds(), secs, "{input}");
            assert_eq!(d.to_string(), out);
        }
        assert!(IcalDuration::parse("1H").is_err());
        assert!(IcalDuration::parse("PT").is_err());
    }

    #[test]
    fn caltime_parse() {
        assert!(CalTime::parse("20261024", None, None).unwrap().is_date());
        assert_eq!(
            CalTime::parse("20261024T090000Z", Some("Europe/Berlin"), None)
                .unwrap()
                .zone(),
            Some(&TimeZoneRef::Utc)
        );
        assert_eq!(
            CalTime::parse("20261024T090000", None, None)
                .unwrap()
                .zone(),
            Some(&TimeZoneRef::Floating)
        );
        assert!(CalTime::parse("2026-10-24", None, None).is_err());
    }
}
