//! Scoped editing of events and series: "only this", "this and following",
//! "all". All functions are pure: they take a [`CalendarObject`] and return
//! the resulting object(s); the input is never modified.

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;

use crate::error::{Error, Result};
use crate::event::{Event, EventEnd, EventStatus, Transparency};
use crate::expand::{event_span, recurrence_key, series};
use crate::object::{same_instant, CalendarObject};
use crate::parser::Component;
use crate::rule::RecurRule;
use crate::tz::{Zone, ZoneResolver};
use crate::value::{CalTime, IcalDuration, PeriodEnd, RDate, TimeZoneRef};

/// Geltungsbereich of an edit or delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditScope {
    /// Only the selected occurrence ("nur dieser").
    This,
    /// The selected occurrence and all later ones ("dieser und folgende").
    ThisAndFuture,
    /// The whole series ("alle"). Also used for non-recurring events.
    All,
}

/// Changes to apply. `None` leaves a field untouched; for optional fields
/// `Some(None)` removes the property.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventChanges {
    /// `SUMMARY`
    pub summary: Option<Option<String>>,
    /// `LOCATION`
    pub location: Option<Option<String>>,
    /// `DESCRIPTION`
    pub description: Option<Option<String>>,
    /// `URL`
    pub url: Option<Option<String>>,
    /// `STATUS`
    pub status: Option<Option<EventStatus>>,
    /// `TRANSP`
    pub transparency: Option<Transparency>,
    /// `X-APPLE-TRAVEL-DURATION`
    pub travel_duration: Option<Option<IcalDuration>>,
    /// New start **of the selected occurrence**. For [`EditScope::All`] and
    /// [`EditScope::ThisAndFuture`] the series is moved by the same
    /// wall-clock difference (and may change zone or all-day state).
    /// UTC values are converted into the event's own zone (or into floating
    /// time for floating events), so a series keeps its DST behaviour.
    pub start: Option<CalTime>,
    /// New end of the selected occurrence (written as `DTEND`, or as
    /// `DURATION` if the event used `DURATION`).
    pub end: Option<CalTime>,
    /// New recurrence rule (`Some(None)` ends recurrence). Not allowed with
    /// [`EditScope::This`].
    ///
    /// `COUNT` and `UNTIL` are compared with the series' rule before the
    /// edit (LIC-57):
    /// - An unchanged `COUNT` still counts the whole series: with
    ///   [`EditScope::ThisAndFuture`] the new series gets the remainder
    ///   (as without a rule change). A changed `COUNT` counts from the
    ///   selected occurrence on and is taken as it is. A sender may thus
    ///   pass either the series' count or the remainder; the result is the
    ///   same, nothing is subtracted twice.
    /// - An unchanged `UNTIL` moves along with the series when its start
    ///   changes. A changed `UNTIL` is taken as it is.
    pub rrule: Option<Option<RecurRule>>,
}

impl EventChanges {
    fn has_time_change(&self) -> bool {
        self.start.is_some() || self.end.is_some()
    }
}

/// Context for edits.
#[derive(Debug, Clone)]
pub struct EditContext {
    /// Written to `DTSTAMP` / `LAST-MODIFIED`.
    pub now: DateTime<Utc>,
    /// Zone for floating times and all-day dates.
    pub default_tz: Tz,
    /// `UID` for the new series created by a "this and following" split.
    pub new_uid: Option<String>,
}

impl EditContext {
    /// Creates a context.
    pub fn new(now: DateTime<Utc>, default_tz: Tz) -> Self {
        Self {
            now,
            default_tz,
            new_uid: None,
        }
    }

    /// Builder: sets the `UID` for a series split.
    pub fn with_new_uid(mut self, uid: impl Into<String>) -> Self {
        self.new_uid = Some(uid.into());
        self
    }
}

/// Result of an edit or delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOutcome {
    /// The original object after the change; `None` means: delete it.
    pub updated: Option<CalendarObject>,
    /// A new object to create (new series after a split).
    pub created: Option<CalendarObject>,
}

