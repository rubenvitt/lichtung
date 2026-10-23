//! Expansion of a calendar object (master + overrides) into concrete
//! occurrences within a time window.
//!
//! Rules applied:
//!
//! * `RRULE` is evaluated in **wall-clock time** of the `DTSTART` zone, so a
//!   weekly 09:00 Europe/Berlin series stays at 09:00 across DST changes.
//! * Wall-clock times that do not exist (spring-forward gap) are shifted
//!   forward by the gap length, ambiguous times (fall back) use the first
//!   instance (RFC 5545 §3.3.5).
//! * `DTSTART` is always the first instance; `COUNT` counts it.
//! * `UNTIL` is compared as a date (DATE) or as an instant (UTC).
//! * `RDATE` adds instances (a PERIOD carries its own length), `EXDATE`
//!   removes them (matching by instant, so a different zone is fine; an
//!   `EXDATE` given as DATE removes all instances on that day).
//! * Overrides (`RECURRENCE-ID`) replace their instance wherever they were
//!   moved to, `STATUS:CANCELLED` drops it (or marks it, see
//!   [`ExpandOptions::include_cancelled`]). `RANGE=THISANDFUTURE` overrides
//!   shift and re-time all later instances (best effort).
//! * The duration is `DTEND − DTSTART` in wall-clock time (exact if the two
//!   use different zones) or `DURATION` (days nominal, time exact).

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use std::collections::HashSet;

use crate::error::Result;
use crate::event::{Event, EventEnd, EventStatus};
use crate::object::CalendarObject;
use crate::parser::Component;
use crate::rule::until_allows;
use crate::tz::{Zone, ZoneResolver};
use crate::value::{CalTime, PeriodEnd, RDate, TimeZoneRef};

/// Default cap for returned occurrences per query.
pub const DEFAULT_MAX_OCCURRENCES: usize = 5_000;
/// Cap for generated instances while scanning towards the window.
const MAX_SCANNED_INSTANCES: usize = 2_000_000;

/// Options for [`expand`].
#[derive(Debug, Clone)]
pub struct ExpandOptions {
    /// Zone for floating times and all-day events.
    pub default_tz: Tz,
    /// Maximum number of occurrences returned (protects against endless
    /// rules, e.g. `FREQ=MINUTELY` over a year).
    pub max_occurrences: usize,
    /// If `true`, cancelled occurrences are returned with
    /// [`Occurrence::cancelled`] set; otherwise they are dropped.
    pub include_cancelled: bool,
}

impl ExpandOptions {
    /// Defaults: [`DEFAULT_MAX_OCCURRENCES`], cancelled occurrences dropped.
    pub fn new(default_tz: Tz) -> Self {
        Self {
            default_tz,
            max_occurrences: DEFAULT_MAX_OCCURRENCES,
            include_cancelled: false,
        }
    }
}

/// An all-day span (end exclusive, as in iCalendar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AllDay {
    /// First day.
    pub start: NaiveDate,
    /// Day after the last day.
    pub end: NaiveDate,
}

/// One concrete occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// `UID` of the series.
    pub uid: Option<String>,
    /// Index of the `VEVENT` (within the `VCALENDAR`'s sub-components) that
    /// provides title, location, attendees etc.; see
    /// [`CalendarObject::event_at`].
    pub component_index: usize,
    /// Start instant.
    pub start: DateTime<Utc>,
    /// End instant (≥ start).
    pub end: DateTime<Utc>,
    /// Start in [`Occurrence::time_zone`].
    pub local_start: NaiveDateTime,
    /// End in [`Occurrence::time_zone`].
    pub local_end: NaiveDateTime,
    /// Zone of the local representation (IANA name, custom TZID, `UTC`, or
    /// the default zone for floating/all-day events).
    pub time_zone: String,
    /// Whether the time was floating (interpreted in the default zone).
    pub floating: bool,
    /// Set for all-day occurrences.
    pub all_day: Option<AllDay>,
    /// Original start of the instance within its series (`None` for
    /// non-recurring events). Pass it to the edit functions.
    pub recurrence_id: Option<CalTime>,
    /// Stable textual key of [`Occurrence::recurrence_id`]: `YYYYMMDD` for
    /// dates, the UTC instant `YYYYMMDDTHHMMSSZ` otherwise.
    pub recurrence_key: Option<String>,
    /// Whether this occurrence comes from an override `VEVENT`.
    pub is_override: bool,
    /// `STATUS` of the providing `VEVENT`.
    pub status: Option<EventStatus>,
    /// Whether the occurrence is cancelled (only returned with
    /// [`ExpandOptions::include_cancelled`]).
    pub cancelled: bool,
}

