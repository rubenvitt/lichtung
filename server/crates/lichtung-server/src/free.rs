//! Free time, shared by `find_free_times` (MCP) and the openings
//! ([`crate::openings`]): the allowed time (a range, optionally limited to
//! a daily window and to weekdays, or to weekly windows, in one zone) minus
//! the busy intervals widened by a buffer, keeping gaps of at least the
//! minimum duration.
//!
//! **Busy** (spec section 6): occurrences in calendars that count for
//! openings (`openings_busy`, or the calendars the caller names), except
//! cancelled ones, ones marked free (`TRANSP`/showAs), ones I declined, and
//! all-day ones in calendars whose `all_day_blocks` is off. A zero-length
//! event blocks its moment. Placeholders of one-off links (LIC-32) are
//! busy everywhere, whatever calendar they are in, except for their own
//! link.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, Duration, NaiveTime, Utc, Weekday};
use chrono_tz::Tz;
use lichtung_ical::Zone;
use serde_json::Value;

use crate::app::App;
use crate::error::{AppError, AppResult};
use crate::events::occurrences::OccurrenceQuery;
use crate::mcp::time::{fmt_at, when_of};

pub type Interval = (DateTime<Utc>, DateTime<Utc>);

/// What to look for.
#[derive(Debug, Clone)]
pub struct FreeQuery {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// Zone of the daily window and the weekdays.
    pub tz: Tz,
    pub min: Duration,
    /// Kept free before and after every busy interval.
    pub buffer: Duration,
    /// Daily window `[start, end)` in `tz`.
    pub window: Option<(NaiveTime, NaiveTime)>,
    /// Allowed weekdays in `tz` (all if `None`).
    pub weekdays: Option<Vec<Weekday>>,
}

/// A weekly time window `[start, end)` on one weekday, in the zone of the
/// caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeeklyWindow {
    pub weekday: Weekday,
    pub start: NaiveTime,
    pub end: NaiveTime,
}

/// Midnight at the start of `date` in `tz`.
fn midnight(date: chrono::NaiveDate, tz: Tz) -> DateTime<Utc> {
    Zone::Iana(tz).to_utc(date.and_time(NaiveTime::MIN))
}