/// Edits an event or series.
///
/// `occurrence` is the [`crate::Occurrence::recurrence_id`] of the selected
/// occurrence (any equivalent DATE/DATE-TIME works, e.g. the parsed
/// [`crate::Occurrence::recurrence_key`]). It is required for
/// [`EditScope::This`] and [`EditScope::ThisAndFuture`] on recurring events;
/// for non-recurring events every scope behaves like [`EditScope::All`].
pub fn edit_event(
    object: &CalendarObject,
    occurrence: Option<&CalTime>,
    scope: EditScope,
    changes: &EventChanges,
    ctx: &EditContext,
) -> Result<EditOutcome> {
    let resolver = object.resolver(ctx.default_tz);
    let mut obj = object.clone();
    let normalized = normalize_changes(&obj, changes, &resolver);
    // The rule the sender saw (before a split), see `EventChanges::rrule`.
    let seen_rule = obj.master().and_then(|m| m.rrule().ok().flatten());
    let changes = &normalized;

    let Some(master_idx) = obj.master_index() else {
        // Only overrides (e.g. invitation to a single occurrence).
        let idx = pick_override(&obj, occurrence, &resolver)?;
        if changes.rrule.is_some() {
            return Err(Error::InvalidEdit(
                "eine einzelne Ausnahme kann keine Wiederholung bekommen".into(),
            ));
        }
        let mut ev = obj.event_at_mut(idx).expect("override index");
        apply_fields(&mut ev, changes);
        apply_times_single(&mut ev, changes, &resolver)?;
        ev.bump_sequence();
        ev.touch(ctx.now);
        return Ok(EditOutcome {
            updated: Some(obj),
            created: None,
        });
    };

    let recurring = obj.event_at(master_idx).expect("master").is_recurring();
    let scope = if recurring { scope } else { EditScope::All };

    match scope {
        EditScope::All => {
            apply_all(
                &mut obj,
                occurrence,
                changes,
                seen_rule.as_ref(),
                ctx,
                &resolver,
            )?;
            Ok(EditOutcome {
                updated: Some(obj),
                created: None,
            })
        }
        EditScope::This => {
            let rid = occurrence
                .ok_or_else(|| Error::InvalidEdit("für „nur dieser“ fehlt das Vorkommen".into()))?;
            if changes.rrule.is_some() {
                return Err(Error::InvalidEdit(
                    "die Wiederholung lässt sich nur für alle oder folgende Termine ändern".into(),
                ));
            }
            let idx = ensure_override(&mut obj, rid, &resolver)?;
            let mut ev = obj.event_at_mut(idx).expect("override index");
            apply_fields(&mut ev, changes);
            apply_times_single(&mut ev, changes, &resolver)?;
            ev.touch(ctx.now);
            Ok(EditOutcome {
                updated: Some(obj),
                created: None,
            })
        }
        EditScope::ThisAndFuture => {
            let rid = occurrence.ok_or_else(|| {
                Error::InvalidEdit("für „dieser und folgende“ fehlt das Vorkommen".into())
            })?;
            let split = locate(&obj, rid, &resolver)?;
            if split.is_first {
                apply_all(
                    &mut obj,
                    Some(rid),
                    changes,
                    seen_rule.as_ref(),
                    ctx,
                    &resolver,
                )?;
                return Ok(EditOutcome {
                    updated: Some(obj),
                    created: None,
                });
            }
            let new_uid = ctx.new_uid.clone().ok_or_else(|| {
                Error::InvalidEdit("für das Teilen der Serie wird eine neue UID benötigt".into())
            })?;
            let mut new_obj = split_series(&mut obj, &split, Some(&new_uid), ctx, &resolver)?
                .expect("split with uid yields a new series");
            let new_resolver = new_obj.resolver(ctx.default_tz);
            apply_all(
                &mut new_obj,
                Some(&split.rid),
                changes,
                seen_rule.as_ref(),
                ctx,
                &new_resolver,
            )?;
            // The new series starts fresh.
            if let Some(mut m) = new_obj.master_mut() {
                m.set_sequence(0);
            }
            Ok(EditOutcome {
                updated: Some(obj),
                created: Some(new_obj),
            })
        }
    }
}

