//! Time zone resolution.
//!
//! A `TZID` is resolved in this order:
//!
//! 1. IANA name known to `chrono-tz` (`Europe/Berlin`),
//! 2. Windows zone name (`W. Europe Standard Time`) or Outlook display name
//!    (`(UTC+01:00) Amsterdam, Berlin, …`) via a mapping table,
//! 3. an IANA name embedded in a path (`/mozilla.org/20050126_1/Europe/Berlin`),
//! 4. the calendar's own `VTIMEZONE` with that TZID (offsets evaluated from
//!    its `STANDARD`/`DAYLIGHT` observances incl. `RRULE`/`RDATE`),
//! 5. a fixed UTC offset found in the name (`(UTC+05:30) …`, `GMT+0200`),
//! 6. the caller's default zone (reported as [`ResolvedBy::Fallback`]).
//!
//! Resolution never panics; the worst case is the fallback.

use chrono::{DateTime, Datelike, Duration, NaiveDateTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use std::str::FromStr;
use std::sync::Arc;

use crate::parser::Component;
use crate::rule::RecurRule;
use crate::value::{CalTime, TimeZoneRef};
use crate::windows_zones::{OUTLOOK_DISPLAY_TO_IANA, WINDOWS_TO_IANA};

/// Last year for which custom `VTIMEZONE` transitions are precomputed.
const CUSTOM_ZONE_LAST_YEAR: i32 = 2200;
/// Safety cap for transitions per observance.
const CUSTOM_ZONE_MAX_TRANSITIONS: usize = 4000;

/// How a TZID was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBy {
    /// Exact IANA name.
    Iana,
    /// Windows or Outlook zone name via the mapping table.
    Windows,
    /// IANA name extracted from a path-like TZID.
    Prefix,
    /// Offsets computed from the calendar's own `VTIMEZONE`.
    VTimezone,
    /// Fixed offset parsed from the TZID text (no DST).
    FixedOffset,
    /// Unknown TZID: the default zone was used.
    Fallback,
}

/// A resolved time zone.
#[derive(Debug, Clone)]
pub enum Zone {
    /// A zone from the IANA database.
    Iana(Tz),
    /// A zone defined by a `VTIMEZONE` component.
    Custom(Arc<CustomZone>),
}

impl Zone {
    /// Display name: IANA name or the custom TZID.
    pub fn name(&self) -> String {
        match self {
            Zone::Iana(tz) => tz.name().to_string(),
            Zone::Custom(c) => c.tzid.clone(),
        }
    }

    /// UTC offset in seconds at the given UTC instant.
    pub fn offset_at(&self, utc: NaiveDateTime) -> i32 {
        match self {
            Zone::Iana(tz) => tz.offset_from_utc_datetime(&utc).fix().local_minus_utc(),
            Zone::Custom(c) => c.offset_at(utc),
        }
    }

    /// Converts wall-clock time to an instant following RFC 5545 §3.3.5:
    /// a time inside a DST gap is interpreted with the offset *before* the
    /// transition (so 02:30 on a spring-forward night becomes 03:30), an
    /// ambiguous time (fall back) maps to its first occurrence.
    pub fn to_utc(&self, local: NaiveDateTime) -> DateTime<Utc> {
        let before = self.offset_at(local - Duration::days(1));
        let after = self.offset_at(local + Duration::days(1));
        let mut valid: Vec<NaiveDateTime> = [before, after]
            .into_iter()
            .map(|o| local - Duration::seconds(i64::from(o)))
            .filter(|u| i64::from(self.offset_at(*u)) == (local - *u).num_seconds())
            .collect();
        valid.sort();
        let utc = valid
            .first()
            .copied()
            .unwrap_or_else(|| local - Duration::seconds(i64::from(before)));
        Utc.from_utc_datetime(&utc)
    }

    /// Converts an instant to wall-clock time in this zone.
    pub fn to_local(&self, utc: DateTime<Utc>) -> NaiveDateTime {
        let naive = utc.naive_utc();
        naive + Duration::seconds(i64::from(self.offset_at(naive)))
    }
}

/// A zone built from a `VTIMEZONE` component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomZone {
    tzid: String,
    initial_offset: i32,
    /// `(utc instant of transition, offset after it)`, sorted.
    transitions: Vec<(NaiveDateTime, i32)>,
}

