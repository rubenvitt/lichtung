//! Typed view on a `VEVENT` component.
//!
//! [`Event`] wraps anything that borrows a [`Component`]: `Event<&Component>`
//! for reading, `Event<&mut Component>` for in-place editing and
//! `Event<Component>` for an owned event. Setters touch only the properties
//! they are about; everything else (including unknown properties and their
//! original line folding) stays as it was.

use chrono::{DateTime, Utc};
use std::borrow::{Borrow, BorrowMut};

use crate::error::{Error, Result};
use crate::parser::{Component, Property};
use crate::rule::RecurRule;
use crate::value::{CalTime, IcalDuration, RDate};

/// `STATUS` of an event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventStatus {
    /// `TENTATIVE`
    Tentative,
    /// `CONFIRMED`
    Confirmed,
    /// `CANCELLED`
    Cancelled,
    /// Anything else (kept verbatim).
    Other(String),
}

impl EventStatus {
    /// Parses a `STATUS` value.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_uppercase().as_str() {
            "TENTATIVE" => Self::Tentative,
            "CONFIRMED" => Self::Confirmed,
            "CANCELLED" => Self::Cancelled,
            _ => Self::Other(s.trim().to_string()),
        }
    }

    /// The iCalendar keyword.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Tentative => "TENTATIVE",
            Self::Confirmed => "CONFIRMED",
            Self::Cancelled => "CANCELLED",
            Self::Other(s) => s,
        }
    }
}

/// `TRANSP` of an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transparency {
    /// `OPAQUE`: blocks time (default).
    Opaque,
    /// `TRANSPARENT`: shown as free.
    Transparent,
}

/// Participation status (`PARTSTAT`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PartStat {
    /// `NEEDS-ACTION`
    NeedsAction,
    /// `ACCEPTED`
    Accepted,
    /// `DECLINED`
    Declined,
    /// `TENTATIVE`
    Tentative,
    /// `DELEGATED`
    Delegated,
    /// Anything else (kept verbatim).
    Other(String),
}

impl PartStat {
    /// Parses a `PARTSTAT` value.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_uppercase().as_str() {
            "NEEDS-ACTION" => Self::NeedsAction,
            "ACCEPTED" => Self::Accepted,
            "DECLINED" => Self::Declined,
            "TENTATIVE" => Self::Tentative,
            "DELEGATED" => Self::Delegated,
            _ => Self::Other(s.trim().to_string()),
        }
    }

    /// The iCalendar keyword.
    pub fn as_str(&self) -> &str {
        match self {
            Self::NeedsAction => "NEEDS-ACTION",
            Self::Accepted => "ACCEPTED",
            Self::Declined => "DECLINED",
            Self::Tentative => "TENTATIVE",
            Self::Delegated => "DELEGATED",
            Self::Other(s) => s,
        }
    }
}

/// `ORGANIZER`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Organizer {
    /// Calendar user address, usually `mailto:…`.
    pub address: String,
    /// Display name (`CN`).
    pub cn: Option<String>,
}

/// `ATTENDEE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attendee {
    /// Calendar user address, usually `mailto:…`.
    pub address: String,
    /// Display name (`CN`).
    pub cn: Option<String>,
    /// `PARTSTAT` (absent means `NEEDS-ACTION` per RFC 5545).
    pub partstat: Option<PartStat>,
    /// `ROLE`, e.g. `REQ-PARTICIPANT`.
    pub role: Option<String>,
    /// `RSVP`.
    pub rsvp: Option<bool>,
}

impl Attendee {
    /// The e-mail address without `mailto:`.
    pub fn email(&self) -> &str {
        strip_mailto(&self.address)
    }
}

fn strip_mailto(s: &str) -> &str {
    if s.len() >= 7 && s[..7].eq_ignore_ascii_case("mailto:") {
        &s[7..]
    } else {
        s
    }
}

/// `TRIGGER` of a `VALARM`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlarmTrigger {
    /// Relative to the start (or end with `RELATED=END`) of the event.
    Relative {
        /// Offset; negative means before.
        offset: IcalDuration,
        /// `RELATED=END`.
        related_to_end: bool,
    },
    /// Absolute instant (`VALUE=DATE-TIME`).
    Absolute(CalTime),
}

/// A `VALARM` sub-component (only the fields Lichtung uses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    /// `ACTION`, e.g. `DISPLAY`.
    pub action: Option<String>,
    /// `TRIGGER`.
    pub trigger: Option<AlarmTrigger>,
    /// `DESCRIPTION`.
    pub description: Option<String>,
}