/// Result of [`expand`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expansion {
    /// Occurrences overlapping the window, sorted by start.
    pub occurrences: Vec<Occurrence>,
    /// `true` if the result was cut off by a limit.
    pub truncated: bool,
    /// Non-fatal problems (unparseable rule, unknown zone, …), German.
    pub warnings: Vec<String>,
}

/// Textual key of a recurrence id (see [`Occurrence::recurrence_key`]).
pub fn recurrence_key(rid: &CalTime, resolver: &ZoneResolver) -> String {
    match rid {
        CalTime::Date(d) => d.format("%Y%m%d").to_string(),
        _ => resolver
            .to_utc(rid)
            .naive_utc()
            .format("%Y%m%dT%H%M%SZ")
            .to_string(),
    }
}

/// Parses a key produced by [`recurrence_key`] back into a [`CalTime`]
/// (DATE or UTC DATE-TIME), suitable for the edit functions.
pub fn parse_recurrence_key(key: &str) -> Option<CalTime> {
    CalTime::parse(key, None, None).ok()
}

/// A length to add to an instance start.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Span {
    /// Added in wall-clock time.
    wall: Duration,
    /// Added to the instant afterwards.
    exact: Duration,
}

impl Span {
    /// Part added in wall-clock time.
    pub(crate) fn wall_part(&self) -> Duration {
        self.wall
    }

    /// Part added to the instant.
    pub(crate) fn exact_part(&self) -> Duration {
        self.exact
    }

    fn zero() -> Self {
        Span {
            wall: Duration::zero(),
            exact: Duration::zero(),
        }
    }
}

/// One raw instance of a series (before EXDATE/override handling).
#[derive(Debug, Clone)]
pub(crate) struct Instance {
    /// Recurrence id in the master's DTSTART format.
    pub rid: CalTime,
    /// Wall-clock start in the master zone.
    pub wall: NaiveDateTime,
    /// Start instant.
    pub start: DateTime<Utc>,
    /// Own length (RDATE PERIOD).
    pub span: Option<Span>,
    /// Generated by the RRULE (incl. DTSTART), not by an RDATE.
    pub from_rule: bool,
}

/// Generates the raw instances of a master event in chronological order.
pub(crate) struct SeriesGen {
    rule: Option<Box<dyn Iterator<Item = Instance>>>,
    rule_peek: Option<Instance>,
    rdates: std::collections::VecDeque<Instance>,
}

impl Iterator for SeriesGen {
    type Item = Instance;

    fn next(&mut self) -> Option<Instance> {
        if self.rule_peek.is_none() {
            self.rule_peek = self.rule.as_mut().and_then(|r| r.next());
        }
        let take_rule = match (&self.rule_peek, self.rdates.front()) {
            (Some(r), Some(d)) => {
                if r.start == d.start {
                    // Same instant from RRULE and RDATE: emit once.
                    self.rdates.pop_front();
                    true
                } else {
                    r.start < d.start
                }
            }
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => return None,
        };
        if take_rule {
            self.rule_peek.take()
        } else {
            self.rdates.pop_front()
        }
    }
}

/// Builds the instance generator for a master event.
pub(crate) fn series(
    master: &Event<&Component>,
    resolver: &ZoneResolver,
    warnings: &mut Vec<String>,
) -> Result<SeriesGen> {
    let dtstart = master.start()?;
    let zone = resolver.zone_of(&dtstart);
    let is_date = dtstart.is_date();

    let rule = match master.rrule() {
        Ok(r) => r,
        Err(e) => {
            warnings.push(format!("RRULE ignoriert: {e}"));
            None
        }
    };
    let rule_iter: Box<dyn Iterator<Item = Instance>> = match rule {
        Some(rule) => match rule.wall_clock_iter(dtstart.naive()) {
            Ok(iter) => {
                let until = rule.until();
                let z = zone.clone();
                let base = dtstart.clone();
                Box::new(
                    iter.take_while(move |w| match &until {
                        Some(u) => until_allows(u, *w, &|x| z.to_utc(x).naive_utc(), is_date),
                        None => true,
                    })
                    .map({
                        let z = zone.clone();
                        move |w| Instance {
                            rid: base.with_naive(w),
                            wall: w,
                            start: z.to_utc(w),
                            span: None,
                            from_rule: true,
                        }
                    }),
                )
            }
            Err(e) => {
                warnings.push(format!("RRULE ignoriert: {e}"));
                Box::new(std::iter::once(single_instance(&dtstart, &zone)))
            }
        },
        None => Box::new(std::iter::once(single_instance(&dtstart, &zone))),
    };

    let mut rdates: Vec<Instance> = Vec::new();
    match master.rdates() {
        Ok(list) => {
            for rd in list {
                rdates.push(rdate_instance(&rd, &dtstart, &zone, resolver));
            }
        }
        Err(e) => warnings.push(format!("RDATE ignoriert: {e}")),
    }
    rdates.sort_by_key(|i| i.start);
    rdates.dedup_by_key(|i| i.start);

    Ok(SeriesGen {
        rule: Some(rule_iter),
        rule_peek: None,
        rdates: rdates.into(),
    })
}

