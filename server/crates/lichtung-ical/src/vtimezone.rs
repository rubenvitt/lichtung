//! `VTIMEZONE` components generated from the IANA database (`chrono-tz`).
//!
//! RFC 5545 §3.6.5 requires a `VTIMEZONE` for every `TZID` an object uses.
//! Some servers (Cyrus/Fastmail, iCloud) refuse objects without one, so the
//! write path adds them with [`ensure_vtimezones`] before every PUT.
//!
//! `chrono-tz` does not expose its transition table, so transitions are found
//! by sampling the offset and bisecting to the second. The rule that is in
//! force at the end of the range is written as a yearly `RRULE` (as long as it
//! held for at least [`MIN_RULE_YEARS`] years); every other transition becomes
//! an observance of its own. That is robust rather than pretty.

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc};
use chrono_tz::{OffsetComponents, OffsetName, Tz};
use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::str::FromStr;

use crate::object::CalendarObject;
use crate::parser::{Component, Property};
use crate::value::{CalTime, RDate};

/// Sampling step when searching transitions (no zone changes twice within it).
const SAMPLE_STEP_SECS: i64 = 6 * 3600;
/// A yearly pattern must hold this many consecutive years to become an `RRULE`.
const MIN_RULE_YEARS: usize = 3;
/// How far beyond the current year (or the latest used value) transitions
/// are written out.
const YEARS_AHEAD: i32 = 10;
/// Properties whose `TZID` needs a `VTIMEZONE`.
const ZONED_PROPERTIES: [&str; 5] = ["DTSTART", "DTEND", "RECURRENCE-ID", "EXDATE", "RDATE"];

/// Offset, DST flag and abbreviation in force at an instant.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    offset: i32,
    dst: bool,
    name: Option<String>,
}

impl State {
    fn at(tz: Tz, ts: i64) -> Self {
        let utc = DateTime::from_timestamp(ts, 0)
            .unwrap_or_default()
            .naive_utc();
        let off = tz.offset_from_utc_datetime(&utc);
        Self {
            offset: off.fix().local_minus_utc(),
            dst: off.dst_offset().num_seconds() > 0,
            name: off.abbreviation().map(str::to_string),
        }
    }
}

/// One change of [`State`].
#[derive(Debug, Clone)]
struct Transition {
    /// UTC timestamp of the change.
    at: i64,
    /// Offset before the change.
    from: i32,
    /// State after the change.
    to: State,
}

impl Transition {
    /// Wall-clock onset in the offset before the change (as `DTSTART` wants).
    fn onset(&self) -> NaiveDateTime {
        DateTime::from_timestamp(self.at + i64::from(self.from), 0)
            .unwrap_or_default()
            .naive_utc()
    }
}

fn year_start(year: i32) -> i64 {
    NaiveDate::from_ymd_opt(year, 1, 1)
        .unwrap_or_default()
        .and_time(NaiveTime::MIN)
        .and_utc()
        .timestamp()
}

/// All state changes between 1 January of the first and 31 December of the
/// last year (UTC), plus the state at the start.
fn transitions(tz: Tz, years: &RangeInclusive<i32>) -> (State, Vec<Transition>) {
    let start = year_start(*years.start());
    let end = year_start(years.end().saturating_add(1));
    let initial = State::at(tz, start);
    let mut prev = initial.clone();
    let mut out = Vec::new();
    let mut t = start;
    while t < end {
        let next = (t + SAMPLE_STEP_SECS).min(end);
        if State::at(tz, next) == prev {
            t = next;
            continue;
        }
        let (mut lo, mut hi) = (t, next);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if State::at(tz, mid) == prev {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let to = State::at(tz, hi);
        out.push(Transition {
            at: hi,
            from: prev.offset,
            to: to.clone(),
        });
        prev = to;
        t = hi;
    }
    (initial, out)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .and_then(|d| d.pred_opt())
        .map_or(28, |d| d.day())
}

/// "Weekday on or after day `first_day` of the month" at a fixed wall-clock
/// time, with fixed offsets and name: one yearly transition.
#[derive(Debug, Clone)]
struct YearlyRule {
    month: u32,
    weekday: chrono::Weekday,
    time: NaiveTime,
    first_day: u32,
    from: i32,
    to: State,
}

impl YearlyRule {
    fn fits(&self, t: &Transition) -> bool {
        let o = t.onset();
        o.month() == self.month
            && o.weekday() == self.weekday
            && o.time() == self.time
            && (self.first_day..self.first_day + 7).contains(&o.day())
            && t.from == self.from
            && t.to == self.to
    }

    /// Whether the window is "the n-th" (1, 8, 15, 22) or "the last" weekday.
    fn canonical(&self, month_len: u32) -> bool {
        matches!(self.first_day, 1 | 8 | 15 | 22)
            || (self.month != 2 && self.first_day + 6 == month_len)
    }

    fn rrule(&self, month_len: u32) -> String {
        let day = weekday_code(self.weekday);
        let by = match self.first_day {
            1 | 8 | 15 | 22 => format!("BYDAY={}{day}", (self.first_day - 1) / 7 + 1),
            d if self.month != 2 && d + 6 == month_len => format!("BYDAY=-1{day}"),
            d => {
                let days: Vec<String> = (d..d + 7)
                    .filter(|x| *x <= month_len)
                    .map(|x| x.to_string())
                    .collect();
                format!("BYDAY={day};BYMONTHDAY={}", days.join(","))
            }
        };
        format!("FREQ=YEARLY;BYMONTH={};{by}", self.month)
    }
}

fn weekday_code(w: chrono::Weekday) -> &'static str {
    use chrono::Weekday::*;
    match w {
        Mon => "MO",
        Tue => "TU",
        Wed => "WE",
        Thu => "TH",
        Fri => "FR",
        Sat => "SA",
        Sun => "SU",
    }
}