/// `RECURRENCE-ID` of an override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurrenceId {
    /// Original start of the overridden occurrence.
    pub value: CalTime,
    /// `RANGE=THISANDFUTURE`.
    pub this_and_future: bool,
}

/// How the end of an event is specified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventEnd {
    /// `DTEND`.
    End(CalTime),
    /// `DURATION`.
    Duration(IcalDuration),
    /// Neither: zero length for date-times, one day for dates (RFC 5545).
    Implicit,
}

/// Typed view on a `VEVENT`. See the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event<C = Component> {
    inner: C,
}

impl<C: Borrow<Component>> Event<C> {
    /// Wraps a component. Does not check the component name.
    pub fn new(inner: C) -> Self {
        Self { inner }
    }

    /// The underlying component.
    pub fn component(&self) -> &Component {
        self.inner.borrow()
    }

    /// Unwraps the component.
    pub fn into_inner(self) -> C {
        self.inner
    }

    fn text(&self, name: &str) -> Option<String> {
        self.component().property(name).map(Property::text)
    }

    /// `UID`.
    pub fn uid(&self) -> Option<String> {
        self.component()
            .property("UID")
            .map(|p| p.value().trim().to_string())
    }

    /// `DTSTART` (required).
    pub fn start(&self) -> Result<CalTime> {
        let p = self
            .component()
            .property("DTSTART")
            .ok_or_else(|| Error::MissingProperty("DTSTART".into()))?;
        CalTime::from_property(p)
    }

    /// `DTEND`, `DURATION` or neither.
    pub fn end(&self) -> Result<EventEnd> {
        if let Some(p) = self.component().property("DTEND") {
            return Ok(EventEnd::End(CalTime::from_property(p)?));
        }
        if let Some(p) = self.component().property("DURATION") {
            return Ok(EventEnd::Duration(IcalDuration::parse(p.value())?));
        }
        Ok(EventEnd::Implicit)
    }

    /// Whether the event is all-day (`DTSTART` is a DATE).
    pub fn is_all_day(&self) -> bool {
        self.start().map(|s| s.is_date()).unwrap_or(false)
    }

    /// `SUMMARY` (unescaped).
    pub fn summary(&self) -> Option<String> {
        self.text("SUMMARY")
    }

    /// `LOCATION` (unescaped).
    pub fn location(&self) -> Option<String> {
        self.text("LOCATION")
    }

    /// `DESCRIPTION` (unescaped).
    pub fn description(&self) -> Option<String> {
        self.text("DESCRIPTION")
    }

    /// `URL`.
    pub fn url(&self) -> Option<String> {
        self.component()
            .property("URL")
            .map(|p| p.value().to_string())
    }

    /// `STATUS`.
    pub fn status(&self) -> Option<EventStatus> {
        self.component()
            .property("STATUS")
            .map(|p| EventStatus::parse(p.value()))
    }

    /// Whether `STATUS:CANCELLED`.
    pub fn is_cancelled(&self) -> bool {
        self.status() == Some(EventStatus::Cancelled)
    }

    /// `TRANSP` (default [`Transparency::Opaque`]).
    pub fn transparency(&self) -> Transparency {
        match self.component().property("TRANSP") {
            Some(p) if p.value().trim().eq_ignore_ascii_case("TRANSPARENT") => {
                Transparency::Transparent
            }
            _ => Transparency::Opaque,
        }
    }

    /// `SEQUENCE` (default 0).
    pub fn sequence(&self) -> u32 {
        self.component()
            .property("SEQUENCE")
            .and_then(|p| p.value().trim().parse().ok())
            .unwrap_or(0)
    }

    /// `RRULE` (the first one; multiple RRULEs are deprecated).
    pub fn rrule(&self) -> Result<Option<RecurRule>> {
        self.component()
            .property("RRULE")
            .map(|p| RecurRule::parse(p.value()))
            .transpose()
    }

    /// All `RDATE` entries.
    pub fn rdates(&self) -> Result<Vec<RDate>> {
        let mut out = Vec::new();
        for p in self.component().properties_named("RDATE") {
            out.extend(RDate::list_from_property(p)?);
        }
        Ok(out)
    }

    /// All `EXDATE` entries.
    pub fn exdates(&self) -> Result<Vec<CalTime>> {
        let mut out = Vec::new();
        for p in self.component().properties_named("EXDATE") {
            out.extend(CalTime::list_from_property(p)?);
        }
        Ok(out)
    }

    /// Whether this event is a series master (has `RRULE` or `RDATE`).
    pub fn is_recurring(&self) -> bool {
        self.component().property("RRULE").is_some() || self.component().property("RDATE").is_some()
    }

