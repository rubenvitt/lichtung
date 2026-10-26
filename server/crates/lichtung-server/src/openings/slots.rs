//! Bookable slots of a link, as a pure function of its rules, the busy
//! intervals and "now" (table tests below).
//!
//! - **Used up:** a booked one-off link offers nothing.
//! - **Suggestions** (one-off link with suggestions): exactly the
//!   suggestions that start after `now` and are still free. Minimum
//!   notice, horizon and windows do not apply (the owner picked the times),
//!   buffers do.
//! - **Rules** (templates, one-off links without suggestions): slots of
//!   the link's length inside the weekly windows, starting every
//!   [`step`] from the start of a window, no earlier than `now` + minimum
//!   notice and before `now` + horizon. A slot is free when no busy interval
//!   touches it widened by the buffers (`buffer_before` before its start,
//!   `buffer_after` after its end); buffers may reach outside the window.

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;

use crate::free::{merge, weekly_allowed, Interval, WeeklyWindow};

/// One bookable time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// The suggestion this slot is (one-off links with suggestions).
    pub suggestion_id: Option<String>,
}

/// What the computation needs of a link.
#[derive(Debug, Clone)]
pub struct Rules {
    pub duration: Duration,
    pub tz: Tz,
    pub windows: Vec<WeeklyWindow>,
    pub buffer_before: Duration,
    pub buffer_after: Duration,
    pub min_notice: Duration,
    pub horizon: Duration,
    /// Offer only these (`id`, start, end); `None` = by the rules.
    pub suggestions: Option<Vec<(String, Interval)>>,
    pub booked: bool,
}

/// Distance between the starts of two slots: 15 minutes for links of up
/// to 15 minutes, else 30 (whole and half hours for windows that start on
/// them).
pub fn step(duration: Duration) -> Duration {
    if duration <= Duration::minutes(15) {
        Duration::minutes(15)
    } else {
        Duration::minutes(30)
    }
}

/// Whether `[s, e)` widened by the buffers touches a busy interval.
/// `busy` must be merged.
fn is_busy(busy: &[Interval], s: DateTime<Utc>, e: DateTime<Utc>, r: &Rules) -> bool {
    let (s, e) = (s - r.buffer_before, e + r.buffer_after);
    // First busy interval that ends after `s`.
    let i = busy.partition_point(|(_, be)| *be <= s);
    busy.get(i).is_some_and(|(bs, _)| *bs < e)
}

