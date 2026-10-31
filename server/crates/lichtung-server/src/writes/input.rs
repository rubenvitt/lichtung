//! Inputs of the write path (sync API contract, section 6): mutations,
//! `EventInput` and partial changes, `DateValue`s; their validation and how
//! they become iCalendar.
//!
//! Validation errors are German and carry a stable code: `invalid_request`
//! for a malformed mutation, `invalid_event` for an event that cannot be
//! (end before start, mixed forms, missing title, unknown zone, bad rule).

use std::str::FromStr;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use lichtung_ical::{
    Attendee, CalTime, CalendarObject, Component, Event, EventEnd, EventStatus, IcalDuration,
    PartStat, Property, RDate, RecurRule, Transparency, ZoneResolver,
};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// Upper bounds for free text and lists.
pub const MAX_TITLE_CHARS: usize = 1_000;
pub const MAX_LOCATION_CHARS: usize = 1_000;
pub const MAX_NOTES_CHARS: usize = 100_000;
pub const MAX_URL_CHARS: usize = 2_000;
pub const MAX_UID_CHARS: usize = 255;
pub const MAX_ATTENDEES: usize = 500;
pub const MAX_ALARMS: usize = 20;
pub const MAX_RECURRENCE_DATES: usize = 1_000;
pub const MAX_COMMENT_CHARS: usize = 2_000;
/// Alarms and travel time: at most four weeks.
const MAX_MINUTES: i64 = 4 * 7 * 24 * 60;

/// `PRODID` of objects Lichtung creates.
pub const PRODID: &str = "-//Lichtung//Lichtung-Server//DE";

/// A validation error of a mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError {
    /// `invalid_request` or `invalid_event` (also `scope_not_supported`).
    pub code: &'static str,
    /// German, names the field.
    pub message: String,
}

impl InputError {
    pub fn request(message: impl Into<String>) -> Self {
        InputError {
            code: "invalid_request",
            message: message.into(),
        }
    }

    pub fn event(message: impl Into<String>) -> Self {
        InputError {
            code: "invalid_event",
            message: message.into(),
        }
    }

    pub fn scope(message: impl Into<String>) -> Self {
        InputError {
            code: "scope_not_supported",
            message: message.into(),
        }
    }
}

pub type InputResult<T> = Result<T, InputError>;

/// Missing field → `None`, `null` → `Some(None)`, value → `Some(Some(v))`.
pub type Patch<T> = Option<Option<T>>;

fn explicit_null<'de, D, T>(d: D) -> Result<Patch<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

// ----- mutations ----------------------------------------------------------------------

/// Geltungsbereich of an update or delete.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    All,
    This,
    ThisAndFollowing,
}

impl Scope {
    pub fn edit_scope(self) -> lichtung_ical::EditScope {
        match self {
            Scope::All => lichtung_ical::EditScope::All,
            Scope::This => lichtung_ical::EditScope::This,
            Scope::ThisAndFollowing => lichtung_ical::EditScope::ThisAndFuture,
        }
    }
}

/// One mutation of `POST /api/sync/push` (without its `id`). Unknown
/// fields of the envelope are ignored; those of an event are not.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Mutation {
    CreateEvent {
        calendar_id: String,
        event: EventInput,
    },
    UpdateEvent {
        event_id: String,
        base_version: i64,
        #[serde(default)]
        scope: Scope,
        #[serde(default)]
        occurrence: Option<DateInput>,
        changes: EventInput,
    },
    DeleteEvent {
        event_id: String,
        base_version: i64,
        #[serde(default)]
        scope: Scope,
        #[serde(default)]
        occurrence: Option<DateInput>,
    },
    Respond {
        event_id: String,
        #[serde(default)]
        occurrence: Option<DateInput>,
        status: ResponseStatus,
        #[serde(default)]
        comment: Option<String>,
    },
}

impl Mutation {
    /// Parses a mutation; the error names what is wrong.
    pub fn parse(raw: &Value) -> InputResult<Mutation> {
        serde_json::from_value(raw.clone())
            .map_err(|e| InputError::request(format!("Ungültige Mutation: {e}")))
    }
}

/// Answer to an invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Accepted,
    Tentative,
    Declined,
}

impl ResponseStatus {
    pub fn partstat(self) -> PartStat {
        match self {
            ResponseStatus::Accepted => PartStat::Accepted,
            ResponseStatus::Tentative => PartStat::Tentative,
            ResponseStatus::Declined => PartStat::Declined,
        }
    }