    /// `RECURRENCE-ID` (present on overrides).
    pub fn recurrence_id(&self) -> Result<Option<RecurrenceId>> {
        let Some(p) = self.component().property("RECURRENCE-ID") else {
            return Ok(None);
        };
        Ok(Some(RecurrenceId {
            value: CalTime::from_property(p)?,
            this_and_future: p
                .param("RANGE")
                .is_some_and(|r| r.eq_ignore_ascii_case("THISANDFUTURE")),
        }))
    }

    /// `ORGANIZER`.
    pub fn organizer(&self) -> Option<Organizer> {
        self.component().property("ORGANIZER").map(|p| Organizer {
            address: p.value().trim().to_string(),
            cn: p.param("CN").map(str::to_string),
        })
    }

    /// All `ATTENDEE`s.
    pub fn attendees(&self) -> Vec<Attendee> {
        self.component()
            .properties_named("ATTENDEE")
            .map(|p| Attendee {
                address: p.value().trim().to_string(),
                cn: p.param("CN").map(str::to_string),
                partstat: p.param("PARTSTAT").map(PartStat::parse),
                role: p.param("ROLE").map(str::to_string),
                rsvp: p.param("RSVP").map(|v| v.eq_ignore_ascii_case("TRUE")),
            })
            .collect()
    }

    /// All `VALARM`s.
    pub fn alarms(&self) -> Vec<Alarm> {
        self.component()
            .components_named("VALARM")
            .map(|a| Alarm {
                action: a.property("ACTION").map(|p| p.value().trim().to_string()),
                trigger: a.property("TRIGGER").and_then(parse_trigger),
                description: a.property("DESCRIPTION").map(Property::text),
            })
            .collect()
    }

    /// `X-APPLE-TRAVEL-DURATION` (travel time before the event).
    pub fn travel_duration(&self) -> Option<IcalDuration> {
        self.component()
            .property("X-APPLE-TRAVEL-DURATION")
            .and_then(|p| IcalDuration::parse(p.value()).ok())
    }
}

fn parse_trigger(p: &Property) -> Option<AlarmTrigger> {
    let is_abs = p
        .param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"));
    if is_abs {
        return CalTime::from_property(p).ok().map(AlarmTrigger::Absolute);
    }
    match IcalDuration::parse(p.value()) {
        Ok(offset) => Some(AlarmTrigger::Relative {
            offset,
            related_to_end: p
                .param("RELATED")
                .is_some_and(|r| r.eq_ignore_ascii_case("END")),
        }),
        Err(_) => CalTime::from_property(p).ok().map(AlarmTrigger::Absolute),
    }
}

impl<C: BorrowMut<Component>> Event<C> {
    /// Mutable access to the underlying component.
    pub fn component_mut(&mut self) -> &mut Component {
        self.inner.borrow_mut()
    }

    fn set_text(&mut self, name: &str, value: Option<&str>) {
        let c = self.component_mut();
        match value {
            Some(v) => match c.property_mut(name) {
                Some(p) => {
                    if p.text() != v {
                        p.set_text(v);
                    }
                }
                None => c.insert_property(Property::new_text(name, v)),
            },
            None => {
                c.remove_properties(name);
            }
        }
    }

    fn set_raw(&mut self, name: &str, value: Option<&str>) {
        let c = self.component_mut();
        match value {
            Some(v) => match c.property_mut(name) {
                Some(p) => p.set_value(v),
                None => c.insert_property(Property::new(name, v)),
            },
            None => {
                c.remove_properties(name);
            }
        }
    }

    /// Sets `UID`.
    pub fn set_uid(&mut self, uid: &str) {
        self.set_raw("UID", Some(uid));
    }

    /// Sets `DTSTART`.
    pub fn set_start(&mut self, start: &CalTime) {
        self.component_mut()
            .set_property(start.to_property("DTSTART"));
    }

    /// Sets the end: `DTEND` or `DURATION` (removing the other one).
    pub fn set_end(&mut self, end: &EventEnd) {
        let c = self.component_mut();
        match end {
            EventEnd::End(t) => {
                c.remove_properties("DURATION");
                c.set_property(t.to_property("DTEND"));
            }
            EventEnd::Duration(d) => {
                c.remove_properties("DTEND");
                c.set_property(Property::new("DURATION", d.to_string()));
            }
            EventEnd::Implicit => {
                c.remove_properties("DTEND");
                c.remove_properties("DURATION");
            }
        }
    }

    /// Sets or removes `SUMMARY`.
    pub fn set_summary(&mut self, value: Option<&str>) {
        self.set_text("SUMMARY", value);
    }