/// Deletes an occurrence, the rest of a series, or the whole object.
///
/// * [`EditScope::This`]: adds an `EXDATE` and removes a matching override.
/// * [`EditScope::ThisAndFuture`]: ends the series before the occurrence
///   (`UNTIL` or `COUNT`); deleting from the first occurrence deletes all.
/// * [`EditScope::All`] and non-recurring events: `updated` is `None`.
pub fn delete_event(
    object: &CalendarObject,
    occurrence: Option<&CalTime>,
    scope: EditScope,
    ctx: &EditContext,
) -> Result<EditOutcome> {
    let resolver = object.resolver(ctx.default_tz);
    let mut obj = object.clone();
    let gone = EditOutcome {
        updated: None,
        created: None,
    };

    let Some(master_idx) = obj.master_index() else {
        if scope == EditScope::All {
            return Ok(gone);
        }
        let idx = pick_override(&obj, occurrence, &resolver)?;
        obj.calendar_mut().components_mut().remove(idx);
        if obj.events().next().is_none() {
            return Ok(gone);
        }
        return Ok(EditOutcome {
            updated: Some(obj),
            created: None,
        });
    };
    let recurring = obj.event_at(master_idx).expect("master").is_recurring();
    if !recurring || scope == EditScope::All {
        return Ok(gone);
    }
    let rid =
        occurrence.ok_or_else(|| Error::InvalidEdit("zum Löschen fehlt das Vorkommen".into()))?;
    let split = locate(&obj, rid, &resolver)?;

    match scope {
        EditScope::This => {
            if let Some(i) = obj.find_override(&split.rid, &resolver) {
                obj.calendar_mut().components_mut().remove(i);
            }
            let mut m = obj.master_mut().expect("master");
            m.add_exdate(&split.rid);
            m.bump_sequence();
            m.touch(ctx.now);
            Ok(EditOutcome {
                updated: Some(obj),
                created: None,
            })
        }
        EditScope::ThisAndFuture => {
            if split.is_first {
                return Ok(gone);
            }
            split_series(&mut obj, &split, None, ctx, &resolver)?;
            Ok(EditOutcome {
                updated: Some(obj),
                created: None,
            })
        }
        EditScope::All => Ok(gone),
    }
}

/// Converts UTC start/end values into the zone of the event (see
/// [`EventChanges::start`]).
fn normalize_changes(
    obj: &CalendarObject,
    changes: &EventChanges,
    resolver: &ZoneResolver,
) -> EventChanges {
    let reference = obj
        .master()
        .or_else(|| obj.overrides().into_iter().next())
        .and_then(|e| e.start().ok());
    let convert = |t: &CalTime| -> CalTime {
        let Some(r) = &reference else {
            return t.clone();
        };
        match (t.zone(), r.zone()) {
            (Some(TimeZoneRef::Utc), Some(TimeZoneRef::Tzid(id))) => {
                let local = resolver.resolve(id).0.to_local(resolver.to_utc(t));
                CalTime::zoned(local, id.clone())
            }
            (Some(TimeZoneRef::Utc), Some(TimeZoneRef::Floating)) => {
                CalTime::floating(resolver.zone_of(r).to_local(resolver.to_utc(t)))
            }
            _ => t.clone(),
        }
    };
    let mut out = changes.clone();
    out.start = changes.start.as_ref().map(convert);
    out.end = changes.end.as_ref().map(convert);
    out
}

// ---------------------------------------------------------------------------
// Locating occurrences

/// A located occurrence of the series.
struct Located {
    /// Recurrence id in master form (kind and zone of DTSTART).
    rid: CalTime,
    /// Its original start instant.
    start: DateTime<Utc>,
    /// Whether it is the first instance (= DTSTART).
    is_first: bool,
    /// Number of RRULE-generated instances (incl. DTSTART) before it.
    rule_instances_before: u32,
}

fn master_zone(master: &Event<&Component>, resolver: &ZoneResolver) -> Result<(CalTime, Zone)> {
    let start = master.start()?;
    let zone = resolver.zone_of(&start);
    Ok((start, zone))
}

/// Expresses `rid` in the master's DTSTART form.
fn to_master_form(
    rid: &CalTime,
    master_start: &CalTime,
    zone: &Zone,
    resolver: &ZoneResolver,
) -> CalTime {
    match (master_start.is_date(), rid.is_date()) {
        (true, true) => rid.clone(),
        (true, false) => CalTime::Date(zone.to_local(resolver.to_utc(rid)).date()),
        (false, true) => master_start.with_naive(rid.date().and_time(master_start.naive().time())),
        (false, false) => master_start.with_naive(zone.to_local(resolver.to_utc(rid))),
    }
}

fn locate(obj: &CalendarObject, rid: &CalTime, resolver: &ZoneResolver) -> Result<Located> {
    let master = obj.master().expect("caller checked master");
    let (mstart, zone) = master_zone(&master, resolver)?;
    let form = to_master_form(rid, &mstart, &zone, resolver);
    let target = zone.to_utc(form.naive());
    let first = zone.to_utc(mstart.naive());
    let mut warnings = Vec::new();
    let mut before = 0u32;
    let mut found = false;
    for (n, inst) in series(&master, resolver, &mut warnings)?.enumerate() {
        if n > 2_000_000 || inst.start > target {
            break;
        }
        if inst.start == target {
            found = true;
            break;
        }
        if inst.from_rule {
            before += 1;
        }
    }
    if !found && obj.find_override(&form, resolver).is_none() {
        return Err(Error::UnknownOccurrence(recurrence_key(rid, resolver)));
    }
    Ok(Located {
        rid: form,
        start: target,
        is_first: target <= first,
        rule_instances_before: before,
    })
}