/// For a transition in the last year, the yearly rule that explains the
/// longest unbroken run of years ending there, with the indices of that run
/// (oldest first).
fn best_rule(all: &[Transition], last: usize) -> Option<(YearlyRule, Vec<usize>)> {
    let t = &all[last];
    let o = t.onset();
    let len = days_in_month(o.year(), o.month());
    let max_first = if o.month() == 2 { 22 } else { len - 6 };
    let lo = o.day().saturating_sub(6).max(1);
    let hi = o.day().min(max_first);
    let mut candidates: Vec<YearlyRule> = (lo..=hi)
        .map(|first_day| YearlyRule {
            month: o.month(),
            weekday: o.weekday(),
            time: o.time(),
            first_day,
            from: t.from,
            to: t.to.clone(),
        })
        .collect();
    // Prefer the readable forms on equal runs.
    candidates.sort_by_key(|r| !r.canonical(len));
    let mut best: Option<(YearlyRule, Vec<usize>)> = None;
    for rule in candidates {
        let mut run = vec![last];
        let mut year = o.year();
        loop {
            year -= 1;
            let found = all
                .iter()
                .enumerate()
                .find(|(_, x)| x.onset().year() == year && rule.fits(x));
            match found {
                Some((i, _)) => run.push(i),
                None => break,
            }
        }
        if best.as_ref().is_none_or(|(_, b)| run.len() > b.len()) {
            best = Some((rule, run));
        }
    }
    best.map(|(rule, mut run)| {
        run.reverse();
        (rule, run)
    })
}

fn format_offset(secs: i32) -> String {
    let sign = if secs < 0 { '-' } else { '+' };
    let a = secs.unsigned_abs();
    let (h, m, s) = (a / 3600, a / 60 % 60, a % 60);
    if s == 0 {
        format!("{sign}{h:02}{m:02}")
    } else {
        format!("{sign}{h:02}{m:02}{s:02}")
    }
}

fn observance(onset: NaiveDateTime, from: i32, to: &State, rrule: Option<String>) -> Component {
    let mut c = Component::new(if to.dst { "DAYLIGHT" } else { "STANDARD" });
    let props = c.properties_mut();
    props.push(Property::new(
        "DTSTART",
        onset.format("%Y%m%dT%H%M%S").to_string(),
    ));
    if let Some(rule) = rrule {
        props.push(Property::new("RRULE", rule));
    }
    props.push(Property::new("TZOFFSETFROM", format_offset(from)));
    props.push(Property::new("TZOFFSETTO", format_offset(to.offset)));
    if let Some(name) = &to.name {
        props.push(Property::new_text("TZNAME", name));
    }
    c
}