    /// Sets or removes `LOCATION`.
    pub fn set_location(&mut self, value: Option<&str>) {
        self.set_text("LOCATION", value);
    }

    /// Sets or removes `DESCRIPTION`.
    pub fn set_description(&mut self, value: Option<&str>) {
        self.set_text("DESCRIPTION", value);
    }

    /// Sets or removes `URL`.
    pub fn set_url(&mut self, value: Option<&str>) {
        self.set_raw("URL", value);
    }

    /// Sets or removes `STATUS`.
    pub fn set_status(&mut self, value: Option<&EventStatus>) {
        self.set_raw("STATUS", value.map(EventStatus::as_str));
    }

    /// Sets `TRANSP`.
    pub fn set_transparency(&mut self, value: Transparency) {
        let v = match value {
            Transparency::Opaque => "OPAQUE",
            Transparency::Transparent => "TRANSPARENT",
        };
        self.set_raw("TRANSP", Some(v));
    }

    /// Sets `SEQUENCE`.
    pub fn set_sequence(&mut self, seq: u32) {
        self.set_raw("SEQUENCE", Some(&seq.to_string()));
    }

    /// Increments `SEQUENCE` by one.
    pub fn bump_sequence(&mut self) {
        let s = self.sequence();
        self.set_sequence(s + 1);
    }

    /// Sets `DTSTAMP` and, if present, `LAST-MODIFIED` to `now`.
    pub fn touch(&mut self, now: DateTime<Utc>) {
        let v = CalTime::utc(now.naive_utc().with_nanosecond_zero()).format_value();
        self.set_raw("DTSTAMP", Some(&v));
        if self.component().property("LAST-MODIFIED").is_some() {
            self.set_raw("LAST-MODIFIED", Some(&v));
        }
    }

    /// Sets or removes `RRULE`.
    pub fn set_rrule(&mut self, rule: Option<&RecurRule>) {
        self.set_raw("RRULE", rule.map(|r| r.to_string()).as_deref());
    }

    /// Replaces all `EXDATE`s (grouped into one property per kind/zone).
    pub fn set_exdates(&mut self, values: &[CalTime]) {
        let c = self.component_mut();
        c.remove_properties("EXDATE");
        for group in group_by_kind(values, |t| t) {
            c.insert_property(CalTime::list_to_property("EXDATE", &group));
        }
    }

    /// Adds one `EXDATE` (as a new property; existing ones stay untouched).
    pub fn add_exdate(&mut self, value: &CalTime) {
        self.component_mut()
            .insert_property(value.to_property("EXDATE"));
    }

    /// Keeps only `EXDATE` values matching `keep`; untouched properties keep
    /// their original text.
    pub fn retain_exdates(&mut self, mut keep: impl FnMut(&CalTime) -> bool) -> Result<()> {
        let c = self.component_mut();
        let mut out = Vec::new();
        for p in c.properties().iter() {
            if !p.is("EXDATE") {
                out.push(p.clone());
                continue;
            }
            let values = CalTime::list_from_property(p)?;
            let kept: Vec<CalTime> = values.iter().filter(|v| keep(v)).cloned().collect();
            if kept.len() == values.len() {
                out.push(p.clone());
            } else if !kept.is_empty() {
                let mut np = p.clone();
                np.set_value(
                    kept.iter()
                        .map(CalTime::format_value)
                        .collect::<Vec<_>>()
                        .join(","),
                );
                out.push(np);
            }
        }
        *c.properties_mut() = out;
        Ok(())
    }

    /// Replaces all `RDATE`s.
    pub fn set_rdates(&mut self, values: &[RDate]) {
        let c = self.component_mut();
        c.remove_properties("RDATE");
        for group in group_by_kind(values, |r| r.start()) {
            c.insert_property(RDate::list_to_property(&group));
        }
    }

    /// Keeps only `RDATE` entries matching `keep`.
    pub fn retain_rdates(&mut self, mut keep: impl FnMut(&RDate) -> bool) -> Result<()> {
        let c = self.component_mut();
        let mut out = Vec::new();
        for p in c.properties().iter() {
            if !p.is("RDATE") {
                out.push(p.clone());
                continue;
            }
            let values = RDate::list_from_property(p)?;
            let kept: Vec<RDate> = values.iter().filter(|v| keep(v)).cloned().collect();
            if kept.len() == values.len() {
                out.push(p.clone());
            } else if !kept.is_empty() {
                let mut np = p.clone();
                np.set_value(
                    kept.iter()
                        .map(RDate::format_value)
                        .collect::<Vec<_>>()
                        .join(","),
                );
                out.push(np);
            }
        }
        *c.properties_mut() = out;
        Ok(())
    }