impl CustomZone {
    /// Builds the zone from a `VTIMEZONE`. Returns `None` if it has no usable
    /// observance.
    pub fn from_vtimezone(vtz: &Component) -> Option<Self> {
        let tzid = vtz.property("TZID")?.value().to_string();
        let mut transitions = Vec::new();
        let mut first: Option<(NaiveDateTime, i32)> = None;
        for obs in vtz.components() {
            if !(obs.is("STANDARD") || obs.is("DAYLIGHT")) {
                continue;
            }
            let Some(from) = obs
                .property("TZOFFSETFROM")
                .and_then(|p| parse_offset(p.value()))
            else {
                continue;
            };
            let Some(to) = obs
                .property("TZOFFSETTO")
                .and_then(|p| parse_offset(p.value()))
            else {
                continue;
            };
            let Some(start) = obs
                .property("DTSTART")
                .and_then(|p| CalTime::from_property(p).ok())
                .map(|t| t.naive())
            else {
                continue;
            };
            let mut onsets = vec![start];
            if let Some(rule) = obs
                .property("RRULE")
                .and_then(|p| RecurRule::parse(p.value()).ok())
            {
                if let Ok(iter) = rule.wall_clock_iter(start) {
                    let until = rule.until();
                    for onset in iter
                        .skip(1)
                        .take_while(|o| o.year() <= CUSTOM_ZONE_LAST_YEAR)
                        .take(CUSTOM_ZONE_MAX_TRANSITIONS)
                    {
                        if let Some(u) = &until {
                            let utc = onset - Duration::seconds(i64::from(from));
                            let ok = match u {
                                CalTime::Date(d) => onset.date() <= *d,
                                CalTime::DateTime {
                                    local,
                                    zone: TimeZoneRef::Utc,
                                } => utc <= *local,
                                CalTime::DateTime { local, .. } => onset <= *local,
                            };
                            if !ok {
                                break;
                            }
                        }
                        onsets.push(onset);
                    }
                }
            }
            for p in obs.properties_named("RDATE") {
                if let Ok(list) = CalTime::list_from_property(p) {
                    onsets.extend(list.iter().map(CalTime::naive));
                }
            }
            for onset in onsets {
                let utc = onset - Duration::seconds(i64::from(from));
                if first.is_none_or(|(f, _)| utc < f) {
                    first = Some((utc, from));
                }
                transitions.push((utc, to));
            }
        }
        let (_, initial_offset) = first?;
        transitions.sort();
        transitions.dedup_by_key(|t| t.0);
        Some(Self {
            tzid,
            initial_offset,
            transitions,
        })
    }

    /// The TZID of the `VTIMEZONE`.
    pub fn tzid(&self) -> &str {
        &self.tzid
    }

    fn offset_at(&self, utc: NaiveDateTime) -> i32 {
        match self.transitions.partition_point(|(t, _)| *t <= utc) {
            0 => self.initial_offset,
            i => self.transitions[i - 1].1,
        }
    }
}

/// Parses `+0100`, `-0500`, `+013000`.
fn parse_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => (1, s),
    };
    if !(rest.len() == 4 || rest.len() == 6) || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = rest[0..2].parse().ok()?;
    let m: i32 = rest[2..4].parse().ok()?;
    let sec: i32 = if rest.len() == 6 {
        rest[4..6].parse().ok()?
    } else {
        0
    };
    Some(sign * (h * 3600 + m * 60 + sec))
}

/// Resolves a TZID without calendar context (steps 1–3 of the module docs).
pub fn resolve_known_tzid(tzid: &str) -> Option<(Tz, ResolvedBy)> {
    let t = tzid.trim().trim_matches('"').trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(tz) = Tz::from_str(t) {
        return Some((tz, ResolvedBy::Iana));
    }
    if let Some((_, iana)) = WINDOWS_TO_IANA
        .iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(t))
    {
        return Tz::from_str(iana).ok().map(|tz| (tz, ResolvedBy::Windows));
    }
    let display = strip_utc_prefix(t);
    if let Some((_, iana)) = OUTLOOK_DISPLAY_TO_IANA
        .iter()
        .find(|(d, _)| d.eq_ignore_ascii_case(display))
    {
        return Tz::from_str(iana).ok().map(|tz| (tz, ResolvedBy::Windows));
    }
    if t.contains('/') {
        let segments: Vec<&str> = t.split('/').filter(|s| !s.is_empty()).collect();
        for i in 0..segments.len() {
            let candidate = segments[i..].join("/");
            if let Ok(tz) = Tz::from_str(&candidate) {
                if i > 0 {
                    return Some((tz, ResolvedBy::Prefix));
                }
            }
        }
    }
    None
}