fn pick_override(
    obj: &CalendarObject,
    occurrence: Option<&CalTime>,
    resolver: &ZoneResolver,
) -> Result<usize> {
    match occurrence {
        Some(rid) => obj
            .find_override(rid, resolver)
            .ok_or_else(|| Error::UnknownOccurrence(recurrence_key(rid, resolver))),
        None => obj
            .override_indices()
            .first()
            .copied()
            .ok_or_else(|| Error::InvalidObject("keine VEVENT-Komponente vorhanden".into())),
    }
}

// ---------------------------------------------------------------------------
// "Only this"

/// Returns the index of the override for `rid`, creating it from the master
/// if needed (fields copied, `RRULE`/`RDATE`/`EXDATE` removed, `SEQUENCE`
/// increased).
fn ensure_override(
    obj: &mut CalendarObject,
    rid: &CalTime,
    resolver: &ZoneResolver,
) -> Result<usize> {
    let loc = locate(obj, rid, resolver)?;
    if let Some(i) = obj.find_override(&loc.rid, resolver) {
        let mut ev = obj.event_at_mut(i).expect("override index");
        ev.bump_sequence();
        return Ok(i);
    }
    let master = obj.master().expect("master");
    let (mstart, zone) = master_zone(&master, resolver)?;
    let span = event_span(&master, resolver)?;
    let end = master.end()?;
    let seq = master.sequence();
    let mut comp = master.component().clone();
    for name in ["RRULE", "RDATE", "EXDATE", "EXRULE"] {
        comp.remove_properties(name);
    }
    let mut ev = Event::new(comp);
    ev.set_recurrence_id(Some(&loc.rid));
    ev.set_start(&loc.rid);
    let wall = loc.rid.naive();
    if let EventEnd::End(old_end) = end {
        let end_wall = if span.exact_part().is_zero() {
            wall + span.wall_part()
        } else {
            zone.to_local(zone.to_utc(wall + span.wall_part()) + span.exact_part())
        };
        let new_end = if old_end.is_date() {
            CalTime::Date(end_wall.date())
        } else if old_end.zone() == mstart.zone() {
            old_end.with_naive(end_wall)
        } else {
            let end_zone = resolver.zone_of(&old_end);
            old_end.with_naive(end_zone.to_local(zone.to_utc(end_wall)))
        };
        ev.set_end(&EventEnd::End(new_end));
    }
    ev.set_sequence(seq + 1);
    let comps = obj.calendar_mut().components_mut();
    let insert_at = comps
        .iter()
        .rposition(|c| c.is("VEVENT"))
        .map(|i| i + 1)
        .unwrap_or(comps.len());
    comps.insert(insert_at, ev.into_inner());
    Ok(insert_at)
}

/// Applies start/end changes to a single (non-series) event or override.
fn apply_times_single<C: std::borrow::BorrowMut<Component>>(
    ev: &mut Event<C>,
    changes: &EventChanges,
    resolver: &ZoneResolver,
) -> Result<()> {
    if !changes.has_time_change() {
        return Ok(());
    }
    let old_start = ev.start()?;
    let old_end = ev.end()?;
    if let Some(new_start) = &changes.start {
        ev.set_start(new_start);
    }
    let new_start = changes.start.clone().unwrap_or_else(|| old_start.clone());
    match &changes.end {
        Some(new_end) => set_end_keeping_style(ev, &old_end, &new_start, new_end),
        None => {
            // Keep the length.
            let new_end = shifted_end(&old_start, &old_end, &new_start, resolver);
            if let Some(e) = new_end {
                ev.set_end(&e);
            }
        }
    }
    Ok(())
}

/// Writes `new_end`, as `DURATION` if the event used `DURATION`.
fn set_end_keeping_style<C: std::borrow::BorrowMut<Component>>(
    ev: &mut Event<C>,
    old_end: &EventEnd,
    new_start: &CalTime,
    new_end: &CalTime,
) {
    if matches!(old_end, EventEnd::Duration(_)) {
        let diff = new_end.naive() - new_start.naive();
        let d = if new_start.is_date() {
            IcalDuration::from_parts(diff.num_days(), 0)
        } else {
            IcalDuration::from_seconds(diff.num_seconds())
        };
        ev.set_end(&EventEnd::Duration(d));
    } else {
        ev.set_end(&EventEnd::End(new_end.clone()));
    }
}

