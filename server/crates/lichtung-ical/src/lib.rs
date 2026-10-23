//! # lichtung-ical
//!
//! iCalendar handling for the Lichtung server:
//!
//! * a **lossless model** of RFC 5545 data ([`parse`], [`Component`],
//!   [`Property`]): unknown properties/components, order and the original line
//!   folding survive a parse → serialize roundtrip,
//! * a **typed view** on `VEVENT`s ([`Event`]) with setters that only touch
//!   the affected properties,
//! * **time zones** ([`ZoneResolver`]): IANA, Windows/Outlook names, path
//!   prefixes, custom `VTIMEZONE`s, floating time,
//! * **expansion** of series into occurrences ([`expand`]): `RRULE`, `RDATE`,
//!   `EXDATE`, overrides, DST-correct wall-clock semantics,
//! * **scoped editing** ([`edit_event`], [`delete_event`]): only this, this
//!   and following, all.
//!
//! ```
//! use chrono::{TimeZone, Utc};
//! use lichtung_ical::{expand, CalendarObject, ExpandOptions};
//!
//! let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//DE\r\n\
//! BEGIN:VEVENT\r\nUID:team-sync@example.org\r\nDTSTAMP:20261001T080000Z\r\n\
//! DTSTART;TZID=Europe/Berlin:20261019T090000\r\nDTEND;TZID=Europe/Berlin:20261019T093000\r\n\
//! RRULE:FREQ=WEEKLY;COUNT=3\r\nSUMMARY:Team-Sync\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
//! let object = CalendarObject::parse(ics).unwrap();
//! assert_eq!(object.to_ics(), ics);
//!
//! let from = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
//! let to = Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap();
//! let result = expand(&object, from, to, &ExpandOptions::new(chrono_tz::Europe::Berlin)).unwrap();
//! // 09:00 local stays 09:00 across the switch to winter time on 25 October.
//! let starts: Vec<String> = result.occurrences.iter().map(|o| o.start.to_rfc3339()).collect();
//! assert_eq!(starts, [
//!     "2026-10-19T07:00:00+00:00",
//!     "2026-10-26T08:00:00+00:00",
//!     "2026-11-02T08:00:00+00:00",
//! ]);
//! ```

#![warn(missing_docs)]

mod edit;
mod error;
mod event;
mod expand;
mod object;
mod parser;
mod rule;
mod tz;
mod value;
mod windows_zones;

pub use edit::{delete_event, edit_event, EditContext, EditOutcome, EditScope, EventChanges};
pub use error::{Error, Result};
pub use event::{
    Alarm, AlarmTrigger, Attendee, Event, EventEnd, EventStatus, Organizer, PartStat, RecurrenceId,
    Transparency,
};
pub use expand::{
    expand, parse_recurrence_key, recurrence_key, AllDay, ExpandOptions, Expansion, Occurrence,
    DEFAULT_MAX_OCCURRENCES,
};
pub use object::{same_instant, split_by_uid, CalendarObject};
pub use parser::{escape_text, parse, serialize, unescape_text, Component, Param, Property};
pub use rule::{Frequency, RecurRule, WallClockIter};
pub use tz::{resolve_known_tzid, CustomZone, ResolvedBy, Zone, ZoneResolver};
pub use value::{CalTime, IcalDuration, Period, PeriodEnd, RDate, TimeZoneRef};