/// Removes a leading `(UTC+01:00) ` / `(GMT+01:00) ` from an Outlook name.
fn strip_utc_prefix(t: &str) -> &str {
    if t.starts_with('(') {
        if let Some(end) = t.find(')') {
            return t[end + 1..].trim();
        }
    }
    t
}

/// Extracts a fixed offset like `UTC+05:30`, `GMT-0300` from a TZID.
fn fixed_offset_from_name(t: &str) -> Option<i32> {
    let upper = t.to_ascii_uppercase();
    let idx = upper.find("UTC").or_else(|| upper.find("GMT"))?;
    let rest = &t[idx + 3..];
    let sign = match rest.chars().next()? {
        '+' => 1,
        '-' | '\u{2212}' => -1,
        _ => return None,
    };
    let digits: String = rest
        .chars()
        .skip(1)
        .take_while(|c| c.is_ascii_digit() || *c == ':')
        .filter(char::is_ascii_digit)
        .collect();
    let (h, m) = match digits.len() {
        1 | 2 => (digits.parse::<i32>().ok()?, 0),
        3 => (
            digits[..1].parse::<i32>().ok()?,
            digits[1..].parse::<i32>().ok()?,
        ),
        4 => (
            digits[..2].parse::<i32>().ok()?,
            digits[2..].parse::<i32>().ok()?,
        ),
        _ => return None,
    };
    if h > 14 || m > 59 {
        return None;
    }
    Some(sign * (h * 3600 + m * 60))
}

/// Resolves TZIDs and wall-clock values for one calendar object.
#[derive(Debug, Clone)]
pub struct ZoneResolver {
    default: Tz,
    custom: Vec<Arc<CustomZone>>,
}

impl ZoneResolver {
    /// A resolver without `VTIMEZONE` definitions. `default` is used for
    /// floating times, all-day dates and unknown TZIDs.
    pub fn new(default: Tz) -> Self {
        Self {
            default,
            custom: Vec::new(),
        }
    }

    /// A resolver that also knows all `VTIMEZONE`s inside `calendar`
    /// (usually a `VCALENDAR`).
    pub fn for_calendar(calendar: &Component, default: Tz) -> Self {
        let mut r = Self::new(default);
        for c in calendar.components_named("VTIMEZONE") {
            r.add_vtimezone(c);
        }
        r
    }

    /// Registers a `VTIMEZONE` definition.
    pub fn add_vtimezone(&mut self, vtz: &Component) {
        if let Some(z) = CustomZone::from_vtimezone(vtz) {
            self.custom.push(Arc::new(z));
        }
    }

    /// The default zone.
    pub fn default_zone(&self) -> Tz {
        self.default
    }

    /// Resolves a TZID (see the module docs for the order).
    pub fn resolve(&self, tzid: &str) -> (Zone, ResolvedBy) {
        if let Some((tz, how)) = resolve_known_tzid(tzid) {
            return (Zone::Iana(tz), how);
        }
        let wanted = tzid.trim().trim_matches('"');
        if let Some(c) = self.custom.iter().find(|c| c.tzid == wanted) {
            return (Zone::Custom(c.clone()), ResolvedBy::VTimezone);
        }
        if let Some(offset) = fixed_offset_from_name(wanted) {
            if offset % 3600 == 0 {
                // Etc/GMT zones have inverted signs.
                let name = match -offset / 3600 {
                    0 => "Etc/UTC".to_string(),
                    h if h > 0 => format!("Etc/GMT+{h}"),
                    h => format!("Etc/GMT{h}"),
                };
                if let Ok(tz) = Tz::from_str(&name) {
                    return (Zone::Iana(tz), ResolvedBy::FixedOffset);
                }
            } else {
                let zone = CustomZone {
                    tzid: wanted.to_string(),
                    initial_offset: offset,
                    transitions: Vec::new(),
                };
                return (Zone::Custom(Arc::new(zone)), ResolvedBy::FixedOffset);
            }
        }
        (Zone::Iana(self.default), ResolvedBy::Fallback)
    }