/// Builds a `VTIMEZONE` for an IANA zone.
///
/// `TZID` and `X-LIC-LOCATION` are the IANA name. Transitions within `years`
/// are listed; the rule still in force at the end of the range is written as
/// a yearly `RRULE` (open-ended), everything else as single observances.
/// A zone without any transition in the range (e.g. `Asia/Kolkata`) gets a
/// single observance (`STANDARD` unless it is on permanent DST).
pub fn vtimezone_for(tz: Tz, years: RangeInclusive<i32>) -> Component {
    let mut vtz = Component::new("VTIMEZONE");
    vtz.properties_mut().push(Property::new("TZID", tz.name()));
    vtz.properties_mut()
        .push(Property::new("X-LIC-LOCATION", tz.name()));

    let (initial, all) = transitions(tz, &years);
    let mut observances: Vec<(NaiveDateTime, Component)> = Vec::new();
    let mut covered: HashSet<usize> = HashSet::new();
    let last_year = *years.end();
    for i in 0..all.len() {
        if covered.contains(&i) || all[i].onset().year() != last_year {
            continue;
        }
        let Some((rule, run)) = best_rule(&all, i) else {
            continue;
        };
        if run.len() < MIN_RULE_YEARS {
            continue;
        }
        let first = &all[run[0]];
        let onset = first.onset();
        let len = days_in_month(onset.year(), onset.month());
        observances.push((
            onset,
            observance(onset, first.from, &first.to, Some(rule.rrule(len))),
        ));
        covered.extend(run);
    }
    for (i, t) in all.iter().enumerate() {
        if !covered.contains(&i) {
            observances.push((t.onset(), observance(t.onset(), t.from, &t.to, None)));
        }
    }
    if observances.is_empty() {
        let start = NaiveDate::from_ymd_opt(*years.start(), 1, 1)
            .unwrap_or_default()
            .and_time(NaiveTime::MIN);
        observances.push((start, observance(start, initial.offset, &initial, None)));
    }
    observances.sort_by_key(|(onset, _)| *onset);
    vtz.components_mut()
        .extend(observances.into_iter().map(|(_, c)| c));
    vtz
}