    /// Sets or removes `RECURRENCE-ID` (without `RANGE`).
    pub fn set_recurrence_id(&mut self, value: Option<&CalTime>) {
        let c = self.component_mut();
        match value {
            Some(v) => c.set_property(v.to_property("RECURRENCE-ID")),
            None => {
                c.remove_properties("RECURRENCE-ID");
            }
        }
    }

    /// Sets the `PARTSTAT` of the attendee with the given address (with or
    /// without `mailto:`, case-insensitive). Clears `RSVP=TRUE`. Returns
    /// whether the attendee was found.
    pub fn set_partstat(&mut self, address: &str, partstat: &PartStat) -> bool {
        let wanted = strip_mailto(address.trim()).to_ascii_lowercase();
        let mut found = false;
        for p in self.component_mut().properties_mut().iter_mut() {
            if p.is("ATTENDEE") && strip_mailto(p.value().trim()).to_ascii_lowercase() == wanted {
                p.set_param("PARTSTAT", partstat.as_str());
                if p.param("RSVP").is_some() {
                    p.remove_param("RSVP");
                }
                found = true;
            }
        }
        found
    }

    /// Adds an attendee.
    pub fn add_attendee(&mut self, attendee: &Attendee) {
        let address = if attendee.address.contains(':') {
            attendee.address.clone()
        } else {
            format!("mailto:{}", attendee.address)
        };
        let mut p = Property::new("ATTENDEE", address);
        if let Some(cn) = &attendee.cn {
            p.set_param("CN", cn.clone());
        }
        if let Some(role) = &attendee.role {
            p.set_param("ROLE", role.clone());
        }
        if let Some(ps) = &attendee.partstat {
            p.set_param("PARTSTAT", ps.as_str());
        }
        if let Some(rsvp) = attendee.rsvp {
            p.set_param("RSVP", if rsvp { "TRUE" } else { "FALSE" });
        }
        self.component_mut().insert_property(p);
    }

    /// Removes the attendee with the given address; returns whether it existed.
    pub fn remove_attendee(&mut self, address: &str) -> bool {
        let wanted = strip_mailto(address.trim()).to_ascii_lowercase();
        let props = self.component_mut().properties_mut();
        let before = props.len();
        props.retain(|p| {
            !(p.is("ATTENDEE") && strip_mailto(p.value().trim()).to_ascii_lowercase() == wanted)
        });
        props.len() != before
    }

    /// Adds a `DISPLAY` alarm relative to the start (negative = before).
    pub fn add_relative_alarm(&mut self, offset: IcalDuration, description: &str) {
        let mut a = Component::new("VALARM");
        a.insert_property(Property::new("ACTION", "DISPLAY"));
        a.insert_property(Property::new("TRIGGER", offset.to_string()));
        a.insert_property(Property::new_text("DESCRIPTION", description));
        self.component_mut().components_mut().push(a);
    }

    /// Removes all `VALARM`s.
    pub fn remove_alarms(&mut self) {
        self.component_mut()
            .components_mut()
            .retain(|c| !c.is("VALARM"));
    }

    /// Sets or removes `X-APPLE-TRAVEL-DURATION`.
    pub fn set_travel_duration(&mut self, value: Option<IcalDuration>) {
        let c = self.component_mut();
        match value {
            Some(d) => c.set_property(
                Property::new("X-APPLE-TRAVEL-DURATION", d.to_string())
                    .with_param("VALUE", "DURATION"),
            ),
            None => {
                c.remove_properties("X-APPLE-TRAVEL-DURATION");
                c.remove_properties("X-APPLE-TRAVEL-START");
            }
        }
    }
}

trait NoNanos {
    fn with_nanosecond_zero(self) -> Self;
}

impl NoNanos for chrono::NaiveDateTime {
    fn with_nanosecond_zero(self) -> Self {
        use chrono::Timelike;
        self.with_nanosecond(0).unwrap_or(self)
    }
}

/// Groups values into runs that can share one property (same kind and zone),
/// keeping their order.
fn group_by_kind<T: Clone>(values: &[T], key: impl Fn(&T) -> &CalTime) -> Vec<Vec<T>> {
    let mut groups: Vec<Vec<T>> = Vec::new();
    for v in values {
        let k = key(v);
        let same = |other: &T| {
            let o = key(other);
            o.is_date() == k.is_date() && o.zone() == k.zone()
        };
        match groups.iter_mut().find(|g| same(&g[0])) {
            Some(g) => g.push(v.clone()),
            None => groups.push(vec![v.clone()]),
        }
    }
    groups
}