    /// The zone a value is interpreted in: UTC, the resolved TZID, or the
    /// default zone (floating times and dates).
    pub fn zone_of(&self, t: &CalTime) -> Zone {
        match t.zone() {
            Some(TimeZoneRef::Utc) => Zone::Iana(Tz::UTC),
            Some(TimeZoneRef::Tzid(id)) => self.resolve(id).0,
            Some(TimeZoneRef::Floating) | None => Zone::Iana(self.default),
        }
    }

    /// The instant of a value (dates: local midnight in the default zone).
    pub fn to_utc(&self, t: &CalTime) -> DateTime<Utc> {
        self.zone_of(t).to_utc(t.naive())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn ndt(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap()
    }

    #[test]
    fn windows_table_entries_are_valid_iana() {
        for (w, iana) in WINDOWS_TO_IANA.iter().chain(OUTLOOK_DISPLAY_TO_IANA) {
            assert!(Tz::from_str(iana).is_ok(), "{w} -> {iana}");
        }
    }

    #[test]
    fn resolves_names() {
        let cases = [
            ("Europe/Berlin", Some(("Europe/Berlin", ResolvedBy::Iana))),
            (
                "W. Europe Standard Time",
                Some(("Europe/Berlin", ResolvedBy::Windows)),
            ),
            (
                "\"Eastern Standard Time\"",
                Some(("America/New_York", ResolvedBy::Windows)),
            ),
            (
                "(UTC+01:00) Amsterdam, Berlin, Bern, Rome, Stockholm, Vienna",
                Some(("Europe/Berlin", ResolvedBy::Windows)),
            ),
            (
                "/mozilla.org/20050126_1/Europe/Berlin",
                Some(("Europe/Berlin", ResolvedBy::Prefix)),
            ),
            (
                "/softwarestudio.org/Olson_20011030_5/America/Argentina/Buenos_Aires",
                Some(("America/Argentina/Buenos_Aires", ResolvedBy::Prefix)),
            ),
            ("Lichtung Testzone", None),
        ];
        for (input, expected) in cases {
            let got = resolve_known_tzid(input).map(|(tz, how)| (tz.name(), how));
            assert_eq!(got, expected, "{input}");
        }
    }

    #[test]
    fn gap_and_overlap_follow_rfc5545() {
        let z = Zone::Iana(chrono_tz::Europe::Berlin);
        // Spring forward 2026-03-29 02:00 -> 03:00.
        assert_eq!(
            z.to_utc(ndt(2026, 3, 29, 2, 30)).naive_utc(),
            ndt(2026, 3, 29, 1, 30)
        );
        // Fall back 2026-10-25 03:00 -> 02:00; 02:30 happens twice, take the first.
        assert_eq!(
            z.to_utc(ndt(2026, 10, 25, 2, 30)).naive_utc(),
            ndt(2026, 10, 25, 0, 30)
        );
        assert_eq!(
            z.to_utc(ndt(2026, 7, 1, 9, 0)).naive_utc(),
            ndt(2026, 7, 1, 7, 0)
        );
    }

    #[test]
    fn fixed_offset_names() {
        let r = ZoneResolver::new(chrono_tz::UTC);
        let (z, how) = r.resolve("(UTC+05:30) Irgendwo");
        assert_eq!(how, ResolvedBy::FixedOffset);
        assert_eq!(
            z.to_utc(ndt(2026, 1, 1, 12, 0)).naive_utc(),
            ndt(2026, 1, 1, 6, 30)
        );
        let (z, how) = r.resolve("GMT+0200 Testzone");
        assert_eq!(how, ResolvedBy::FixedOffset);
        assert_eq!(z.name(), "Etc/GMT-2");
        let (_, how) = r.resolve("Lichtung Testzone");
        assert_eq!(how, ResolvedBy::Fallback);
    }
}