fn single_instance(dtstart: &CalTime, zone: &Zone) -> Instance {
    let wall = dtstart.naive();
    Instance {
        rid: dtstart.clone(),
        wall,
        start: zone.to_utc(wall),
        span: None,
        from_rule: true,
    }
}

fn rdate_instance(rd: &RDate, dtstart: &CalTime, zone: &Zone, resolver: &ZoneResolver) -> Instance {
    let t = rd.start();
    let wall = match (dtstart.is_date(), t.is_date()) {
        (_, true) => t.date().and_time(dtstart.naive().time()),
        (true, false) => t.naive(),
        (false, false) => zone.to_local(resolver.to_utc(t)),
    };
    let start = if t.is_date() || dtstart.is_date() {
        zone.to_utc(wall)
    } else {
        resolver.to_utc(t)
    };
    let span = match rd {
        RDate::Time(_) => None,
        RDate::Period(p) => Some(match &p.end {
            PeriodEnd::End(e) => Span {
                wall: Duration::zero(),
                exact: (resolver.to_utc(e) - resolver.to_utc(&p.start)).max(Duration::zero()),
            },
            PeriodEnd::Duration(d) => Span {
                wall: Duration::days(d.nominal_days()),
                exact: Duration::seconds(d.exact_seconds()),
            },
        }),
    };
    Instance {
        rid: dtstart.with_naive(wall),
        wall,
        start,
        span,
        from_rule: false,
    }
}

/// The default length of instances of `ev` (from DTEND/DURATION).
pub(crate) fn event_span(ev: &Event<&Component>, resolver: &ZoneResolver) -> Result<Span> {
    let start = ev.start()?;
    Ok(match ev.end()? {
        EventEnd::End(end) => {
            let same_zone = match (start.zone(), end.zone()) {
                (None, None) => true,
                (Some(a), Some(b)) => a == b,
                _ => false,
            };
            if start.is_date() || end.is_date() {
                Span {
                    wall: Duration::days((end.date() - start.date()).num_days().max(0)),
                    exact: Duration::zero(),
                }
            } else if same_zone {
                Span {
                    wall: (end.naive() - start.naive()).max(Duration::zero()),
                    exact: Duration::zero(),
                }
            } else {
                Span {
                    wall: Duration::zero(),
                    exact: (resolver.to_utc(&end) - resolver.to_utc(&start)).max(Duration::zero()),
                }
            }
        }
        EventEnd::Duration(d) => Span {
            wall: Duration::days(d.nominal_days()),
            exact: Duration::seconds(d.exact_seconds()),
        },
        EventEnd::Implicit => {
            if start.is_date() {
                Span {
                    wall: Duration::days(1),
                    exact: Duration::zero(),
                }
            } else {
                Span::zero()
            }
        }
    })
}

/// Computes the end instant for a wall-clock start.
fn end_of(zone: &Zone, wall: NaiveDateTime, start: DateTime<Utc>, span: Span) -> DateTime<Utc> {
    let end = if span.wall.is_zero() {
        start
    } else if zone.to_local(start) != wall {
        // The start fell into a DST gap and was moved forward; move the
        // whole occurrence instead of shortening it.
        start + span.wall
    } else {
        zone.to_utc(wall + span.wall)
    };
    (end + span.exact).max(start)
}

struct RidIndex {
    instants: HashSet<DateTime<Utc>>,
    dates: HashSet<NaiveDate>,
    datetime_dates: HashSet<NaiveDate>,
}

impl RidIndex {
    fn new<'a>(values: impl Iterator<Item = &'a CalTime>, resolver: &ZoneResolver) -> Self {
        let mut idx = RidIndex {
            instants: HashSet::new(),
            dates: HashSet::new(),
            datetime_dates: HashSet::new(),
        };
        for v in values {
            if v.is_date() {
                idx.dates.insert(v.date());
            } else {
                idx.instants.insert(resolver.to_utc(v));
                idx.datetime_dates.insert(v.date());
            }
        }
        idx
    }

    fn contains(&self, inst: &Instance) -> bool {
        if inst.rid.is_date() {
            self.dates.contains(&inst.wall.date())
                || self.datetime_dates.contains(&inst.wall.date())
        } else {
            self.instants.contains(&inst.start) || self.dates.contains(&inst.wall.date())
        }
    }
}