/// Computes the end for a moved start that keeps the old length. Returns
/// `None` if the end can stay as it is (DURATION, implicit).
fn shifted_end(
    old_start: &CalTime,
    old_end: &EventEnd,
    new_start: &CalTime,
    resolver: &ZoneResolver,
) -> Option<EventEnd> {
    let kind_changed = old_start.is_date() != new_start.is_date();
    match old_end {
        EventEnd::End(e) => {
            if kind_changed {
                return Some(EventEnd::End(default_end(new_start)));
            }
            if e.zone() == old_start.zone() || e.is_date() {
                let len = e.naive() - old_start.naive();
                let mut ne = e.with_naive(new_start.naive() + len);
                if let (Some(z), false) = (new_start.zone(), e.is_date()) {
                    // Follow a zone change of the start.
                    if e.zone() == old_start.zone() {
                        ne = CalTime::DateTime {
                            local: ne.naive(),
                            zone: z.clone(),
                        };
                    }
                }
                Some(EventEnd::End(ne))
            } else {
                let exact = resolver.to_utc(e) - resolver.to_utc(old_start);
                let end_zone = resolver.zone_of(e);
                let instant = resolver.to_utc(new_start) + exact;
                Some(EventEnd::End(e.with_naive(end_zone.to_local(instant))))
            }
        }
        EventEnd::Duration(_) | EventEnd::Implicit => {
            kind_changed.then(|| EventEnd::End(default_end(new_start)))
        }
    }
}

/// Default end when switching between all-day and timed: one day / one hour.
fn default_end(start: &CalTime) -> CalTime {
    match start {
        CalTime::Date(d) => CalTime::Date(d.succ_opt().unwrap_or(*d)),
        CalTime::DateTime { local, zone } => CalTime::DateTime {
            local: *local + Duration::hours(1),
            zone: zone.clone(),
        },
    }
}

// ---------------------------------------------------------------------------
// Field changes

fn apply_fields<C: std::borrow::BorrowMut<Component>>(ev: &mut Event<C>, changes: &EventChanges) {
    if let Some(v) = &changes.summary {
        ev.set_summary(v.as_deref());
    }
    if let Some(v) = &changes.location {
        ev.set_location(v.as_deref());
    }
    if let Some(v) = &changes.description {
        ev.set_description(v.as_deref());
    }
    if let Some(v) = &changes.url {
        ev.set_url(v.as_deref());
    }
    if let Some(v) = &changes.status {
        ev.set_status(v.as_ref());
    }
    if let Some(v) = changes.transparency {
        ev.set_transparency(v);
    }
    if let Some(v) = changes.travel_duration {
        ev.set_travel_duration(v);
    }
}

/// Snapshot of the master's fields before an edit, to propagate changes to
/// overrides that still carried the old master value.
struct FieldSnapshot {
    summary: Option<String>,
    location: Option<String>,
    description: Option<String>,
    url: Option<String>,
    status: Option<EventStatus>,
    transparency: Transparency,
}

impl FieldSnapshot {
    fn of<C: std::borrow::Borrow<Component>>(ev: &Event<C>) -> Self {
        Self {
            summary: ev.summary(),
            location: ev.location(),
            description: ev.description(),
            url: ev.url(),
            status: ev.status(),
            transparency: ev.transparency(),
        }
    }

    /// The subset of `changes` that applies to an override with the values `ov`.
    fn filter_for(&self, ov: &FieldSnapshot, changes: &EventChanges) -> EventChanges {
        EventChanges {
            summary: changes
                .summary
                .clone()
                .filter(|_| ov.summary == self.summary),
            location: changes
                .location
                .clone()
                .filter(|_| ov.location == self.location),
            description: changes
                .description
                .clone()
                .filter(|_| ov.description == self.description),
            url: changes.url.clone().filter(|_| ov.url == self.url),
            status: changes.status.clone().filter(|_| ov.status == self.status),
            transparency: changes
                .transparency
                .filter(|_| ov.transparency == self.transparency),
            travel_duration: None,
            start: None,
            end: None,
            rrule: None,
        }
    }
}

// ---------------------------------------------------------------------------
// "All"