/// Adds a `VTIMEZONE` for every IANA `TZID` used in `DTSTART`, `DTEND`,
/// `RECURRENCE-ID`, `EXDATE` or `RDATE` that has none yet.
///
/// Existing `VTIMEZONE`s are left alone; UTC and floating values need none;
/// TZIDs that are not exact IANA names are skipped (an original that uses
/// them brings its own definition, and nothing is made up). The range of
/// listed transitions starts at the earliest year the zone is used in and
/// ends ten years after the later of now and its latest use. Returns the
/// number of components added; with none, the object is untouched.
pub fn ensure_vtimezones(obj: &mut CalendarObject) -> usize {
    let now_year = Utc::now().year();
    let cal = obj.calendar();
    let existing: HashSet<&str> = cal
        .components_named("VTIMEZONE")
        .filter_map(|c| c.property("TZID"))
        .map(|p| p.value().trim())
        .collect();
    // (zone, earliest year, latest year) in order of first use.
    let mut needed: Vec<(Tz, i32, i32)> = Vec::new();
    for comp in cal.components().iter().filter(|c| !c.is("VTIMEZONE")) {
        for p in comp.properties() {
            if !ZONED_PROPERTIES.iter().any(|n| p.is(n)) {
                continue;
            }
            let Some(tzid) = p.param("TZID").map(str::trim) else {
                continue;
            };
            if existing.contains(tzid) {
                continue;
            }
            let Ok(tz) = Tz::from_str(tzid) else {
                continue;
            };
            let years: Vec<i32> = if p.is("RDATE") {
                RDate::list_from_property(p)
                    .map(|l| l.iter().map(|r| r.start().naive().year()).collect())
                    .unwrap_or_default()
            } else {
                CalTime::list_from_property(p)
                    .map(|l| l.iter().map(|t| t.naive().year()).collect())
                    .unwrap_or_default()
            };
            let lo = years.iter().copied().min().unwrap_or(now_year);
            let hi = years.iter().copied().max().unwrap_or(now_year);
            match needed.iter_mut().find(|(z, _, _)| *z == tz) {
                Some((_, a, b)) => {
                    *a = (*a).min(lo);
                    *b = (*b).max(hi);
                }
                None => needed.push((tz, lo, hi)),
            }
        }
    }
    if needed.is_empty() {
        return 0;
    }
    let count = needed.len();
    let comps = obj.calendar_mut().components_mut();
    let at = comps
        .iter()
        .position(|c| !c.is("VTIMEZONE"))
        .unwrap_or(comps.len());
    let added: Vec<Component> = needed
        .into_iter()
        .map(|(tz, lo, hi)| {
            let end = hi.max(now_year).saturating_add(YEARS_AHEAD);
            vtimezone_for(tz, lo.min(end)..=end)
        })
        .collect();
    comps.splice(at..at, added);
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tz::{CustomZone, Zone};
    use chrono::Duration;

    fn props(c: &Component, name: &str) -> Vec<String> {
        c.properties_named(name)
            .map(|p| p.value().to_string())
            .collect()
    }

    fn observances(vtz: &Component, kind: &str) -> Vec<Component> {
        vtz.components_named(kind).cloned().collect()
    }

    #[test]
    fn table_of_zones() {
        struct Case {
            tz: Tz,
            /// (kind, TZOFFSETFROM, TZOFFSETTO, TZNAME, RRULE) of the rule observances.
            rules: &'static [(
                &'static str,
                &'static str,
                &'static str,
                &'static str,
                &'static str,
            )],
            daylight: bool,
        }
        let cases = [
            Case {
                tz: chrono_tz::Europe::Berlin,
                rules: &[
                    (
                        "DAYLIGHT",
                        "+0100",
                        "+0200",
                        "CEST",
                        "FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU",
                    ),
                    (
                        "STANDARD",
                        "+0200",
                        "+0100",
                        "CET",
                        "FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU",
                    ),
                ],
                daylight: true,
            },
            Case {
                tz: chrono_tz::America::New_York,
                rules: &[
                    (
                        "DAYLIGHT",
                        "-0500",
                        "-0400",
                        "EDT",
                        "FREQ=YEARLY;BYMONTH=3;BYDAY=2SU",
                    ),
                    (
                        "STANDARD",
                        "-0400",
                        "-0500",
                        "EST",
                        "FREQ=YEARLY;BYMONTH=11;BYDAY=1SU",
                    ),
                ],
                daylight: true,
            },
            Case {
                tz: chrono_tz::Asia::Kolkata,
                rules: &[],
                daylight: false,
            },
        ];
        for case in cases {
            let name = case.tz.name();
            let vtz = vtimezone_for(case.tz, 2020..=2036);
            assert_eq!(props(&vtz, "TZID"), [name]);
            assert_eq!(props(&vtz, "X-LIC-LOCATION"), [name]);
            assert_eq!(
                !observances(&vtz, "DAYLIGHT").is_empty(),
                case.daylight,
                "{name}"
            );
            for (kind, from, to, tzname, rrule) in case.rules {
                let found = observances(&vtz, kind).into_iter().any(|o| {
                    props(&o, "TZOFFSETFROM") == [*from]
                        && props(&o, "TZOFFSETTO") == [*to]
                        && props(&o, "TZNAME") == [*tzname]
                        && props(&o, "RRULE") == [*rrule]
                });
                assert!(found, "{name}: {kind} {rrule}\n{}", vtz.to_ics());
            }
            if case.rules.is_empty() {
                let std = observances(&vtz, "STANDARD");
                assert_eq!(std.len(), 1, "{}", vtz.to_ics());
                assert_eq!(props(&std[0], "TZOFFSETFROM"), ["+0530"]);
                assert_eq!(props(&std[0], "TZOFFSETTO"), ["+0530"]);
                assert_eq!(props(&std[0], "TZNAME"), ["IST"]);
                assert_eq!(props(&std[0], "DTSTART"), ["20200101T000000"]);
            }
        }
        // Berlin follows the EU rule for the whole range: exactly two observances.
        let berlin = vtimezone_for(chrono_tz::Europe::Berlin, 1996..=2036);
        assert_eq!(berlin.components().len(), 2, "{}", berlin.to_ics());
        assert_eq!(
            props(&observances(&berlin, "DAYLIGHT")[0], "DTSTART"),
            ["19960331T020000"]
        );
    }

    #[test]
    fn utc_values_need_no_vtimezone() {
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//DE\r\n\
BEGIN:VEVENT\r\nUID:utc@example.org\r\nDTSTAMP:20261001T080000Z\r\n\
DTSTART:20261104T120000Z\r\nDTEND:20261104T130000Z\r\nSUMMARY:Abstimmung\r\n\
END:VEVENT\r\nBEGIN:VEVENT\r\nUID:utc@example.org\r\nDTSTAMP:20261001T080000Z\r\n\
DTSTART:20261105T090000\r\nSUMMARY:Schwebend\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let mut obj = CalendarObject::parse(ics).unwrap();
        assert_eq!(ensure_vtimezones(&mut obj), 0);
        assert_eq!(obj.to_ics(), ics);
    }

    /// The generated definitions read back through `CustomZone` give the same
    /// offsets as chrono-tz, including the years before the rule took over
    /// and the years after the listed range (from the `RRULE`).
    #[test]
    fn custom_zone_reads_generated_offsets() {
        let zones = [
            chrono_tz::Europe::Berlin,
            chrono_tz::America::New_York,
            chrono_tz::Asia::Kolkata,
            chrono_tz::Australia::Sydney,
            chrono_tz::America::Sao_Paulo,
            chrono_tz::Europe::Moscow,
            chrono_tz::Africa::Casablanca,
            chrono_tz::Asia::Jerusalem,
        ];
        for tz in zones {
            let vtz = vtimezone_for(tz, 1970..=2036);
            let custom = Zone::Custom(std::sync::Arc::new(
                CustomZone::from_vtimezone(&vtz).expect("readable"),
            ));
            assert_eq!(custom.name(), tz.name());
            let iana = Zone::Iana(tz);
            let start = NaiveDate::from_ymd_opt(1970, 1, 1)
                .unwrap()
                .and_time(NaiveTime::MIN);
            // Every ~5 days plus noon, through 2045 (beyond the range only
            // zones with a rule in force are compared).
            let mut t = start;
            let end = NaiveDate::from_ymd_opt(2046, 1, 1)
                .unwrap()
                .and_time(NaiveTime::MIN);
            let has_rule = vtz
                .components()
                .iter()
                .any(|o| o.property("RRULE").is_some());
            while t < end {
                if t.year() <= 2036 || has_rule {
                    assert_eq!(
                        custom.offset_at(t),
                        iana.offset_at(t),
                        "{} at {t}\n{}",
                        tz.name(),
                        vtz.to_ics()
                    );
                }
                t += Duration::hours(5 * 24 + 7);
            }
            // And right around each transition of 2026.
            for (y, m, d) in [(2026, 3, 29), (2026, 10, 25), (2026, 3, 8), (2026, 11, 1)] {
                let day = NaiveDate::from_ymd_opt(y, m, d)
                    .unwrap()
                    .and_time(NaiveTime::MIN);
                for h in 0..24 {
                    let u = day + Duration::hours(h);
                    assert_eq!(
                        custom.offset_at(u),
                        iana.offset_at(u),
                        "{} at {u}",
                        tz.name()
                    );
                }
            }
        }
    }

    #[test]
    fn existing_vtimezone_stays_byte_identical() {
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//DE\r\n\
BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nBEGIN:STANDARD\r\nDTSTART:19701025T030000\r\n\
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\n\
TZNAME:MEZ\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
BEGIN:VTIMEZONE\r\nTZID:Beispielzone Mitte\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\n\
TZOFFSETFROM:+0300\r\nTZOFFSETTO:+0300\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:chor@example.org\r\nDTSTAMP:20261001T080000Z\r\n\
DTSTART;TZID=Europe/Berlin:20261104T190000\r\n\
DTEND;TZID=Beispielzone Mitte:20261104T220000\r\n\
EXDATE;TZID=Unbekannte Zone:20261111T190000\r\n\
SUMMARY:Chorprobe\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let mut obj = CalendarObject::parse(ics).unwrap();
        assert_eq!(ensure_vtimezones(&mut obj), 0);
        assert_eq!(obj.to_ics(), ics);
    }

    #[test]
    fn missing_zones_are_added_once_before_the_events() {
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//DE\r\n\
BEGIN:VEVENT\r\nUID:reise@example.org\r\nDTSTAMP:20261001T080000Z\r\n\
DTSTART;TZID=Europe/Berlin:20261104T190000\r\n\
DTEND;TZID=America/New_York:20261104T220000\r\n\
RRULE:FREQ=WEEKLY;COUNT=4\r\n\
EXDATE;TZID=Europe/Berlin:20261111T190000\r\n\
RDATE;VALUE=PERIOD;TZID=Asia/Kolkata:20151230T100000/PT1H\r\n\
SUMMARY:Reiseplanung\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:reise@example.org\r\nDTSTAMP:20261001T080000Z\r\n\
RECURRENCE-ID;TZID=Europe/Berlin:20261118T190000\r\n\
DTSTART;TZID=Europe/Berlin:20261118T200000\r\nSUMMARY:Reiseplanung\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";
        let mut obj = CalendarObject::parse(ics).unwrap();
        assert_eq!(ensure_vtimezones(&mut obj), 3);
        let names: Vec<String> = obj
            .calendar()
            .components()
            .iter()
            .map(|c| {
                c.property("TZID")
                    .map_or(c.name().to_string(), |p| p.value().to_string())
            })
            .collect();
        assert_eq!(
            names,
            [
                "Europe/Berlin",
                "America/New_York",
                "Asia/Kolkata",
                "VEVENT",
                "VEVENT"
            ]
        );
        // Kolkata starts at the earliest use (2015).
        let kolkata = &obj.calendar().components()[2];
        assert_eq!(
            props(&kolkata.components()[0], "DTSTART"),
            ["20150101T000000"]
        );
        // Idempotent and parseable.
        assert_eq!(ensure_vtimezones(&mut obj), 0);
        let again = CalendarObject::parse(&obj.to_ics()).unwrap();
        assert_eq!(again.to_ics(), obj.to_ics());
    }
}