fn overlaps(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> bool {
    if end > start {
        start < to && end > from
    } else {
        start >= from && start < to
    }
}

fn zone_label(t: &CalTime, zone: &Zone) -> (String, bool) {
    match t.zone() {
        Some(TimeZoneRef::Utc) => ("UTC".to_string(), false),
        Some(TimeZoneRef::Floating) => (zone.name(), true),
        _ => (zone.name(), false),
    }
}

struct ThisAndFuture {
    rid_start: DateTime<Utc>,
    shift: Duration,
    span: Span,
    index: usize,
}

/// Expands `object` into the occurrences overlapping `[from, to)`.
pub fn expand(
    object: &CalendarObject,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    options: &ExpandOptions,
) -> Result<Expansion> {
    let resolver = object.resolver(options.default_tz);
    let mut out = Expansion::default();
    let uid = object.uid();

    // Overrides.
    struct Ov {
        index: usize,
        rid: CalTime,
        this_and_future: bool,
    }
    let mut overrides: Vec<Ov> = Vec::new();
    for index in object.override_indices() {
        let Some(ev) = object.event_at(index) else {
            continue;
        };
        match ev.recurrence_id() {
            Ok(Some(r)) => overrides.push(Ov {
                index,
                rid: r.value,
                this_and_future: r.this_and_future,
            }),
            Ok(None) => {}
            Err(e) => out.warnings.push(format!("Ausnahme ignoriert: {e}")),
        }
    }

    let mut exdates = RidIndex::new(std::iter::empty(), &resolver);
    let master_index = object.master_index();
    if let Some(mi) = master_index {
        let master = object.event_at(mi).expect("master index is a VEVENT");
        report_zone_fallbacks(&master, &resolver, &mut out.warnings);
        let dtstart = master.start()?;
        let zone = resolver.zone_of(&dtstart);
        let (zone_name, floating) = zone_label(&dtstart, &zone);
        let span = event_span(&master, &resolver)?;
        let master_cancelled = master.is_cancelled();
        let master_status = master.status();
        let recurring = master.is_recurring();
        let ex_list = master.exdates().unwrap_or_else(|e| {
            out.warnings.push(format!("EXDATE ignoriert: {e}"));
            Vec::new()
        });
        exdates = RidIndex::new(ex_list.iter(), &resolver);
        let overridden = RidIndex::new(
            overrides
                .iter()
                .filter(|o| !o.this_and_future)
                .map(|o| &o.rid),
            &resolver,
        );

        // RANGE=THISANDFUTURE overrides, sorted by their recurrence id.
        let mut taf: Vec<ThisAndFuture> = Vec::new();
        for o in overrides.iter().filter(|o| o.this_and_future) {
            let ev = object
                .event_at(o.index)
                .expect("override index is a VEVENT");
            let Ok(ostart) = ev.start() else { continue };
            let rid_wall = zone.to_local(resolver.to_utc(&o.rid));
            let start_wall = zone.to_local(resolver.to_utc(&ostart));
            taf.push(ThisAndFuture {
                rid_start: resolver.to_utc(&o.rid),
                shift: start_wall - rid_wall,
                span: event_span(&ev, &resolver).unwrap_or(span),
                index: o.index,
            });
        }
        taf.sort_by_key(|t| t.rid_start);
        let max_shift = taf
            .iter()
            .map(|t| t.shift.num_seconds().abs())
            .max()
            .unwrap_or(0);
        let stop_at = to + Duration::seconds(max_shift) + Duration::days(1);

        let gen = series(&master, &resolver, &mut out.warnings)?;
        for (scanned, inst) in gen.enumerate() {
            if scanned >= MAX_SCANNED_INSTANCES {
                out.truncated = true;
                out.warnings
                    .push("Serie zu lang: Expansion abgebrochen (Obergrenze erreicht)".into());
                break;
            }
            if inst.start >= stop_at {
                break;
            }
            if exdates.contains(&inst) {
                continue;
            }
            // Non-recurring masters can still be replaced by an override with
            // the same RECURRENCE-ID (seen in the wild); then the override wins.
            if overridden.contains(&inst) {
                continue;
            }
            let applied = taf.iter().rev().find(|t| t.rid_start < inst.start);
            if taf.iter().any(|t| t.rid_start == inst.start) {
                // The THISANDFUTURE override itself is emitted below.
                continue;
            }
            let (wall, inst_span, index) = match applied {
                Some(t) => (inst.wall + t.shift, t.span, t.index),
                None => (inst.wall, inst.span.unwrap_or(span), mi),
            };
            let start = if applied.is_some() {
                zone.to_utc(wall)
            } else {
                inst.start
            };
            let end = end_of(&zone, wall, start, inst_span);
            if !overlaps(start, end, from, to) {
                continue;
            }
            let cancelled = master_cancelled;
            if cancelled && !options.include_cancelled {
                continue;
            }
            let (status, is_override) = match applied {
                Some(t) => (object.event_at(t.index).and_then(|e| e.status()), true),
                None => (master_status.clone(), false),
            };
            out.occurrences.push(make_occurrence(
                &uid,
                index,
                start,
                end,
                &zone,
                &zone_name,
                floating,
                dtstart.is_date(),
                recurring.then(|| inst.rid.clone()),
                &resolver,
                is_override,
                status,
                cancelled,
            ));
        }
    }

    for o in &overrides {
        let ev = object
            .event_at(o.index)
            .expect("override index is a VEVENT");
        if master_index.is_some() && exdates_contains_rid(&exdates, &o.rid, &resolver) {
            continue;
        }
        report_zone_fallbacks(&ev, &resolver, &mut out.warnings);
        let start_t = ev.start().unwrap_or_else(|_| o.rid.clone());
        let zone = resolver.zone_of(&start_t);
        let (zone_name, floating) = zone_label(&start_t, &zone);
        let span = event_span(&ev, &resolver).unwrap_or(Span::zero());
        let wall = start_t.naive();
        let start = zone.to_utc(wall);
        let end = end_of(&zone, wall, start, span);
        if !overlaps(start, end, from, to) {
            continue;
        }
        let cancelled = ev.is_cancelled();
        if cancelled && !options.include_cancelled {
            continue;
        }
        out.occurrences.push(make_occurrence(
            &uid,
            o.index,
            start,
            end,
            &zone,
            &zone_name,
            floating,
            start_t.is_date(),
            Some(o.rid.clone()),
            &resolver,
            true,
            ev.status(),
            cancelled,
        ));
    }

    out.occurrences.sort_by_key(|o| (o.start, o.end));
    if out.occurrences.len() > options.max_occurrences {
        out.occurrences.truncate(options.max_occurrences);
        out.truncated = true;
    }
    out.warnings.dedup();
    Ok(out)
}

fn exdates_contains_rid(ex: &RidIndex, rid: &CalTime, resolver: &ZoneResolver) -> bool {
    if rid.is_date() {
        ex.dates.contains(&rid.date()) || ex.datetime_dates.contains(&rid.date())
    } else {
        ex.instants.contains(&resolver.to_utc(rid)) || ex.dates.contains(&rid.date())
    }
}

fn report_zone_fallbacks(
    ev: &Event<&Component>,
    resolver: &ZoneResolver,
    warnings: &mut Vec<String>,
) {
    for p in ev.component().properties() {
        if let Some(tzid) = p.param("TZID") {
            if resolver.resolve(tzid).1 == crate::tz::ResolvedBy::Fallback {
                warnings.push(format!(
                    "Unbekannte Zeitzone „{tzid}“, verwende {}",
                    resolver.default_zone().name()
                ));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn make_occurrence(
    uid: &Option<String>,
    component_index: usize,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    zone: &Zone,
    zone_name: &str,
    floating: bool,
    is_date: bool,
    rid: Option<CalTime>,
    resolver: &ZoneResolver,
    is_override: bool,
    status: Option<EventStatus>,
    cancelled: bool,
) -> Occurrence {
    let local_start = zone.to_local(start);
    let local_end = zone.to_local(end);
    let all_day = is_date.then(|| AllDay {
        start: local_start.date(),
        end: if local_end.date() > local_start.date() {
            local_end.date()
        } else {
            local_start.date().succ_opt().unwrap_or(local_start.date())
        },
    });
    let recurrence_key = rid.as_ref().map(|r| recurrence_key(r, resolver));
    Occurrence {
        uid: uid.clone(),
        component_index,
        start,
        end,
        local_start,
        local_end,
        time_zone: zone_name.to_string(),
        floating,
        all_day,
        recurrence_id: rid,
        recurrence_key,
        is_override,
        status,
        cancelled,
    }
}