    /// German noun for messages.
    pub fn label(self) -> &'static str {
        match self {
            ResponseStatus::Accepted => "Zusage",
            ResponseStatus::Tentative => "Vielleicht",
            ResponseStatus::Declined => "Absage",
        }
    }
}

// ----- DateValue ----------------------------------------------------------------------

/// A `DateValue` sent by a client. `utc` is ignored (contract, section 1).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DateInput {
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub date_time: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub utc: Option<Value>,
}

fn parse_local(raw: &str) -> Option<NaiveDateTime> {
    ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(raw.trim(), f).ok())
}

impl DateInput {
    /// The value as iCalendar time; `field` names it in errors.
    pub fn to_caltime(&self, field: &str) -> InputResult<CalTime> {
        match (&self.date, &self.date_time) {
            (Some(d), None) => {
                if self.time_zone.is_some() {
                    return Err(InputError::event(format!(
                        "{field}: ein ganztägiger Wert (date) hat keine time_zone"
                    )));
                }
                NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d")
                    .map(CalTime::Date)
                    .map_err(|_| {
                        InputError::event(format!(
                            "{field}.date muss die Form JJJJ-MM-TT haben, nicht {d:?}"
                        ))
                    })
            }
            (None, Some(dt)) => {
                let local = parse_local(dt).ok_or_else(|| {
                    InputError::event(format!(
                        "{field}.date_time muss die Wanduhrzeit ohne Offset sein \
                         (JJJJ-MM-TTThh:mm:ss), nicht {dt:?}"
                    ))
                })?;
                match self.time_zone.as_deref().map(str::trim) {
                    None => Ok(CalTime::floating(local)),
                    Some("UTC") => Ok(CalTime::utc(local)),
                    Some(name) => {
                        let tz = Tz::from_str(name).map_err(|_| {
                            InputError::event(format!(
                                "{field}.time_zone ist keine bekannte IANA-Zeitzone: {name:?}"
                            ))
                        })?;
                        Ok(CalTime::zoned(local, tz.name()))
                    }
                }
            }
            (Some(_), Some(_)) => Err(InputError::event(format!(
                "{field} hat date und date_time, erlaubt ist nur eines"
            ))),
            (None, None) => Err(InputError::event(format!(
                "{field} braucht date oder date_time"
            ))),
        }
    }
}

// ----- EventInput ---------------------------------------------------------------------

/// `EventInput` (create) and its partial form (`changes` of an update).
/// Every field distinguishes "missing" from `null`. `all_day` is derived
/// from the form of `start`/`end`; a client may send it, it is ignored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventInput {
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub title: Patch<String>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub location: Patch<String>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub notes: Patch<String>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub url: Patch<String>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub start: Patch<DateInput>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub end: Patch<DateInput>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub status: Patch<String>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub availability: Patch<String>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub recurrence: Patch<RecurrenceInput>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub attendees: Patch<Vec<AttendeeInput>>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub alarms: Patch<Vec<AlarmInput>>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub travel_time_minutes: Patch<i64>,
    /// Only in `changes`: move to another calendar (scope `all`).
    #[serde(default)]
    pub calendar_id: Option<String>,
    #[serde(default)]
    pub all_day: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecurrenceInput {
    #[serde(default)]
    pub rrule: Option<String>,
    #[serde(default)]
    pub rdates: Vec<DateInput>,
    #[serde(default)]
    pub exdates: Vec<DateInput>,
    /// Echo of the server's flag; ignored.
    #[serde(default)]
    pub incomplete: Option<Value>,
}

/// An attendee. `status` and `is_self` (as the server sends them) are
/// ignored: the provider sets the status.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttendeeInput {
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<Value>,
    #[serde(default)]
    pub is_self: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlarmInput {
    #[serde(default)]
    pub minutes_before: Option<i64>,
    #[serde(default)]
    pub at: Option<String>,
}

/// A validated recurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recurrence {
    pub rrule: Option<RecurRule>,
    pub rdates: Vec<CalTime>,
    pub exdates: Vec<CalTime>,
}

/// A validated alarm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlarmSpec {
    /// Minutes before the start (negative = after).
    Before(i64),
    At(DateTime<Utc>),
}