/// `seen_rule` is the series' rule as the sender saw it (before a split);
/// `obj` may be the new series of a split, whose rule already carries the
/// remaining `COUNT`.
fn apply_all(
    obj: &mut CalendarObject,
    occurrence: Option<&CalTime>,
    changes: &EventChanges,
    seen_rule: Option<&RecurRule>,
    ctx: &EditContext,
    resolver: &ZoneResolver,
) -> Result<()> {
    let master = obj.master().expect("caller checked master");
    let snapshot = FieldSnapshot::of(&master);
    let (old_start, old_zone) = master_zone(&master, resolver)?;
    let old_end = master.end()?;
    let recurring = master.is_recurring();

    // Current start of the selected occurrence (in the master's wall clock).
    let occ_current: CalTime = match occurrence.filter(|_| recurring) {
        Some(rid) => {
            let loc = locate(obj, rid, resolver)?;
            match obj.find_override(&loc.rid, resolver) {
                Some(i) => {
                    let ov = obj.event_at(i).expect("override");
                    let s = ov.start()?;
                    to_master_form(&s, &old_start, &old_zone, resolver)
                }
                None => loc.rid,
            }
        }
        None => old_start.clone(),
    };

    // New master start.
    let new_start = match &changes.start {
        Some(ns) => {
            let delta = ns.naive() - occ_current.naive();
            let naive = old_start.naive() + delta;
            match ns {
                CalTime::Date(_) => CalTime::Date(naive.date()),
                CalTime::DateTime { zone, .. } => CalTime::DateTime {
                    local: naive,
                    zone: zone.clone(),
                },
            }
        }
        None => old_start.clone(),
    };
    let start_changed = new_start != old_start;
    let new_zone = resolver.zone_of(&new_start);

    // Collect override data before mutating.
    let mut override_updates: Vec<(usize, FieldSnapshot, CalTime, Option<CalTime>)> = Vec::new();
    for i in obj.override_indices() {
        let ov = obj.event_at(i).expect("override");
        if let Ok(Some(rid)) = ov.recurrence_id() {
            override_updates.push((i, FieldSnapshot::of(&ov), rid.value, ov.start().ok()));
        }
    }

    let remove_recurrence = matches!(changes.rrule, Some(None));
    {
        let mut m = obj.master_mut().expect("master");
        apply_fields(&mut m, changes);
        if start_changed {
            m.set_start(&new_start);
        }
        // End.
        match &changes.end {
            Some(new_end_occ) => {
                let occ_start_new = changes.start.clone().unwrap_or_else(|| occ_current.clone());
                let len = new_end_occ.naive() - occ_start_new.naive();
                let master_end = match new_end_occ {
                    CalTime::Date(_) => CalTime::Date((new_start.naive() + len).date()),
                    CalTime::DateTime { zone, .. } => {
                        let z = if new_end_occ.zone() == occ_start_new.zone() {
                            new_start.zone().cloned().unwrap_or_else(|| zone.clone())
                        } else {
                            zone.clone()
                        };
                        CalTime::DateTime {
                            local: new_start.naive() + len,
                            zone: z,
                        }
                    }
                };
                set_end_keeping_style(&mut m, &old_end, &new_start, &master_end);
            }
            None if start_changed => {
                if let Some(e) = shifted_end(&old_start, &old_end, &new_start, resolver) {
                    m.set_end(&e);
                }
            }
            None => {}
        }
        // Rule. Whether `UNTIL` is the series' own and moves with it.
        let mut own_until = true;
        match &changes.rrule {
            Some(Some(rule)) => {
                let current = m.rrule()?;
                let (rule, own) = rule_from_here(rule, seen_rule, current.as_ref());
                own_until = own;
                m.set_rrule(Some(&rule));
            }
            Some(None) => {
                m.set_rrule(None);
                m.component_mut().remove_properties("RDATE");
                m.component_mut().remove_properties("EXDATE");
            }
            None => {}
        }
        if start_changed && !remove_recurrence {
            // UNTIL follows the series (unless the sender chose a new one).
            if let Some(mut rule) = m.rrule()?.filter(|_| own_until) {
                if let Some(until) = rule.until() {
                    let nu = shift_until(&until, &old_start, &new_start, &old_zone, &new_zone);
                    rule.set_until(Some(nu));
                    m.set_rrule(Some(&rule));
                }
            }
            let shift =
                |t: &CalTime| shift_rid(t, &old_start, &new_start, &old_zone, &new_zone, resolver);
            let ex = m.exdates()?;
            if !ex.is_empty() {
                let shifted: Vec<CalTime> = ex.iter().map(shift).collect();
                m.set_exdates(&shifted);
            }
            let rd = m.rdates()?;
            if !rd.is_empty() {
                let shifted: Vec<RDate> = rd
                    .iter()
                    .map(|r| match r {
                        RDate::Time(t) => RDate::Time(shift(t)),
                        RDate::Period(p) => RDate::Period(crate::value::Period {
                            start: shift(&p.start),
                            end: match &p.end {
                                PeriodEnd::End(e) => PeriodEnd::End(shift(e)),
                                PeriodEnd::Duration(d) => PeriodEnd::Duration(*d),
                            },
                        }),
                    })
                    .collect();
                m.set_rdates(&shifted);
            }
        }
        m.bump_sequence();
        m.touch(ctx.now);
    }

    if remove_recurrence {
        let idx = obj.override_indices();
        let comps = obj.calendar_mut().components_mut();
        for i in idx.into_iter().rev() {
            comps.remove(i);
        }
        return Ok(());
    }

    for (i, ov_snapshot, rid, ov_start) in override_updates {
        let field_changes = snapshot.filter_for(&ov_snapshot, changes);
        let mut ov = obj.event_at_mut(i).expect("override");
        apply_fields(&mut ov, &field_changes);
        if start_changed {
            let new_rid = shift_rid(&rid, &old_start, &new_start, &old_zone, &new_zone, resolver);
            // An override that was not moved in time follows the series.
            let unmoved = ov_start
                .as_ref()
                .is_some_and(|s| same_instant(s, &rid, resolver) && s.is_date() == rid.is_date());
            if unmoved {
                let target = to_master_form(&new_rid, &new_start, &new_zone, resolver);
                let changes = EventChanges {
                    start: Some(target),
                    ..Default::default()
                };
                apply_times_single(&mut ov, &changes, resolver)?;
            }
            // Keep the RANGE parameter if present.
            let range = ov
                .component()
                .property("RECURRENCE-ID")
                .and_then(|p| p.param("RANGE").map(str::to_string));
            let mut prop = new_rid.to_property("RECURRENCE-ID");
            if let Some(r) = range {
                prop.set_param("RANGE", r);
            }
            ov.component_mut().set_property(prop);
        }
        if !field_changes.is_noop() || start_changed {
            ov.bump_sequence();
            ov.touch(ctx.now);
        }
    }
    Ok(())
}