/// Sorts and merges overlapping or touching intervals.
pub fn merge(mut list: Vec<Interval>) -> Vec<Interval> {
    list.retain(|(s, e)| e > s);
    list.sort();
    let mut out: Vec<Interval> = Vec::with_capacity(list.len());
    for (s, e) in list {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// The days (in `tz`) that `[from, to)` touches, with a day of slack before.
fn days(from: DateTime<Utc>, to: DateTime<Utc>, tz: Tz) -> impl Iterator<Item = chrono::NaiveDate> {
    let first = from.with_timezone(&tz).date_naive() - Duration::days(1);
    let last = to.with_timezone(&tz).date_naive();
    first.iter_days().take_while(move |d| *d <= last)
}

/// The allowed intervals of the query.
fn allowed(q: &FreeQuery) -> Vec<Interval> {
    if q.window.is_none() && q.weekdays.is_none() {
        return vec![(q.from, q.to)];
    }
    let mut out = Vec::new();
    for day in days(q.from, q.to, q.tz) {
        let ok = q
            .weekdays
            .as_ref()
            .is_none_or(|w| w.contains(&day.weekday()));
        if ok {
            let (s, e) = match q.window {
                Some((ws, we)) => (
                    Zone::Iana(q.tz).to_utc(day.and_time(ws)),
                    Zone::Iana(q.tz).to_utc(day.and_time(we)),
                ),
                None => (midnight(day, q.tz), midnight(day + Duration::days(1), q.tz)),
            };
            let (s, e) = (s.max(q.from), e.min(q.to));
            if e > s {
                out.push((s, e));
            }
        }
    }
    merge(out)
}

/// The weekly windows within `[from, to)`, as instants, merged and in
/// order. Wall-clock times that do not exist (DST gap) move forward.
pub fn weekly_allowed(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    tz: Tz,
    windows: &[WeeklyWindow],
) -> Vec<Interval> {
    if to <= from {
        return Vec::new();
    }
    let mut out = Vec::new();
    for day in days(from, to, tz) {
        for w in windows.iter().filter(|w| w.weekday == day.weekday()) {
            let s = Zone::Iana(tz).to_utc(day.and_time(w.start));
            let e = Zone::Iana(tz).to_utc(day.and_time(w.end));
            let (s, e) = (s.max(from), e.min(to));
            if e > s {
                out.push((s, e));
            }
        }
    }
    merge(out)
}

/// The gaps of at least `min` in `allowed` (in order) that no interval of
/// `busy` touches. `busy` must be merged ([`merge`]).
pub fn gaps(allowed: &[Interval], busy: &[Interval], min: Duration) -> Vec<Interval> {
    let mut out = Vec::new();
    for &(start, end) in allowed {
        let mut cursor = start;
        for &(bs, be) in busy.iter().filter(|(bs, be)| *be > start && *bs < end) {
            if bs > cursor && bs - cursor >= min {
                out.push((cursor, bs));
            }
            cursor = cursor.max(be);
        }
        if end > cursor && end - cursor >= min {
            out.push((cursor, end));
        }
    }
    out
}

/// Free slots of at least `q.min`, in order.
pub fn free_slots(q: &FreeQuery, busy: Vec<Interval>) -> Vec<Interval> {
    let busy = merge(
        busy.into_iter()
            .map(|(s, e)| (s - q.buffer, e + q.buffer))
            .collect(),
    );
    gaps(&allowed(q), &busy, q.min)
}

/// The busy intervals among occurrences (event JSON form, as
/// [`App::occurrences`] gives them), by the rules in the module docs.
/// `all_day_blocks` maps calendar IDs to that setting; events in
/// `exclude` (placeholders of the asking link) do not count.
pub fn busy_of(
    occurrences: &[Value],
    all_day_blocks: &HashMap<String, bool>,
    exclude: &HashSet<String>,
    tz: Tz,
) -> Vec<Interval> {
    occurrences
        .iter()
        .filter(|o| o["status"] != "cancelled")
        .filter(|o| o["availability"] != "free")
        .filter(|o| o["self_status"] != "declined")
        .filter(|o| {
            o["all_day"] != true
                || o["calendar_id"]
                    .as_str()
                    .and_then(|id| all_day_blocks.get(id))
                    .is_some_and(|b| *b)
        })
        .filter(|o| {
            o["event_id"]
                .as_str()
                .is_none_or(|id| !exclude.contains(id))
        })
        .filter_map(|o| {
            let s = when_of(&o["start"], tz)?.instant(tz);
            let e = when_of(&o["end"], tz)?.instant(tz);
            Some((s, e.max(s)))
        })
        // A zero-length event blocks its moment.
        .map(|(s, e)| {
            if e == s {
                (s, s + Duration::minutes(1))
            } else {
                (s, e)
            }
        })
        .collect()
}

/// Busy intervals of a range.
#[derive(Debug, Clone, Default)]
pub struct Busy {
    /// Unmerged, unwidened.
    pub intervals: Vec<Interval>,
    /// The range held too many occurrences to be sure (see
    /// [`crate::events::occurrences::MAX_OCCURRENCES`]).
    pub truncated: bool,
}

impl App {
    /// The busy intervals of `[from, to)` in `calendar_ids` plus the
    /// placeholders of all one-off links, except those of `own_opening`
    /// (rules in the module docs). `tz` decides floating times and all-day
    /// events.
    pub async fn busy_intervals(
        &self,
        calendar_ids: &[String],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        tz: Tz,
        own_opening: Option<&str>,
    ) -> AppResult<Busy> {
        let own = own_opening.map(str::to_owned);
        let (own_placeholders, reserved) = self
            .with_reader(move |conn| {
                Ok(crate::openings::store::reservations(
                    conn,
                    own.as_deref(),
                    from.timestamp_millis(),
                    to.timestamp_millis(),
                )?)
            })
            .await?;
        let mut busy = Busy {
            intervals: reserved,
            truncated: false,
        };
        if calendar_ids.is_empty() {
            return Ok(busy);
        }
        let found = self
            .occurrences(OccurrenceQuery {
                from: Some(fmt_at(from, Tz::UTC)),
                to: Some(fmt_at(to, Tz::UTC)),
                calendar_ids: Some(calendar_ids.join(",")),
                time_zone: Some(tz.name().to_owned()),
                include_cancelled: Some(false),
            })
            .await?;
        busy.truncated = found.truncated;
        let value = serde_json::to_value(&found).map_err(|e| AppError::Internal(e.into()))?;
        let list = value["occurrences"].as_array().cloned().unwrap_or_default();
        let all_day_blocks: HashMap<String, bool> = self
            .list_calendars()
            .await?
            .into_iter()
            .map(|c| (c.id, c.all_day_blocks))
            .collect();
        busy.intervals
            .extend(busy_of(&list, &all_day_blocks, &own_placeholders, tz));
        Ok(busy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn query(from: &str, to: &str) -> FreeQuery {
        FreeQuery {
            from: t(from),
            to: t(to),
            tz: "Europe/Berlin".parse().unwrap(),
            min: Duration::minutes(30),
            buffer: Duration::zero(),
            window: None,
            weekdays: None,
        }
    }

    fn fmt(list: &[Interval]) -> Vec<(String, String)> {
        list.iter()
            .map(|(s, e)| {
                (
                    s.format("%m-%dT%H:%M").to_string(),
                    e.format("%m-%dT%H:%M").to_string(),
                )
            })
            .collect()
    }

    fn clock(s: &str) -> NaiveTime {
        NaiveTime::parse_from_str(s, "%H:%M").unwrap()
    }

    #[test]
    fn gaps_between_busy_intervals() {
        let q = query("2026-11-03T08:00:00Z", "2026-11-03T12:00:00Z");
        let busy = vec![
            (t("2026-11-03T09:00:00Z"), t("2026-11-03T10:00:00Z")),
            (t("2026-11-03T09:30:00Z"), t("2026-11-03T10:15:00Z")),
            (t("2026-11-03T10:30:00Z"), t("2026-11-03T11:45:00Z")),
        ];
        assert_eq!(
            fmt(&free_slots(&q, busy.clone())),
            [("11-03T08:00".into(), "11-03T09:00".into())]
        );
        let q = FreeQuery {
            min: Duration::minutes(15),
            ..q
        };
        assert_eq!(
            fmt(&free_slots(&q, busy)),
            [
                ("11-03T08:00".into(), "11-03T09:00".into()),
                ("11-03T10:15".into(), "11-03T10:30".into()),
                ("11-03T11:45".into(), "11-03T12:00".into()),
            ]
        );
    }

    #[test]
    fn buffer_window_and_weekdays() {
        // Tue 3 Nov to Sun 8 Nov 2026, 9–17 Berlin (UTC+1), weekdays only.
        let q = FreeQuery {
            buffer: Duration::minutes(15),
            window: Some((clock("09:00"), clock("17:00"))),
            weekdays: Some(vec![
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
            ]),
            min: Duration::minutes(60),
            ..query("2026-11-03T00:00:00+01:00", "2026-11-09T00:00:00+01:00")
        };
        let busy = vec![
            // Tue 10:00–15:00 Berlin
            (t("2026-11-03T09:00:00Z"), t("2026-11-03T14:00:00Z")),
            // Wed whole day
            (t("2026-11-03T23:00:00Z"), t("2026-11-04T23:00:00Z")),
        ];
        let slots = free_slots(&q, busy);
        assert_eq!(
            fmt(&slots),
            [
                // Tue 15:15–17:00 Berlin; 09:00–09:45 is too short
                ("11-03T14:15".into(), "11-03T16:00".into()),
                ("11-05T08:00".into(), "11-05T16:00".into()),
                ("11-06T08:00".into(), "11-06T16:00".into()),
            ]
        );
    }

    #[test]
    fn weekly_windows_follow_the_zone_across_dst() {
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        let windows = [
            WeeklyWindow {
                weekday: Weekday::Fri,
                start: clock("09:00"),
                end: clock("12:00"),
            },
            WeeklyWindow {
                weekday: Weekday::Fri,
                start: clock("11:00"),
                end: clock("13:00"),
            },
            WeeklyWindow {
                weekday: Weekday::Mon,
                start: clock("14:00"),
                end: clock("15:00"),
            },
        ];
        // Fri 23 Oct (summer time), Mon 26 Oct and Fri 30 Oct (winter time).
        let list = weekly_allowed(
            t("2026-10-23T10:00:00+02:00"),
            t("2026-10-31T00:00:00+01:00"),
            berlin,
            &windows,
        );
        assert_eq!(
            fmt(&list),
            [
                // clipped to `from`, overlapping windows merged
                ("10-23T08:00".into(), "10-23T11:00".into()),
                ("10-26T13:00".into(), "10-26T14:00".into()),
                ("10-30T08:00".into(), "10-30T12:00".into()),
            ]
        );
        assert!(weekly_allowed(
            t("2026-10-23T10:00:00Z"),
            t("2026-10-23T10:00:00Z"),
            berlin,
            &windows
        )
        .is_empty());
    }

    #[test]
    fn busy_follows_the_opening_rules() {
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        let at = |s: &str| json!({"utc": s});
        let occ = |id: &str, cal: &str, start: Value, end: Value| {
            json!({"event_id": id, "calendar_id": cal, "start": start, "end": end,
                   "status": "confirmed", "availability": "busy", "self_status": null,
                   "all_day": false})
        };
        let mut list = vec![
            occ(
                "ev_a",
                "cal_1",
                at("2026-11-03T09:00:00Z"),
                at("2026-11-03T10:00:00Z"),
            ),
            occ(
                "ev_b",
                "cal_1",
                at("2026-11-03T11:00:00Z"),
                at("2026-11-03T11:00:00Z"),
            ),
            occ(
                "ev_own",
                "cal_1",
                at("2026-11-03T12:00:00Z"),
                at("2026-11-03T13:00:00Z"),
            ),
        ];
        let mut cancelled = occ(
            "ev_c",
            "cal_1",
            at("2026-11-03T14:00:00Z"),
            at("2026-11-03T15:00:00Z"),
        );
        cancelled["status"] = json!("cancelled");
        let mut free = cancelled.clone();
        free["status"] = json!("confirmed");
        free["availability"] = json!("free");
        let mut declined = cancelled.clone();
        declined["status"] = json!("confirmed");
        declined["self_status"] = json!("declined");
        let mut holiday = occ(
            "ev_h",
            "cal_1",
            json!({"date": "2026-11-04"}),
            json!({"date": "2026-11-05"}),
        );
        holiday["all_day"] = json!(true);
        let mut blocking = holiday.clone();
        blocking["calendar_id"] = json!("cal_2");
        list.extend([cancelled, free, declined, holiday, blocking]);
        let blocks = HashMap::from([("cal_1".to_owned(), false), ("cal_2".to_owned(), true)]);
        let exclude = HashSet::from(["ev_own".to_owned()]);
        assert_eq!(
            fmt(&busy_of(&list, &blocks, &exclude, berlin)),
            [
                ("11-03T09:00".into(), "11-03T10:00".into()),
                // zero length blocks its moment
                ("11-03T11:00".into(), "11-03T11:01".into()),
                // all-day only in the calendar with all_day_blocks
                ("11-03T23:00".into(), "11-04T23:00".into()),
            ]
        );
    }
}