/// Validated fields; `None` leaves a field untouched, `Some(None)` clears
/// it. Lists replace the whole list (`Some(vec![])` clears).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    pub title: Patch<String>,
    pub location: Patch<String>,
    pub notes: Patch<String>,
    pub url: Patch<String>,
    pub start: Option<CalTime>,
    pub end: Option<CalTime>,
    pub status: Patch<EventStatus>,
    pub transparency: Option<Transparency>,
    pub recurrence: Patch<Recurrence>,
    pub attendees: Option<Vec<Attendee>>,
    pub alarms: Option<Vec<AlarmSpec>>,
    pub travel: Patch<IcalDuration>,
    pub calendar_id: Option<String>,
}

impl Fields {
    /// Whether anything besides the calendar changes.
    pub fn changes_event(&self) -> bool {
        let without_calendar = Fields {
            calendar_id: None,
            ..self.clone()
        };
        without_calendar != Fields::default()
    }
}

fn text(field: &str, value: &Patch<String>, max: usize) -> InputResult<Patch<String>> {
    let Some(Some(v)) = value else {
        return Ok(value.clone());
    };
    if v.chars().count() > max {
        return Err(InputError::event(format!(
            "{field} ist zu lang (höchstens {max} Zeichen)"
        )));
    }
    // Empty optional text is the same as no text.
    Ok(Some(Some(v.clone()).filter(|v| !v.trim().is_empty())))
}

fn status(raw: &str) -> InputResult<EventStatus> {
    match raw {
        "confirmed" => Ok(EventStatus::Confirmed),
        "tentative" => Ok(EventStatus::Tentative),
        "cancelled" => Ok(EventStatus::Cancelled),
        other => Err(InputError::event(format!(
            "status muss confirmed, tentative oder cancelled sein, nicht {other:?}"
        ))),
    }
}

fn role(raw: Option<&str>) -> InputResult<Option<String>> {
    Ok(Some(
        match raw.unwrap_or("required") {
            "chair" => "CHAIR",
            "required" => "REQ-PARTICIPANT",
            "optional" => "OPT-PARTICIPANT",
            "non_participant" => "NON-PARTICIPANT",
            other => {
                return Err(InputError::event(format!(
                    "attendees.role muss chair, required, optional oder non_participant sein, \
                     nicht {other:?}"
                )))
            }
        }
        .to_owned(),
    ))
}

fn minutes(field: &str, m: i64) -> InputResult<i64> {
    if m.abs() > MAX_MINUTES {
        return Err(InputError::event(format!(
            "{field} liegt außerhalb von ±{MAX_MINUTES} Minuten"
        )));
    }
    Ok(m)
}