impl EventChanges {
    fn is_noop(&self) -> bool {
        *self == EventChanges::default()
    }
}

/// The rule to write for a sent `rule` (see [`EventChanges::rrule`]):
/// `seen` is the series' rule before the edit, `current` the rule of the
/// object being edited (after a split: with the remaining `COUNT`).
/// Returns the rule and whether its `UNTIL` is the series' own (unchanged,
/// so it moves along with the start).
fn rule_from_here(
    rule: &RecurRule,
    seen: Option<&RecurRule>,
    current: Option<&RecurRule>,
) -> (RecurRule, bool) {
    let mut out = rule.clone();
    let Some(seen) = seen else {
        return (out, false);
    };
    if rule.count().is_some() && rule.count() == seen.count() {
        if let Some(remaining) = current.and_then(RecurRule::count) {
            out.set_count(Some(remaining));
        }
    }
    let own_until = rule.until().is_some() && rule.until() == seen.until();
    (out, own_until)
}

/// Moves a recurrence id (or EXDATE/RDATE) along with a master whose start
/// changed from `old` to `new`. The value keeps its style (UTC stays UTC)
/// unless the kind (date/date-time) changes.
fn shift_rid(
    rid: &CalTime,
    old: &CalTime,
    new: &CalTime,
    old_zone: &Zone,
    new_zone: &Zone,
    resolver: &ZoneResolver,
) -> CalTime {
    let wall = if rid.is_date() {
        rid.date().and_time(old.naive().time())
    } else {
        old_zone.to_local(resolver.to_utc(rid))
    };
    let delta = new.naive() - old.naive();
    let new_wall = wall + delta;
    match new {
        CalTime::Date(_) => CalTime::Date(new_wall.date()),
        CalTime::DateTime { zone, .. } => {
            if rid.zone() == Some(&TimeZoneRef::Utc) {
                CalTime::utc(new_zone.to_utc(new_wall).naive_utc())
            } else {
                CalTime::DateTime {
                    local: new_wall,
                    zone: zone.clone(),
                }
            }
        }
    }
}

