//! A calendar object resource: one `VCALENDAR` holding a series master
//! and/or overrides that share one `UID` (the unit CalDAV stores per URL).

use chrono_tz::Tz;
use std::collections::HashSet;

use crate::error::{Error, Result};
use crate::event::Event;
use crate::parser::{self, Component};
use crate::tz::ZoneResolver;
use crate::value::CalTime;

/// One `VCALENDAR` with the events of a single `UID` (master + overrides).
///
/// The whole calendar is kept losslessly; [`CalendarObject::to_ics`]
/// reproduces it byte for byte if nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarObject {
    calendar: Component,
}

impl CalendarObject {
    /// Parses iCalendar text containing exactly one `VCALENDAR`.
    pub fn parse(ics: &str) -> Result<Self> {
        let mut top = parser::parse(ics)?;
        if top.len() != 1 {
            return Err(Error::InvalidObject(format!(
                "genau ein VCALENDAR erwartet, gefunden: {}",
                top.len()
            )));
        }
        Self::from_component(top.remove(0))
    }

    /// Wraps a parsed `VCALENDAR`.
    pub fn from_component(calendar: Component) -> Result<Self> {
        if !calendar.is("VCALENDAR") {
            return Err(Error::InvalidObject(format!(
                "VCALENDAR erwartet, gefunden: {}",
                calendar.name()
            )));
        }
        Ok(Self { calendar })
    }

    /// Serializes to iCalendar text (CRLF, folded).
    pub fn to_ics(&self) -> String {
        self.calendar.to_ics()
    }

    /// The `VCALENDAR` component.
    pub fn calendar(&self) -> &Component {
        &self.calendar
    }

    /// Mutable access to the `VCALENDAR` component.
    pub fn calendar_mut(&mut self) -> &mut Component {
        &mut self.calendar
    }

    /// Consumes the object and returns the `VCALENDAR`.
    pub fn into_component(self) -> Component {
        self.calendar
    }

    /// `UID` of the first event.
    pub fn uid(&self) -> Option<String> {
        self.events().find_map(|e| e.uid())
    }

    /// All `VEVENT`s in source order.
    pub fn events(&self) -> impl Iterator<Item = Event<&Component>> {
        self.calendar.components_named("VEVENT").map(Event::new)
    }

    /// Index (into the `VCALENDAR`'s sub-components) of the master, i.e. the
    /// `VEVENT` without `RECURRENCE-ID`.
    pub fn master_index(&self) -> Option<usize> {
        self.calendar
            .components()
            .iter()
            .position(|c| c.is("VEVENT") && c.property("RECURRENCE-ID").is_none())
    }

    /// The master event (absent if the object only holds overrides, e.g. an
    /// invitation to a single occurrence).
    pub fn master(&self) -> Option<Event<&Component>> {
        self.master_index()
            .map(|i| Event::new(&self.calendar.components()[i]))
    }

    /// Mutable master event.
    pub fn master_mut(&mut self) -> Option<Event<&mut Component>> {
        let i = self.master_index()?;
        Some(Event::new(&mut self.calendar.components_mut()[i]))
    }

    /// Indices of all overrides (`VEVENT`s with `RECURRENCE-ID`).
    pub fn override_indices(&self) -> Vec<usize> {
        self.calendar
            .components()
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is("VEVENT") && c.property("RECURRENCE-ID").is_some())
            .map(|(i, _)| i)
            .collect()
    }

    /// All overrides.
    pub fn overrides(&self) -> Vec<Event<&Component>> {
        self.override_indices()
            .into_iter()
            .map(|i| Event::new(&self.calendar.components()[i]))
            .collect()
    }

    /// The event at a sub-component index (see [`crate::Occurrence::component_index`]).
    pub fn event_at(&self, index: usize) -> Option<Event<&Component>> {
        self.calendar
            .components()
            .get(index)
            .filter(|c| c.is("VEVENT"))
            .map(Event::new)
    }

    /// Mutable event at a sub-component index.
    pub fn event_at_mut(&mut self, index: usize) -> Option<Event<&mut Component>> {
        self.calendar
            .components_mut()
            .get_mut(index)
            .filter(|c| c.is("VEVENT"))
            .map(Event::new)
    }

    /// A resolver knowing this calendar's `VTIMEZONE`s.
    pub fn resolver(&self, default: Tz) -> ZoneResolver {
        ZoneResolver::for_calendar(&self.calendar, default)
    }

    /// Index of the override whose `RECURRENCE-ID` denotes the same
    /// occurrence as `rid`.
    pub fn find_override(&self, rid: &CalTime, resolver: &ZoneResolver) -> Option<usize> {
        self.override_indices().into_iter().find(|&i| {
            Event::new(&self.calendar.components()[i])
                .recurrence_id()
                .ok()
                .flatten()
                .is_some_and(|r| same_instant(&r.value, rid, resolver))
        })
    }
}

/// Whether two DATE/DATE-TIME values denote the same occurrence: equal
/// instants for date-times, equal dates for dates; a date and a date-time
/// match when the date-time falls on that date (in its own zone).
pub fn same_instant(a: &CalTime, b: &CalTime, resolver: &ZoneResolver) -> bool {
    match (a.is_date(), b.is_date()) {
        (true, true) => a.date() == b.date(),
        (false, false) => resolver.to_utc(a) == resolver.to_utc(b),
        _ => a.date() == b.date(),
    }
}

/// Splits a calendar with many events (e.g. an ICS feed) into one
/// [`CalendarObject`] per `UID`.
///
/// Each object gets the calendar-level properties, the `VTIMEZONE`s its
/// events reference and its `VEVENT`s. Other components (`VTODO`, …) are
/// dropped. Events without `UID` become objects of their own.
pub fn split_by_uid(ics: &str) -> Result<Vec<CalendarObject>> {
    let mut out: Vec<(Option<String>, Component)> = Vec::new();
    for cal in parser::parse(ics)? {
        if !cal.is("VCALENDAR") {
            continue;
        }
        let timezones: Vec<&Component> = cal.components_named("VTIMEZONE").collect();
        let shell = |events: &[&Component]| {
            let mut c = Component::new(cal.name());
            *c.properties_mut() = cal.properties().to_vec();
            let used: HashSet<String> = events
                .iter()
                .flat_map(|e| e.properties().iter())
                .filter_map(|p| p.param("TZID").map(str::to_string))
                .collect();
            for tz in &timezones {
                if tz
                    .property("TZID")
                    .is_some_and(|p| used.contains(p.value()))
                {
                    c.components_mut().push((*tz).clone());
                }
            }
            c
        };
        let mut order: Vec<Option<String>> = Vec::new();
        let mut groups: Vec<Vec<&Component>> = Vec::new();
        for ev in cal.components_named("VEVENT") {
            let uid = Event::new(ev).uid();
            match uid
                .as_ref()
                .and_then(|u| order.iter().position(|o| o.as_ref() == Some(u)))
            {
                Some(i) => groups[i].push(ev),
                None => {
                    order.push(uid);
                    groups.push(vec![ev]);
                }
            }
        }
        for (uid, events) in order.into_iter().zip(groups) {
            let mut c = shell(&events);
            c.components_mut().extend(events.into_iter().cloned());
            out.push((uid, c));
        }
    }
    out.into_iter()
        .map(|(_, c)| CalendarObject::from_component(c))
        .collect()
}