/// Validates the fields of an `EventInput` or of `changes`.
pub fn resolve(input: &EventInput) -> InputResult<Fields> {
    let mut f = Fields {
        title: text("title", &input.title, MAX_TITLE_CHARS)?,
        location: text("location", &input.location, MAX_LOCATION_CHARS)?,
        notes: text("notes", &input.notes, MAX_NOTES_CHARS)?,
        url: text("url", &input.url, MAX_URL_CHARS)?,
        ..Fields::default()
    };
    if matches!(input.title, Some(None)) {
        return Err(InputError::event("title darf nicht null sein"));
    }
    // An empty title stays a (blank) title rather than a removal.
    if let Some(Some(t)) = &input.title {
        f.title = Some(Some(t.clone()));
    }
    f.start = match &input.start {
        None => None,
        Some(None) => return Err(InputError::event("start darf nicht null sein")),
        Some(Some(d)) => Some(d.to_caltime("start")?),
    };
    f.end = match &input.end {
        None => None,
        Some(None) => return Err(InputError::event("end darf nicht null sein")),
        Some(Some(d)) => Some(d.to_caltime("end")?),
    };
    if let (Some(s), Some(e)) = (&f.start, &f.end) {
        check_order(s, e)?;
    }
    f.status = match &input.status {
        None => None,
        Some(None) => Some(None),
        Some(Some(s)) => Some(Some(status(s)?)),
    };
    f.transparency = match &input.availability {
        None => None,
        Some(None) => Some(Transparency::Opaque),
        Some(Some(a)) => Some(match a.as_str() {
            "busy" => Transparency::Opaque,
            "free" => Transparency::Transparent,
            other => {
                return Err(InputError::event(format!(
                    "availability muss busy oder free sein, nicht {other:?}"
                )))
            }
        }),
    };
    f.recurrence = match &input.recurrence {
        None => None,
        Some(None) => Some(None),
        Some(Some(r)) => Some(Some(recurrence(r)?)),
    };
    f.attendees = match &input.attendees {
        None => None,
        Some(None) => Some(Vec::new()),
        Some(Some(list)) => {
            if list.len() > MAX_ATTENDEES {
                return Err(InputError::event(format!(
                    "höchstens {MAX_ATTENDEES} Teilnehmer"
                )));
            }
            let mut out: Vec<Attendee> = Vec::with_capacity(list.len());
            for a in list {
                let email = a.email.trim().trim_start_matches("mailto:").to_owned();
                let valid = email.contains('@')
                    && !email.chars().any(|c| c.is_whitespace() || c.is_control())
                    && email.len() <= 320;
                if !valid {
                    return Err(InputError::event(format!(
                        "attendees.email ist keine E-Mail-Adresse: {:?}",
                        a.email
                    )));
                }
                if out.iter().any(|o| o.email().eq_ignore_ascii_case(&email)) {
                    continue;
                }
                out.push(Attendee {
                    address: format!("mailto:{email}"),
                    cn: a
                        .name
                        .clone()
                        .filter(|n| !n.trim().is_empty() && !n.chars().any(char::is_control)),
                    partstat: Some(PartStat::NeedsAction),
                    role: role(a.role.as_deref())?,
                    rsvp: Some(true),
                });
            }
            Some(out)
        }
    };
    f.alarms = match &input.alarms {
        None => None,
        Some(None) => Some(Vec::new()),
        Some(Some(list)) => {
            if list.len() > MAX_ALARMS {
                return Err(InputError::event(format!(
                    "höchstens {MAX_ALARMS} Erinnerungen"
                )));
            }
            let mut out = Vec::new();
            for a in list {
                out.push(match (a.minutes_before, &a.at) {
                    (Some(m), None) => AlarmSpec::Before(minutes("alarms.minutes_before", m)?),
                    (None, Some(at)) => AlarmSpec::At(
                        DateTime::parse_from_rfc3339(at.trim())
                            .map_err(|_| {
                                InputError::event(format!(
                                    "alarms.at muss ein Zeitpunkt nach RFC 3339 sein: {at:?}"
                                ))
                            })?
                            .with_timezone(&Utc),
                    ),
                    _ => {
                        return Err(InputError::event(
                            "eine Erinnerung hat entweder minutes_before oder at",
                        ))
                    }
                });
            }
            Some(out)
        }
    };
    f.travel = match input.travel_time_minutes {
        None => None,
        Some(None) => Some(None),
        Some(Some(m)) if m < 0 => {
            return Err(InputError::event(
                "travel_time_minutes darf nicht negativ sein",
            ))
        }
        Some(Some(0)) => Some(None),
        Some(Some(m)) => Some(Some(IcalDuration::from_seconds(
            minutes("travel_time_minutes", m)? * 60,
        ))),
    };
    f.calendar_id = input.calendar_id.clone();
    Ok(f)
}

fn recurrence(r: &RecurrenceInput) -> InputResult<Recurrence> {
    let rrule = match r.rrule.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => {
            let raw = raw.strip_prefix("RRULE:").unwrap_or(raw);
            let rule = RecurRule::parse(raw)
                .map_err(|e| InputError::event(format!("recurrence.rrule ist ungültig: {e}")))?;
            if rule.freq().is_none() {
                return Err(InputError::event("recurrence.rrule braucht FREQ"));
            }
            Some(rule)
        }
    };
    if r.rdates.len() + r.exdates.len() > MAX_RECURRENCE_DATES {
        return Err(InputError::event(format!(
            "höchstens {MAX_RECURRENCE_DATES} Einträge in rdates und exdates"
        )));
    }
    let dates = |list: &[DateInput], field: &str| -> InputResult<Vec<CalTime>> {
        list.iter().map(|d| d.to_caltime(field)).collect()
    };
    Ok(Recurrence {
        rrule,
        rdates: dates(&r.rdates, "recurrence.rdates")?,
        exdates: dates(&r.exdates, "recurrence.exdates")?,
    })
}

fn resolver() -> ZoneResolver {
    ZoneResolver::new(crate::events::default_zone())
}