/// The bookable slots starting in `[from, to)`, in order.
pub fn bookable(
    r: &Rules,
    busy: Vec<Interval>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Vec<Slot> {
    if r.booked || to <= from {
        return Vec::new();
    }
    let busy = merge(busy);
    if let Some(suggestions) = &r.suggestions {
        let mut out: Vec<Slot> = suggestions
            .iter()
            .filter(|(_, (s, _))| *s > now && *s >= from && *s < to)
            .filter(|(_, (s, e))| !is_busy(&busy, *s, *e, r))
            .map(|(id, (s, e))| Slot {
                start: *s,
                end: *e,
                suggestion_id: Some(id.clone()),
            })
            .collect();
        out.sort_by_key(|s| (s.start, s.end));
        return out;
    }
    let lower = from.max(now + r.min_notice);
    let upper = to.min(now + r.horizon);
    if upper <= lower || r.duration <= Duration::zero() {
        return Vec::new();
    }
    let step = step(r.duration);
    let mut out = Vec::new();
    // A day of slack before `lower` keeps windows whole, so the steps count
    // from the start of each window.
    for (ws, we) in weekly_allowed(
        lower - Duration::days(1),
        upper + r.duration,
        r.tz,
        &r.windows,
    ) {
        let mut start = ws;
        if start < lower {
            let behind = (lower - ws).num_seconds();
            let n = (behind + step.num_seconds() - 1) / step.num_seconds();
            start = ws + Duration::seconds(n * step.num_seconds());
        }
        while start < upper && start + r.duration <= we {
            let end = start + r.duration;
            if !is_busy(&busy, start, end, r) {
                out.push(Slot {
                    start,
                    end,
                    suggestion_id: None,
                });
            }
            start += step;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveTime, Weekday};

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn clock(s: &str) -> NaiveTime {
        NaiveTime::parse_from_str(s, "%H:%M").unwrap()
    }

    /// 30 minutes, Mon–Fri 09:00–12:00 Berlin, no buffers, no notice, 21
    /// days.
    fn rules() -> Rules {
        let weekdays = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ];
        Rules {
            duration: Duration::minutes(30),
            tz: "Europe/Berlin".parse().unwrap(),
            windows: weekdays
                .iter()
                .map(|d| WeeklyWindow {
                    weekday: *d,
                    start: clock("09:00"),
                    end: clock("12:00"),
                })
                .collect(),
            buffer_before: Duration::zero(),
            buffer_after: Duration::zero(),
            min_notice: Duration::zero(),
            horizon: Duration::days(21),
            suggestions: None,
            booked: false,
        }
    }

    /// Slot starts as Berlin wall-clock times (`11-03 09:00`).
    fn starts(slots: &[Slot]) -> Vec<String> {
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        slots
            .iter()
            .map(|s| {
                s.start
                    .with_timezone(&berlin)
                    .format("%m-%d %H:%M")
                    .to_string()
            })
            .collect()
    }

    // Tuesday, 3 Nov 2026, 07:00 Berlin (UTC+1).
    const NOW: &str = "2026-11-03T07:00:00+01:00";
    const DAY_FROM: &str = "2026-11-03T00:00:00+01:00";
    const DAY_TO: &str = "2026-11-04T00:00:00+01:00";

    #[test]
    fn table() {
        let busy_10 = vec![(
            t("2026-11-03T10:00:00+01:00"),
            t("2026-11-03T10:45:00+01:00"),
        )];
        let all_day = [
            "11-03 09:00",
            "11-03 09:30",
            "11-03 10:00",
            "11-03 10:30",
            "11-03 11:00",
            "11-03 11:30",
        ];
        /// Name, rules, busy, now, expected starts.
        type Case<'a> = (&'a str, Rules, Vec<Interval>, &'a str, Vec<&'a str>);
        let cases: Vec<Case> = vec![
            ("free day", rules(), vec![], NOW, all_day.to_vec()),
            (
                "busy 10:00–10:45 blocks the slots it touches",
                rules(),
                busy_10.clone(),
                NOW,
                vec!["11-03 09:00", "11-03 09:30", "11-03 11:00", "11-03 11:30"],
            ),
            (
                "buffer after a slot reaches into the busy time",
                Rules {
                    buffer_after: Duration::minutes(15),
                    buffer_before: Duration::minutes(15),
                    ..rules()
                },
                busy_10.clone(),
                NOW,
                vec!["11-03 09:00", "11-03 11:00", "11-03 11:30"],
            ),
            (
                "minimum notice rounds up to the next step of the window",
                Rules {
                    min_notice: Duration::hours(2) + Duration::minutes(40),
                    ..rules()
                },
                vec![],
                NOW,
                vec!["11-03 10:00", "11-03 10:30", "11-03 11:00", "11-03 11:30"],
            ),
            (
                "nothing in the past",
                rules(),
                vec![],
                "2026-11-03T11:10:00+01:00",
                vec!["11-03 11:30"],
            ),
            (
                "horizon ends the offer",
                Rules {
                    horizon: Duration::hours(3) + Duration::minutes(1),
                    ..rules()
                },
                vec![],
                NOW,
                vec!["11-03 09:00", "11-03 09:30", "11-03 10:00"],
            ),
            (
                "a 45-minute link steps by 30 minutes and must fit the window",
                Rules {
                    duration: Duration::minutes(45),
                    ..rules()
                },
                vec![],
                NOW,
                vec![
                    "11-03 09:00",
                    "11-03 09:30",
                    "11-03 10:00",
                    "11-03 10:30",
                    "11-03 11:00",
                ],
            ),
            (
                "a 15-minute link steps by 15 minutes",
                Rules {
                    duration: Duration::minutes(15),
                    windows: vec![WeeklyWindow {
                        weekday: Weekday::Tue,
                        start: clock("09:10"),
                        end: clock("10:00"),
                    }],
                    ..rules()
                },
                vec![],
                NOW,
                vec!["11-03 09:10", "11-03 09:25", "11-03 09:40"],
            ),
            (
                "booked links offer nothing",
                Rules {
                    booked: true,
                    ..rules()
                },
                vec![],
                NOW,
                vec![],
            ),
            (
                "suggestions: only future and free ones, notice ignored",
                Rules {
                    min_notice: Duration::days(2),
                    buffer_after: Duration::minutes(10),
                    suggestions: Some(vec![
                        (
                            "sug_a".into(),
                            (
                                t("2026-11-03T06:30:00+01:00"),
                                t("2026-11-03T07:00:00+01:00"),
                            ),
                        ),
                        (
                            "sug_b".into(),
                            (
                                t("2026-11-03T09:30:00+01:00"),
                                t("2026-11-03T10:00:00+01:00"),
                            ),
                        ),
                        (
                            "sug_c".into(),
                            (
                                t("2026-11-03T08:00:00+01:00"),
                                t("2026-11-03T08:30:00+01:00"),
                            ),
                        ),
                        (
                            "sug_d".into(),
                            (
                                t("2026-11-03T14:00:00+01:00"),
                                t("2026-11-03T14:30:00+01:00"),
                            ),
                        ),
                    ]),
                    ..rules()
                },
                busy_10.clone(),
                NOW,
                // sug_a is past, sug_b is blocked by the buffer after it.
                vec!["11-03 08:00", "11-03 14:00"],
            ),
            (
                "suggestions, all past: nothing (no fallback to the rules)",
                Rules {
                    suggestions: Some(vec![(
                        "sug_a".into(),
                        (
                            t("2026-11-03T06:30:00+01:00"),
                            t("2026-11-03T07:00:00+01:00"),
                        ),
                    )]),
                    ..rules()
                },
                vec![],
                NOW,
                vec![],
            ),
        ];
        for (name, r, busy, now, want) in cases {
            let got = bookable(&r, busy, t(DAY_FROM), t(DAY_TO), t(now));
            assert_eq!(starts(&got), want, "{name}");
            for s in &got {
                assert_eq!(
                    s.end - s.start,
                    if r.suggestions.is_some() {
                        Duration::minutes(30)
                    } else {
                        r.duration
                    },
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn range_and_weekend() {
        // Fri 6 Nov 11:00 to Mon 9 Nov 10:00: weekend has no windows.
        let got = bookable(
            &rules(),
            vec![],
            t("2026-11-06T11:00:00+01:00"),
            t("2026-11-09T10:00:00+01:00"),
            t(NOW),
        );
        assert_eq!(
            starts(&got),
            ["11-06 11:00", "11-06 11:30", "11-09 09:00", "11-09 09:30"]
        );
        // A slot that starts in the range but ends after it counts.
        let got = bookable(
            &rules(),
            vec![],
            t("2026-11-06T11:30:00+01:00"),
            t("2026-11-06T11:31:00+01:00"),
            t(NOW),
        );
        assert_eq!(starts(&got), ["11-06 11:30"]);
        assert_eq!(got[0].suggestion_id, None);
    }
}
