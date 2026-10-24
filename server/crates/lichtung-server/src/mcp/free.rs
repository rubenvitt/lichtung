//! Free times (`find_free_times`): the allowed time (range, optionally
//! limited to a daily window and to weekdays, in one zone) minus the busy
//! intervals widened by a buffer, keeping gaps of at least the minimum
//! duration. Which events are busy is decided by the caller (rules of
//! spec section 6).

use chrono::{DateTime, Datelike, Duration, NaiveTime, Utc, Weekday};
use chrono_tz::Tz;

use super::time::{local_to_utc, midnight};

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

/// Sorts and merges overlapping or touching intervals.
fn merge(mut list: Vec<Interval>) -> Vec<Interval> {
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

/// The allowed intervals of the query.
fn allowed(q: &FreeQuery) -> Vec<Interval> {
    if q.window.is_none() && q.weekdays.is_none() {
        return vec![(q.from, q.to)];
    }
    let first = q.from.with_timezone(&q.tz).date_naive() - Duration::days(1);
    let last = q.to.with_timezone(&q.tz).date_naive();
    let mut out = Vec::new();
    let mut day = first;
    while day <= last {
        let ok = q
            .weekdays
            .as_ref()
            .is_none_or(|w| w.contains(&day.weekday()));
        if ok {
            let (s, e) = match q.window {
                Some((ws, we)) => (
                    local_to_utc(day.and_time(ws), q.tz),
                    local_to_utc(day.and_time(we), q.tz),
                ),
                None => (midnight(day, q.tz), midnight(day + Duration::days(1), q.tz)),
            };
            let (s, e) = (s.max(q.from), e.min(q.to));
            if e > s {
                out.push((s, e));
            }
        }
        day += Duration::days(1);
    }
    merge(out)
}

/// Free slots of at least `q.min`, in order.
pub fn free_slots(q: &FreeQuery, busy: Vec<Interval>) -> Vec<Interval> {
    let busy = merge(
        busy.into_iter()
            .map(|(s, e)| (s - q.buffer, e + q.buffer))
            .collect(),
    );
    let mut out = Vec::new();
    for (start, end) in allowed(q) {
        let mut cursor = start;
        for &(bs, be) in busy.iter().filter(|(bs, be)| *be > start && *bs < end) {
            if bs > cursor && bs - cursor >= q.min {
                out.push((cursor, bs));
            }
            cursor = cursor.max(be);
        }
        if end > cursor && end - cursor >= q.min {
            out.push((cursor, end));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
            window: Some((
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            )),
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
}