/// Start and end: same form, floating only together, end not before start
/// (a whole day at least for dates).
pub fn check_order(start: &CalTime, end: &CalTime) -> InputResult<()> {
    if start.is_date() != end.is_date() {
        return Err(InputError::event(
            "start und end müssen dieselbe Form haben: beide ganztägig (date) oder beide mit \
             Uhrzeit (date_time)",
        ));
    }
    let floating = |t: &CalTime| {
        matches!(t.zone(), Some(lichtung_ical::TimeZoneRef::Floating) | None) && !t.is_date()
    };
    if floating(start) != floating(end) {
        return Err(InputError::event(
            "start und end müssen beide eine Zeitzone haben oder beide ohne sein (floating)",
        ));
    }
    if start.is_date() {
        if end.date() <= start.date() {
            return Err(InputError::event(
                "Das Ende liegt vor dem Beginn (bei ganztägigen Terminen ist end exklusiv)",
            ));
        }
        return Ok(());
    }
    let r = resolver();
    if r.to_utc(end) < r.to_utc(start) {
        return Err(InputError::event("Das Ende liegt vor dem Beginn"));
    }
    Ok(())
}

/// Checks every `VEVENT` of an edited object (start/end order and form).
pub fn check_object(object: &CalendarObject) -> InputResult<()> {
    for ev in object.events() {
        let Ok(start) = ev.start() else { continue };
        if let Ok(EventEnd::End(end)) = ev.end() {
            check_order(&start, &end)?;
        }
    }
    Ok(())
}

// ----- building iCalendar -------------------------------------------------------------

/// Builds a new calendar object from validated fields (`create_event`).
/// `organizer` is the own address, written as `ORGANIZER` when the event
/// has attendees.
pub fn build_object(
    uid: &str,
    f: &Fields,
    now: DateTime<Utc>,
    organizer: Option<&str>,
) -> CalendarObject {
    let mut calendar = Component::new("VCALENDAR");
    calendar.insert_property(Property::new("VERSION", "2.0"));
    calendar.insert_property(Property::new("PRODID", PRODID));
    calendar.insert_property(Property::new("CALSCALE", "GREGORIAN"));

    let stamp = CalTime::utc(now.naive_utc()).format_value();
    let mut ev = Event::new(Component::new("VEVENT"));
    ev.set_uid(uid);
    let c = ev.component_mut();
    c.insert_property(Property::new("DTSTAMP", &stamp));
    c.insert_property(Property::new("CREATED", &stamp));
    c.insert_property(Property::new("LAST-MODIFIED", &stamp));
    ev.set_sequence(0);
    if let Some(start) = &f.start {
        ev.set_start(start);
    }
    if let Some(end) = &f.end {
        ev.set_end(&EventEnd::End(end.clone()));
    }
    if let Some(Some(t)) = &f.title {
        ev.set_summary(Some(t));
    }
    if let Some(Some(v)) = &f.location {
        ev.set_location(Some(v));
    }
    if let Some(Some(v)) = &f.notes {
        ev.set_description(Some(v));
    }
    if let Some(Some(v)) = &f.url {
        ev.set_url(Some(v));
    }
    if let Some(Some(s)) = &f.status {
        ev.set_status(Some(s));
    }
    if let Some(t) = f.transparency {
        if t == Transparency::Transparent {
            ev.set_transparency(t);
        }
    }
    if let Some(Some(r)) = &f.recurrence {
        set_recurrence(&mut ev, r);
    }
    if let Some(Some(d)) = f.travel {
        ev.set_travel_duration(Some(d));
    }
    if let Some(list) = &f.attendees {
        set_attendees(&mut ev, list, organizer);
    }
    if let Some(list) = &f.alarms {
        set_alarms(&mut ev, list);
    }
    calendar.components_mut().push(ev.into_inner());
    CalendarObject::from_component(calendar).expect("VCALENDAR")
}

/// Writes RRULE, RDATE and EXDATE of a series master (replacing them).
pub fn set_recurrence<C: std::borrow::BorrowMut<Component>>(ev: &mut Event<C>, r: &Recurrence) {
    ev.set_rrule(r.rrule.as_ref());
    set_recurrence_dates(ev, r);
}

/// Writes RDATE and EXDATE of a series master (replacing them), not the
/// RRULE: on an edit, `lichtung_ical::edit_event` sets that (LIC-57).
pub fn set_recurrence_dates<C: std::borrow::BorrowMut<Component>>(
    ev: &mut Event<C>,
    r: &Recurrence,
) {
    let rdates: Vec<RDate> = r.rdates.iter().cloned().map(RDate::Time).collect();
    ev.set_rdates(&rdates);
    ev.set_exdates(&r.exdates);
}