/// Moves `UNTIL` along with the series start.
fn shift_until(
    until: &CalTime,
    old: &CalTime,
    new: &CalTime,
    old_zone: &Zone,
    new_zone: &Zone,
) -> CalTime {
    let delta = new.naive() - old.naive();
    let wall = match until {
        CalTime::Date(d) => d.and_time(old.naive().time()),
        CalTime::DateTime {
            local,
            zone: TimeZoneRef::Utc,
        } => old_zone.to_local(chrono::TimeZone::from_utc_datetime(&Utc, local)),
        CalTime::DateTime { local, .. } => *local,
    } + delta;
    match new {
        CalTime::Date(_) => CalTime::Date(wall.date()),
        CalTime::DateTime {
            zone: TimeZoneRef::Floating,
            ..
        } if !matches!(
            until,
            CalTime::DateTime {
                zone: TimeZoneRef::Utc,
                ..
            }
        ) =>
        {
            CalTime::floating(wall)
        }
        CalTime::DateTime { .. } => {
            let wall = if until.is_date() {
                // A DATE until covers the whole day.
                wall.date().and_hms_opt(23, 59, 59).unwrap_or(wall)
            } else {
                wall
            };
            CalTime::utc(new_zone.to_utc(wall).naive_utc())
        }
    }
}

// ---------------------------------------------------------------------------
// "This and following" split

/// Ends the series in `obj` before `split`; with `new_uid`, returns a new
/// object holding the remainder (new master starting at the split, later
/// overrides/EXDATEs/RDATEs moved over).
fn split_series(
    obj: &mut CalendarObject,
    split: &Located,
    new_uid: Option<&str>,
    ctx: &EditContext,
    resolver: &ZoneResolver,
) -> Result<Option<CalendarObject>> {
    let master = obj.master().expect("master");
    let (mstart, zone) = master_zone(&master, resolver)?;
    let rule = master.rrule()?;
    let master_comp = master.component().clone();
    let is_after = |t: &CalTime| -> bool {
        if t.is_date() || mstart.is_date() {
            to_master_form(t, &mstart, &zone, resolver).date() >= split.rid.date()
        } else {
            resolver.to_utc(t) >= split.start
        }
    };

    // Overrides at or after the split move to the new series.
    let mut moved: Vec<Component> = Vec::new();
    let mut remove = Vec::new();
    for i in obj.override_indices() {
        let ov = obj.event_at(i).expect("override");
        if let Ok(Some(rid)) = ov.recurrence_id() {
            if is_after(&rid.value) {
                moved.push(ov.component().clone());
                remove.push(i);
            }
        }
    }
    {
        let comps = obj.calendar_mut().components_mut();
        for i in remove.into_iter().rev() {
            comps.remove(i);
        }
    }

    // Old series: end before the split.
    let mut new_rule = rule.clone();
    {
        let mut m = obj.master_mut().expect("master");
        if let Some(mut r) = rule.clone() {
            match r.count() {
                Some(total) => {
                    let before = split.rule_instances_before;
                    r.set_count(Some(before.max(1)));
                    if let Some(nr) = new_rule.as_mut() {
                        nr.set_count(Some(total.saturating_sub(before).max(1)));
                    }
                }
                None => {
                    let until = if mstart.is_date() {
                        CalTime::Date(split.rid.date().pred_opt().unwrap_or(split.rid.date()))
                    } else if mstart.zone() == Some(&TimeZoneRef::Floating) {
                        CalTime::floating(split.rid.naive() - Duration::seconds(1))
                    } else {
                        CalTime::utc((split.start - Duration::seconds(1)).naive_utc())
                    };
                    r.set_until(Some(until));
                }
            }
            m.set_rrule(Some(&r));
        }
        m.retain_exdates(|t| !is_after(t))?;
        m.retain_rdates(|r| !is_after(r.start()))?;
        m.bump_sequence();
        m.touch(ctx.now);
    }

    let Some(uid) = new_uid else {
        return Ok(None);
    };

    // New series.
    let mut new_master = Event::new(master_comp);
    new_master.set_uid(uid);
    let old_end = new_master.end()?;
    new_master.set_start(&split.rid);
    if let Some(e) = shifted_end(&mstart, &old_end, &split.rid, resolver) {
        new_master.set_end(&e);
    }
    new_master.set_rrule(new_rule.as_ref());
    new_master.retain_exdates(&is_after)?;
    new_master.retain_rdates(|r| is_after(r.start()))?;
    new_master.set_sequence(0);
    new_master.touch(ctx.now);

    let mut cal = Component::new(obj.calendar().name());
    *cal.properties_mut() = obj.calendar().properties().to_vec();
    for c in obj.calendar().components() {
        if !c.is("VEVENT") {
            cal.components_mut().push(c.clone());
        }
    }
    cal.components_mut().push(new_master.into_inner());
    for mut ov in moved {
        let mut e = Event::new(&mut ov);
        e.set_uid(uid);
        e.touch(ctx.now);
        cal.components_mut().push(ov);
    }
    Ok(Some(CalendarObject::from_component(cal)?))
}