/// Replaces the attendees: known ones keep their parameters (status,
/// RSVP) and get the new name and role, missing ones are removed, new ones
/// are added with `NEEDS-ACTION` and `RSVP=TRUE`. With attendees and no
/// `ORGANIZER`, the own address becomes the organizer.
pub fn set_attendees<C: std::borrow::BorrowMut<Component>>(
    ev: &mut Event<C>,
    wanted: &[Attendee],
    organizer: Option<&str>,
) {
    let email_of = |p: &Property| {
        let v = p.value().trim();
        v.get(..7)
            .filter(|s| s.eq_ignore_ascii_case("mailto:"))
            .map_or(v, |_| &v[7..])
            .to_ascii_lowercase()
    };
    let props = ev.component_mut().properties_mut();
    props.retain(|p| {
        !p.is("ATTENDEE")
            || wanted
                .iter()
                .any(|a| a.email().eq_ignore_ascii_case(&email_of(p)))
    });
    for a in wanted {
        let existing = props
            .iter_mut()
            .find(|p| p.is("ATTENDEE") && a.email().eq_ignore_ascii_case(&email_of(p)));
        match existing {
            Some(p) => {
                match &a.cn {
                    Some(cn) if p.param("CN") != Some(cn.as_str()) => p.set_param("CN", cn.clone()),
                    _ => {}
                }
                if let Some(role) = &a.role {
                    if p.param("ROLE") != Some(role.as_str()) {
                        p.set_param("ROLE", role.clone());
                    }
                }
            }
            None => ev_add_attendee(props, a),
        }
    }
    if !wanted.is_empty() && ev.organizer().is_none() {
        if let Some(address) = organizer {
            ev.component_mut()
                .set_property(Property::new("ORGANIZER", format!("mailto:{address}")));
        }
    }
}

fn ev_add_attendee(props: &mut Vec<Property>, a: &Attendee) {
    let mut ev = Event::new(Component::new("VEVENT"));
    ev.add_attendee(a);
    props.extend(ev.into_inner().properties().iter().cloned());
}

/// Replaces all alarms.
pub fn set_alarms<C: std::borrow::BorrowMut<Component>>(ev: &mut Event<C>, alarms: &[AlarmSpec]) {
    ev.remove_alarms();
    let description = ev.summary().filter(|s| !s.trim().is_empty());
    let description = description.as_deref().unwrap_or("Erinnerung");
    for a in alarms {
        match a {
            AlarmSpec::Before(m) => {
                ev.add_relative_alarm(IcalDuration::from_seconds(-m * 60), description)
            }
            AlarmSpec::At(at) => {
                let mut alarm = Component::new("VALARM");
                alarm.insert_property(Property::new("ACTION", "DISPLAY"));
                alarm.insert_property(
                    Property::new("TRIGGER", CalTime::utc(at.naive_utc()).format_value())
                        .with_param("VALUE", "DATE-TIME"),
                );
                alarm.insert_property(Property::new_text("DESCRIPTION", description));
                ev.component_mut().components_mut().push(alarm);
            }
        }
    }
}

/// The changes `lichtung_ical::edit_event` understands.
pub fn edit_changes(f: &Fields) -> lichtung_ical::EventChanges {
    lichtung_ical::EventChanges {
        summary: f.title.clone(),
        location: f.location.clone(),
        description: f.notes.clone(),
        url: f.url.clone(),
        status: f.status.clone(),
        transparency: f.transparency,
        travel_duration: f.travel,
        start: f.start.clone(),
        end: f.end.clone(),
        rrule: f
            .recurrence
            .as_ref()
            .map(|r| r.as_ref().and_then(|r| r.rrule.clone())),
    }
}

/// What a change does, for conflict messages (German, short).
pub fn describe(f: &Fields, calendar_name: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = &f.title {
        parts.push(format!("Titel „{}“", t.as_deref().unwrap_or("")));
    }
    if let Some(s) = &f.start {
        parts.push(format!("Beginn {}", fmt_caltime(s)));
    }
    if let Some(e) = &f.end {
        parts.push(format!("Ende {}", fmt_caltime(e)));
    }
    for (changed, label) in [
        (f.location.is_some(), "Ort"),
        (f.notes.is_some(), "Notiz"),
        (f.url.is_some(), "Link"),
        (f.status.is_some(), "Status"),
        (f.transparency.is_some(), "Verfügbarkeit"),
        (f.recurrence.is_some(), "Wiederholung"),
        (f.attendees.is_some(), "Teilnehmer"),
        (f.alarms.is_some(), "Erinnerungen"),
        (f.travel.is_some(), "Reisezeit"),
    ] {
        if changed {
            parts.push(label.to_owned());
        }
    }
    if f.calendar_id.is_some() {
        parts.push(match calendar_name {
            Some(name) => format!("Kalender „{name}“"),
            None => "Kalender".to_owned(),
        });
    }
    parts.join(", ")
}

/// `3.11.2026` or `3.11.2026, 14:00`.
pub fn fmt_caltime(t: &CalTime) -> String {
    if t.is_date() {
        t.date().format("%-d.%-m.%Y").to_string()
    } else {
        t.naive().format("%-d.%-m.%Y, %H:%M").to_string()
    }
}

/// Checks a client UID (create).
pub fn check_uid(uid: &str) -> InputResult<String> {
    let uid = uid.trim();
    if uid.is_empty() || uid.chars().count() > MAX_UID_CHARS {
        return Err(InputError::event(format!(
            "uid muss 1 bis {MAX_UID_CHARS} Zeichen lang sein"
        )));
    }
    if uid.chars().any(char::is_control) {
        return Err(InputError::event("uid darf keine Steuerzeichen enthalten"));
    }
    Ok(uid.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(v: Value) -> EventInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn date_values_parse_in_every_form() {
        let d = |v: Value| serde_json::from_value::<DateInput>(v).unwrap();
        assert_eq!(
            d(json!({"date": "2026-11-03"}))
                .to_caltime("start")
                .unwrap(),
            CalTime::Date(NaiveDate::from_ymd_opt(2026, 11, 3).unwrap())
        );
        let zoned = d(
            json!({"date_time": "2026-11-03T14:00:00", "time_zone": "Europe/Berlin",
                             "utc": "2026-11-03T13:00:00Z"}),
        )
        .to_caltime("start")
        .unwrap();
        assert_eq!(zoned.tzid(), Some("Europe/Berlin"));
        let utc = d(json!({"date_time": "2026-11-03T13:00", "time_zone": "UTC"}))
            .to_caltime("start")
            .unwrap();
        assert_eq!(utc.format_value(), "20261103T130000Z");
        let floating = d(json!({"date_time": "2026-11-03T14:00:00", "time_zone": null}))
            .to_caltime("start")
            .unwrap();
        assert_eq!(floating.format_value(), "20261103T140000");
        for bad in [
            json!({"date_time": "2026-11-03T14:00:00", "time_zone": "Mars/Olympus"}),
            json!({"date_time": "2026-11-03T14:00:00+01:00"}),
            json!({"date": "3.11.2026"}),
            json!({"date": "2026-11-03", "date_time": "2026-11-03T14:00:00"}),
            json!({}),
        ] {
            let e = d(bad.clone()).to_caltime("start").unwrap_err();
            assert_eq!(e.code, "invalid_event", "{bad}");
        }
    }

    #[test]
    fn validation_catches_order_forms_and_values() {
        let e = resolve(&input(json!({
            "start": {"date_time": "2026-11-03T14:00:00", "time_zone": "Europe/Berlin"},
            "end": {"date_time": "2026-11-03T13:00:00", "time_zone": "Europe/Berlin"}
        })))
        .unwrap_err();
        assert_eq!(e.message, "Das Ende liegt vor dem Beginn");
        let e = resolve(&input(json!({
            "start": {"date": "2026-11-03"},
            "end": {"date_time": "2026-11-03T13:00:00", "time_zone": "Europe/Berlin"}
        })))
        .unwrap_err();
        assert!(e.message.contains("dieselbe Form"), "{}", e.message);
        let e = resolve(&input(json!({
            "start": {"date": "2026-11-03"}, "end": {"date": "2026-11-03"}
        })))
        .unwrap_err();
        assert!(e.message.contains("exklusiv"));
        // Same instant in different zones is fine.
        resolve(&input(json!({
            "start": {"date_time": "2026-11-03T14:00:00", "time_zone": "Europe/Berlin"},
            "end": {"date_time": "2026-11-03T13:00:00", "time_zone": "UTC"}
        })))
        .unwrap();
        assert_eq!(
            resolve(&input(json!({"status": "abgesagt"})))
                .unwrap_err()
                .code,
            "invalid_event"
        );
        assert!(resolve(&input(json!({"title": null}))).is_err());
        assert!(resolve(&input(json!({"recurrence": {"rrule": "BYDAY=MO"}}))).is_err());
        assert!(resolve(&input(json!({"attendees": [{"email": "kein-at"}]}))).is_err());
        assert!(resolve(&input(json!({"alarms": [{}]}))).is_err());
        assert!(serde_json::from_value::<EventInput>(json!({"titel": "x"})).is_err());
    }

    #[test]
    fn patches_distinguish_null_from_missing() {
        let f = resolve(&input(
            json!({"location": null, "alarms": null, "notes": "  "}),
        ))
        .unwrap();
        assert_eq!(f.location, Some(None));
        assert_eq!(f.notes, Some(None));
        assert_eq!(f.alarms, Some(Vec::new()));
        assert_eq!(f.url, None);
        assert_eq!(f.attendees, None);
        assert!(f.changes_event());
        assert!(!resolve(&input(json!({"calendar_id": "cal_2"})))
            .unwrap()
            .changes_event());
    }

    #[test]
    fn builds_a_complete_object() {
        let f = resolve(&input(json!({
            "title": "Zahnarzt, Kontrolle",
            "location": "Praxis am Markt",
            "notes": "Karte mitbringen\nund pünktlich sein",
            "start": {"date_time": "2026-11-03T14:00:00", "time_zone": "Europe/Berlin"},
            "end": {"date_time": "2026-11-03T15:00:00", "time_zone": "Europe/Berlin"},
            "availability": "free",
            "recurrence": {"rrule": "FREQ=WEEKLY;COUNT=3",
                           "exdates": [{"date_time": "2026-11-10T14:00:00", "time_zone": "Europe/Berlin"}]},
            "attendees": [{"email": "bert@example.org", "name": "Bert", "role": "optional"}],
            "alarms": [{"minutes_before": 60}, {"at": "2026-11-03T12:00:00Z"}],
            "travel_time_minutes": 15
        })))
        .unwrap();
        let now = DateTime::parse_from_rfc3339("2026-10-23T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let obj = build_object("u-1@example.org", &f, now, Some("anna@example.org"));
        let ics = obj.to_ics().replace("\r\n ", "");
        for line in [
            "PRODID:-//Lichtung//Lichtung-Server//DE",
            "UID:u-1@example.org",
            "DTSTAMP:20261023T080000Z",
            "SEQUENCE:0",
            "DTSTART;TZID=Europe/Berlin:20261103T140000",
            "DTEND;TZID=Europe/Berlin:20261103T150000",
            "SUMMARY:Zahnarzt\\, Kontrolle",
            "DESCRIPTION:Karte mitbringen\\nund pünktlich sein",
            "TRANSP:TRANSPARENT",
            "RRULE:FREQ=WEEKLY;COUNT=3",
            "EXDATE;TZID=Europe/Berlin:20261110T140000",
            "ORGANIZER:mailto:anna@example.org",
            "TRIGGER:-PT1H",
            "TRIGGER;VALUE=DATE-TIME:20261103T120000Z",
            "X-APPLE-TRAVEL-DURATION;VALUE=DURATION:PT15M",
        ] {
            assert!(ics.contains(line), "{line} missing in\n{ics}");
        }
        assert!(ics.contains("ATTENDEE;CN=Bert;ROLE=OPT-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:bert@example.org"), "{ics}");
        // Parses back and is readable.
        crate::events::derive(&obj.to_ics(), chrono_tz::Europe::Berlin).unwrap();
    }

    #[test]
    fn attendees_keep_their_status() {
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//DE\r\nBEGIN:VEVENT\r\n\
                   UID:a@example.org\r\nDTSTART:20261103T130000Z\r\n\
                   ORGANIZER:mailto:anna@example.org\r\n\
                   ATTENDEE;PARTSTAT=ACCEPTED;CN=Bert:mailto:bert@example.org\r\n\
                   ATTENDEE;PARTSTAT=DECLINED:mailto:cora@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let mut obj = CalendarObject::parse(ics).unwrap();
        let f = resolve(&input(json!({"attendees": [
            {"email": "BERT@example.org", "name": "Bert B."}, {"email": "dora@example.org"}
        ]})))
        .unwrap();
        set_attendees(
            &mut obj.master_mut().unwrap(),
            f.attendees.as_ref().unwrap(),
            None,
        );
        let out = obj.to_ics().replace("\r\n ", "");
        assert!(out.contains("ATTENDEE;PARTSTAT=ACCEPTED;CN=Bert B.;ROLE=REQ-PARTICIPANT:mailto:bert@example.org"), "{out}");
        assert!(!out.contains("cora@"));
        assert!(out.contains("PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:dora@example.org"));
    }
}
