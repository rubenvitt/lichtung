//! Openings (spec section 6, LIC-31/LIC-32): booking links, managed in the
//! app, booked by guests on the public page `<public_url>/o/<token>`.
//!
//! - **One-off link** (`once`): length and target calendar, optionally
//!   concrete suggestions. Used up by one booking (`booked_at`). Without
//!   suggestions it offers free times by the same rules as a template, with
//!   the defaults below.
//! - **Template** (`template`): weekly windows in a zone, buffers before
//!   and after, minimum notice, horizon, optional fixed video link.
//!   Bookable again and again.
//! - Links do not expire. The token has 256 random bits
//!   ([`crate::auth::random_secret`]).
//! - **Target calendar:** must exist, be writable for Lichtung and send
//!   invitations (`can_invite`); else `400` with `calendar_not_found`,
//!   `calendar_read_only` or `calendar_cannot_invite`.
//!
//! **Free time** follows [`crate::free`] (calendars with `openings_busy`),
//! the slots [`slots`].
//!
//! **Placeholders** (LIC-32): for each suggestion the server creates an
//! event without attendees ("Reserviert: <title>") in the target calendar,
//! through the write path (origin `server`), so the provider has it like
//! any other event. Placeholders are busy everywhere except for their own
//! link. They are removed when the link is booked
//! ([`App::complete_opening_booking`]), when it is deleted, and when the
//! suggestion's start has passed (cleanup every [`CLEANUP_INTERVAL`], and
//! past suggestions are never shown or offered). Removals go through a
//! queue (`placeholder_removals`): what the provider refuses for the moment
//! is tried again by the next cleanup. A placeholder deleted at the
//! provider simply is gone (`placeholder_status: missing`). If a
//! placeholder cannot be created, the link is not created either (the
//! placeholders created so far are removed again) and the error names the
//! cause.
//!
//! The booking page (LIC-33) calls [`App::opening_by_token`],
//! [`App::opening_slots`], and under [`App::opening_booking_guard`]
//! [`App::check_opening_slot`], creates the event and
//! [`App::mark_opening_booked`]; after releasing the guard it calls
//! [`App::remove_booked_placeholders`].
//!
//! **Bookings** (LIC-33, LIC-35): what the guest entered (name, address,
//! note) and the booked time are stored per booking in `opening_bookings`
//! and shown only in the management API and the pull (`bookings`, newest
//! [`MAX_LISTED_BOOKINGS`]). The booked event carries none of it except
//! the guest as attendee: the provider sends it as an invitation from the
//! own address to the address the guest typed. Bookings go with their
//! link; the cleanup deletes those made and ended more than
//! [`BOOKING_RETENTION_DAYS`] ago.

pub mod slots;
pub mod store;

use std::str::FromStr;

use chrono::{DateTime, Duration, NaiveTime, SubsecRound, Utc, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::accounts::store as accounts_store;
use crate::accounts::AccountKind;
use crate::app::{format_ms, App};
use crate::error::{AppError, AppResult};
use crate::events::default_zone;
use crate::events::store as events_store;
use crate::free::WeeklyWindow;
use crate::writes::{Origin, WriteOutcome};
pub use slots::Slot;
use slots::{bookable, Rules};
pub use store::OpeningKind;
use store::{BookingRow, OpeningRow, SuggestionRow};

/// Path of the public booking page: `<public_url>/o/<token>`.
pub const PUBLIC_PATH: &str = "o";
/// Most suggestions of a one-off link.
pub const MAX_SUGGESTIONS: usize = 20;
/// Most weekly windows of a link.
pub const MAX_WINDOWS: usize = 50;
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_VIDEO_URL_CHARS: usize = 2_000;
/// Length of a link: 5 minutes to 12 hours.
pub const MIN_DURATION_MINUTES: i64 = 5;
pub const MAX_DURATION_MINUTES: i64 = 12 * 60;
/// Buffers: up to 12 hours.
pub const MAX_BUFFER_MINUTES: i64 = 12 * 60;
/// Minimum notice: up to 30 days.
pub const MAX_NOTICE_MINUTES: i64 = 30 * 24 * 60;
/// Horizon: 1 to 365 days.
pub const MAX_HORIZON_DAYS: i64 = 365;
/// Defaults: 4 hours notice, 3 weeks horizon, no buffers, Monday to
/// Friday 09:00–17:00 in the server's zone.
pub const DEFAULT_MIN_NOTICE_MINUTES: i64 = 4 * 60;
pub const DEFAULT_HORIZON_DAYS: i64 = 21;
pub const DEFAULT_WINDOW: (&str, &str) = ("09:00", "17:00");
/// Most bookings an opening object lists (newest first).
pub const MAX_LISTED_BOOKINGS: usize = 50;
/// Bookings made and ended longer ago than this are deleted by the cleanup.
pub const BOOKING_RETENTION_DAYS: i64 = 365;
/// How often past suggestions are cleaned up and failed removals retried.
pub const CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Weekday codes as in MCP (`mo` … `su`).
const WEEKDAYS: [(&str, Weekday); 7] = [
    ("mo", Weekday::Mon),
    ("tu", Weekday::Tue),
    ("we", Weekday::Wed),
    ("th", Weekday::Thu),
    ("fr", Weekday::Fri),
    ("sa", Weekday::Sat),
    ("su", Weekday::Sun),
];

// ----- API objects ---------------------------------------------------------------------

/// A weekly time window, `start`/`end` as `hh:mm` in the link's zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub weekday: String,
    pub start: String,
    pub end: String,
}

/// A suggestion of a one-off link with the state of its placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    pub id: String,
    pub start: String,
    pub end: String,
    pub placeholder_event_id: Option<String>,
    /// `reserved` (the placeholder exists), `missing` (deleted at the
    /// provider), `pending` (being created).
    pub placeholder_status: &'static str,
}

/// A booking of a link with what the guest entered. Only in the
/// management API and the pull: the booked event (and so the invitation
/// the provider sends) carries none of it except the guest as attendee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Booking {
    pub id: String,
    pub event_id: String,
    pub start: String,
    pub end: String,
    pub guest_name: String,
    pub guest_email: String,
    pub note: Option<String>,
    pub created_at: String,
}

/// What the guest entered on the booking page, checked
/// ([`crate::booking::guard`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guest {
    pub name: String,
    pub email: String,
    pub note: Option<String>,
}

/// An opening as the API shows it (`docs/api/openings.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Opening {
    pub id: String,
    pub kind: OpeningKind,
    /// The public booking page.
    pub url: String,
    pub title: String,
    pub duration_minutes: i64,
    pub calendar_id: String,
    pub time_zone: String,
    pub windows: Vec<Window>,
    pub buffer_before_minutes: i64,
    pub buffer_after_minutes: i64,
    pub min_notice_minutes: i64,
    pub horizon_days: i64,
    pub video_url: Option<String>,
    /// The link offers only its suggestions (one-off link created with
    /// suggestions), even once they are all past.
    pub suggestions_only: bool,
    /// Suggestions that have not started yet, earliest first.
    pub suggestions: Vec<Suggestion>,
    pub booked_at: Option<String>,
    pub booked_event_id: Option<String>,
    /// The newest [`MAX_LISTED_BOOKINGS`] bookings, newest first (empty on
    /// the public booking page).
    pub bookings: Vec<Booking>,
    pub created_at: String,
    pub updated_at: String,
    /// The secret part of `url`.
    #[serde(skip)]
    pub token: String,
}

/// `POST /api/openings`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewOpening {
    pub kind: OpeningKind,
    #[serde(default)]
    pub title: Option<String>,
    pub duration_minutes: i64,
    pub calendar_id: String,
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub windows: Option<Vec<Window>>,
    #[serde(default)]
    pub buffer_before_minutes: Option<i64>,
    #[serde(default)]
    pub buffer_after_minutes: Option<i64>,
    #[serde(default)]
    pub min_notice_minutes: Option<i64>,
    #[serde(default)]
    pub horizon_days: Option<i64>,
    #[serde(default)]
    pub video_url: Option<String>,
    #[serde(default)]
    pub suggestions: Option<Vec<SuggestionInput>>,
}

/// A suggestion when creating a one-off link: RFC 3339 with offset; `end`
/// defaults to `start` plus the link's length.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestionInput {
    pub start: String,
    #[serde(default)]
    pub end: Option<String>,
}

/// `PATCH /api/openings/{id}`: only the fields sent change; `video_url:
/// null` removes the link. Suggestions and `kind` cannot change.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpeningPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub duration_minutes: Option<i64>,
    #[serde(default)]
    pub calendar_id: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub windows: Option<Vec<Window>>,
    #[serde(default)]
    pub buffer_before_minutes: Option<i64>,
    #[serde(default)]
    pub buffer_after_minutes: Option<i64>,
    #[serde(default)]
    pub min_notice_minutes: Option<i64>,
    #[serde(default)]
    pub horizon_days: Option<i64>,
    #[serde(default, deserialize_with = "explicit_null")]
    pub video_url: Option<Option<String>>,
}

/// Missing field → `None`, `null` → `Some(None)`, value → `Some(Some(v))`.
fn explicit_null<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// What a removal round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RemovalReport {
    /// Deleted at the provider (or already gone).
    pub removed: usize,
    /// Left in the queue for the next round (provider unavailable).
    pub pending: usize,
}

// ----- validation ----------------------------------------------------------------------

fn invalid(message: impl Into<String>) -> AppError {
    AppError::bad_request(message)
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::now_v7().simple())
}

fn check_title(raw: &str) -> AppResult<String> {
    let title = raw.trim().to_owned();
    let len = title.chars().count();
    if len == 0 || len > MAX_TITLE_CHARS {
        return Err(invalid(format!(
            "title muss 1 bis {MAX_TITLE_CHARS} Zeichen lang sein"
        )));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid("title darf keine Steuerzeichen enthalten"));
    }
    Ok(title)
}

fn check_range(name: &str, value: i64, min: i64, max: i64) -> AppResult<i64> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(invalid(format!(
            "{name} muss zwischen {min} und {max} liegen, nicht {value}"
        )))
    }
}

fn check_duration(value: i64) -> AppResult<i64> {
    check_range(
        "duration_minutes",
        value,
        MIN_DURATION_MINUTES,
        MAX_DURATION_MINUTES,
    )
}

fn check_zone(raw: &str) -> AppResult<String> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("utc") {
        return Ok("UTC".to_owned());
    }
    Tz::from_str(raw)
        .map(|tz| tz.name().to_owned())
        .map_err(|_| {
            invalid(format!(
                "time_zone muss eine IANA-Zeitzone sein (z. B. Europe/Berlin), nicht „{raw}“"
            ))
        })
}

fn weekday_of(code: &str) -> Option<Weekday> {
    WEEKDAYS.iter().find(|(c, _)| *c == code).map(|(_, d)| *d)
}

fn parse_clock(raw: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(raw.trim(), "%H:%M").ok()
}

/// Checks and normalizes weekly windows (codes `mo` … `su`, `hh:mm`, end
/// after start, `24:00` not allowed: a window ends at 23:59 at the latest).
fn check_windows(list: Vec<Window>) -> AppResult<Vec<Window>> {
    if list.is_empty() {
        return Err(invalid(
            "windows braucht mindestens ein Zeitfenster, z. B. {\"weekday\": \"mo\", \"start\": \
             \"09:00\", \"end\": \"17:00\"}",
        ));
    }
    if list.len() > MAX_WINDOWS {
        return Err(invalid(format!(
            "windows hat höchstens {MAX_WINDOWS} Zeitfenster"
        )));
    }
    let mut out = Vec::with_capacity(list.len());
    for w in list {
        let code = w.weekday.trim().to_lowercase();
        if weekday_of(&code).is_none() {
            return Err(invalid(format!(
                "Ungültiger Wochentag „{}“ in windows. Erlaubt: mo, tu, we, th, fr, sa, su",
                w.weekday
            )));
        }
        let (Some(s), Some(e)) = (parse_clock(&w.start), parse_clock(&w.end)) else {
            return Err(invalid(format!(
                "Zeiten in windows müssen hh:mm sein (24 Stunden), nicht „{}“–„{}“",
                w.start, w.end
            )));
        };
        if e <= s {
            return Err(invalid(format!(
                "Ein Zeitfenster muss nach seinem Beginn enden (über Mitternacht geht nicht): \
                 „{}“–„{}“",
                w.start, w.end
            )));
        }
        out.push(Window {
            weekday: code,
            start: s.format("%H:%M").to_string(),
            end: e.format("%H:%M").to_string(),
        });
    }
    Ok(out)
}

fn default_windows() -> Vec<Window> {
    WEEKDAYS[..5]
        .iter()
        .map(|(code, _)| Window {
            weekday: (*code).to_owned(),
            start: DEFAULT_WINDOW.0.to_owned(),
            end: DEFAULT_WINDOW.1.to_owned(),
        })
        .collect()
}

fn check_video_url(raw: &str) -> AppResult<Option<String>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let ok = raw.chars().count() <= MAX_VIDEO_URL_CHARS
        && url::Url::parse(raw).is_ok_and(|u| matches!(u.scheme(), "https" | "http"));
    if ok {
        Ok(Some(raw.to_owned()))
    } else {
        Err(invalid(format!(
            "video_url muss eine https-Adresse mit höchstens {MAX_VIDEO_URL_CHARS} Zeichen sein, \
             z. B. „https://meet.example.org/raum“"
        )))
    }
}

/// A time of a suggestion, cut to whole seconds: the booking page names
/// times in whole seconds, so a fraction would make the suggestion
/// unbookable.
fn parse_instant(name: &str, raw: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .map(|t| t.with_timezone(&Utc).trunc_subsecs(0))
        .map_err(|_| {
            invalid(format!(
                "{name} muss ein Zeitpunkt nach RFC 3339 mit Offset sein, z. B. \
                 „2026-11-03T14:00:00+01:00“, nicht „{raw}“"
            ))
        })
}

/// Checks the suggestions of a new link: only for one-off links, at most
/// [`MAX_SUGGESTIONS`], in the future, exactly the link's length, no time
/// twice. Returns them ordered.
fn check_suggestions(
    list: Vec<SuggestionInput>,
    duration: Duration,
    now: DateTime<Utc>,
) -> AppResult<Vec<(DateTime<Utc>, DateTime<Utc>)>> {
    if list.len() > MAX_SUGGESTIONS {
        return Err(invalid(format!(
            "Ein Einmal-Link hat höchstens {MAX_SUGGESTIONS} Vorschläge"
        )));
    }
    let mut out = Vec::with_capacity(list.len());
    for s in list {
        let start = parse_instant("suggestions[].start", &s.start)?;
        let end = match &s.end {
            Some(raw) => parse_instant("suggestions[].end", raw)?,
            None => start + duration,
        };
        if end - start != duration {
            return Err(invalid(format!(
                "Ein Vorschlag dauert so lange wie der Link ({} Minuten): {} bis {}",
                duration.num_minutes(),
                s.start,
                s.end.as_deref().unwrap_or("")
            )));
        }
        if start <= now {
            return Err(invalid(format!(
                "Der Vorschlag {} liegt in der Vergangenheit",
                s.start
            )));
        }
        if start > now + Duration::days(MAX_HORIZON_DAYS) {
            return Err(invalid(format!(
                "Der Vorschlag {} liegt mehr als {MAX_HORIZON_DAYS} Tage in der Zukunft",
                s.start
            )));
        }
        out.push((start, end));
    }
    out.sort();
    if out.windows(2).any(|w| w[0] == w[1]) {
        return Err(invalid("Ein Vorschlag steht doppelt in suggestions"));
    }
    Ok(out)
}

// ----- rows → objects ------------------------------------------------------------------

fn windows_of(row: &OpeningRow) -> Vec<Window> {
    serde_json::from_str(&row.windows).unwrap_or_default()
}

fn suggestion_view(s: &SuggestionRow) -> Suggestion {
    let status = match (&s.event_id, s.placeholder_exists) {
        (Some(_), true) => "reserved",
        (Some(_), false) => "missing",
        (None, _) => "pending",
    };
    Suggestion {
        id: s.id.clone(),
        start: format_ms(s.start_utc),
        end: format_ms(s.end_utc),
        placeholder_event_id: s.event_id.clone(),
        placeholder_status: status,
    }
}

fn booking_view(b: &BookingRow) -> Booking {
    Booking {
        id: b.id.clone(),
        event_id: b.event_id.clone(),
        start: format_ms(b.start_utc),
        end: format_ms(b.end_utc),
        guest_name: b.guest_name.clone(),
        guest_email: b.guest_email.clone(),
        note: b.note.clone(),
        created_at: format_ms(b.created_at),
    }
}

/// The public address of a link.
pub fn public_url(base: &url::Url, token: &str) -> String {
    base.join(&format!("{PUBLIC_PATH}/{token}"))
        .map(|u| u.to_string())
        .unwrap_or_else(|_| format!("{base}{PUBLIC_PATH}/{token}"))
}

/// The API object of a stored opening; suggestions that have started by
/// `now` are left out (the cleanup removes them).
pub(crate) fn view(
    row: &OpeningRow,
    suggestions: &[SuggestionRow],
    bookings: &[BookingRow],
    base: &url::Url,
    now: i64,
) -> Opening {
    Opening {
        id: row.id.clone(),
        kind: row.kind,
        url: public_url(base, &row.token),
        title: row.title.clone(),
        duration_minutes: row.duration_minutes,
        calendar_id: row.calendar_id.clone(),
        time_zone: row.time_zone.clone(),
        windows: windows_of(row),
        buffer_before_minutes: row.buffer_before_minutes,
        buffer_after_minutes: row.buffer_after_minutes,
        min_notice_minutes: row.min_notice_minutes,
        horizon_days: row.horizon_days,
        video_url: row.video_url.clone(),
        suggestions_only: row.suggested,
        suggestions: suggestions
            .iter()
            .filter(|s| s.start_utc > now)
            .map(suggestion_view)
            .collect(),
        booked_at: row.booked_at.map(format_ms),
        booked_event_id: row.booked_event_id.clone(),
        bookings: bookings.iter().map(booking_view).collect(),
        created_at: format_ms(row.created_at),
        updated_at: format_ms(row.updated_at),
        token: row.token.clone(),
    }
}

/// The rules of a stored link for [`slots::bookable`]; `None` if its zone
/// is unknown to this server.
fn rules_of(row: &OpeningRow, suggestions: &[SuggestionRow]) -> Option<Rules> {
    let tz = Tz::from_str(&row.time_zone).ok()?;
    let windows = windows_of(row)
        .iter()
        .filter_map(|w| {
            Some(WeeklyWindow {
                weekday: weekday_of(&w.weekday)?,
                start: parse_clock(&w.start)?,
                end: parse_clock(&w.end)?,
            })
        })
        .collect();
    Some(Rules {
        duration: Duration::minutes(row.duration_minutes),
        tz,
        windows,
        buffer_before: Duration::minutes(row.buffer_before_minutes),
        buffer_after: Duration::minutes(row.buffer_after_minutes),
        min_notice: Duration::minutes(row.min_notice_minutes),
        horizon: Duration::days(row.horizon_days),
        suggestions: row.suggested.then(|| {
            suggestions
                .iter()
                .map(|s| (s.id.clone(), s.interval()))
                .collect()
        }),
        booked: row.booked_at.is_some(),
    })
}

/// Maps the outcome of a placeholder write that did not apply.
fn placeholder_error(outcome: WriteOutcome, at: &str) -> AppError {
    match outcome {
        WriteOutcome::Retry { message, .. } => AppError::ProviderUnavailable(format!(
            "Der Platzhalter für den Vorschlag {at} ließ sich nicht anlegen: {message} Der Link \
             wurde nicht angelegt."
        )),
        WriteOutcome::Rejected { code, message } => AppError::BadRequest {
            code,
            message: format!(
                "Der Platzhalter für den Vorschlag {at} ließ sich nicht anlegen: {message} Der \
                 Link wurde nicht angelegt."
            ),
        },
        other => AppError::Internal(anyhow::anyhow!(
            "unexpected outcome creating a placeholder: {}",
            other.status()
        )),
    }
}

impl App {
    fn opening_base(&self) -> url::Url {
        self.config().public_url.clone()
    }

    /// Checks the target calendar of a link (module docs).
    async fn check_target(&self, calendar_id: &str) -> AppResult<()> {
        let id = calendar_id.to_owned();
        let found = self
            .with_reader(move |conn| {
                let Some(calendar) = accounts_store::calendar(conn, &id)? else {
                    return Ok(None);
                };
                let account = accounts_store::account(conn, &calendar.account_id)?;
                Ok(Some((calendar, account.map(|a| a.kind))))
            })
            .await?;
        let Some((calendar, kind)) = found else {
            return Err(AppError::BadRequest {
                code: "calendar_not_found",
                message: format!(
                    "Den Kalender „{calendar_id}“ gibt es nicht. Gültige IDs liefert GET \
                     /api/calendars."
                ),
            });
        };
        let name = &calendar.name;
        let writable = calendar.writable
            && !matches!(kind, None | Some(AccountKind::Ics | AccountKind::Microsoft));
        if !writable {
            return Err(AppError::BadRequest {
                code: "calendar_read_only",
                message: format!(
                    "Der Kalender „{name}“ ist nur lesbar. Ein Buchungslink braucht einen \
                     Zielkalender, in den Lichtung schreiben kann."
                ),
            });
        }
        if !calendar.can_invite {
            return Err(AppError::BadRequest {
                code: "calendar_cannot_invite",
                message: format!(
                    "Der Kalender „{name}“ kann keine Einladungen verschicken. Zielkalender eines \
                     Buchungslinks kann nur ein Kalender mit Einladungen sein (can_invite)."
                ),
            });
        }
        Ok(())
    }

    async fn opening_rows(&self, id: &str) -> AppResult<(OpeningRow, Vec<SuggestionRow>)> {
        let id = id.to_owned();
        self.with_reader(move |conn| {
            let row = store::opening(conn, &id)?.ok_or_else(not_found)?;
            let suggestions = store::suggestions(conn, &id)?;
            Ok((row, suggestions))
        })
        .await
    }

    // ----- management (app) ------------------------------------------------------------

    /// All links, newest first.
    pub async fn list_openings(&self) -> AppResult<Vec<Opening>> {
        let base = self.opening_base();
        let now = Utc::now().timestamp_millis();
        self.with_reader(move |conn| {
            let mut out = Vec::new();
            for row in store::openings(conn)? {
                let suggestions = store::suggestions(conn, &row.id)?;
                let bookings = store::bookings(conn, &row.id, MAX_LISTED_BOOKINGS)?;
                out.push(view(&row, &suggestions, &bookings, &base, now));
            }
            Ok(out)
        })
        .await
    }

    pub async fn opening(&self, id: &str) -> AppResult<Opening> {
        let base = self.opening_base();
        let now = Utc::now().timestamp_millis();
        let id = id.to_owned();
        self.with_reader(move |conn| {
            let row = store::opening(conn, &id)?.ok_or_else(not_found)?;
            let suggestions = store::suggestions(conn, &id)?;
            let bookings = store::bookings(conn, &id, MAX_LISTED_BOOKINGS)?;
            Ok(view(&row, &suggestions, &bookings, &base, now))
        })
        .await
    }

    /// Creates a link; for suggestions also their placeholders (module
    /// docs).
    pub async fn create_opening(&self, input: NewOpening) -> AppResult<Opening> {
        let now = Utc::now();
        let duration_minutes = check_duration(input.duration_minutes)?;
        let title = match (&input.title, input.kind) {
            (Some(t), _) => check_title(t)?,
            (None, OpeningKind::Once) => format!("Termin ({duration_minutes} Minuten)"),
            (None, OpeningKind::Template) => {
                return Err(invalid("title fehlt (eine Vorlage braucht einen Titel)"))
            }
        };
        let time_zone = match &input.time_zone {
            Some(z) => check_zone(z)?,
            None => default_zone().name().to_owned(),
        };
        let windows = match input.windows {
            Some(w) => check_windows(w)?,
            None => default_windows(),
        };
        let duration = Duration::minutes(duration_minutes);
        let suggestions = match input.suggestions {
            Some(list) if !list.is_empty() => {
                if input.kind == OpeningKind::Template {
                    return Err(invalid(
                        "Vorschläge gibt es nur bei Einmal-Links (kind once), nicht bei Vorlagen",
                    ));
                }
                check_suggestions(list, duration, now)?
            }
            _ => Vec::new(),
        };
        let now_ms = now.timestamp_millis();
        let row = OpeningRow {
            id: new_id("opn"),
            kind: input.kind,
            token: crate::auth::random_secret(),
            title,
            duration_minutes,
            calendar_id: input.calendar_id.trim().to_owned(),
            time_zone,
            windows: serde_json::to_string(&windows).map_err(|e| AppError::Internal(e.into()))?,
            buffer_before_minutes: check_range(
                "buffer_before_minutes",
                input.buffer_before_minutes.unwrap_or(0),
                0,
                MAX_BUFFER_MINUTES,
            )?,
            buffer_after_minutes: check_range(
                "buffer_after_minutes",
                input.buffer_after_minutes.unwrap_or(0),
                0,
                MAX_BUFFER_MINUTES,
            )?,
            min_notice_minutes: check_range(
                "min_notice_minutes",
                input
                    .min_notice_minutes
                    .unwrap_or(DEFAULT_MIN_NOTICE_MINUTES),
                0,
                MAX_NOTICE_MINUTES,
            )?,
            horizon_days: check_range(
                "horizon_days",
                input.horizon_days.unwrap_or(DEFAULT_HORIZON_DAYS),
                1,
                MAX_HORIZON_DAYS,
            )?,
            video_url: match &input.video_url {
                Some(u) => check_video_url(u)?,
                None => None,
            },
            suggested: !suggestions.is_empty(),
            booked_at: None,
            booked_event_id: None,
            created_at: now_ms,
            updated_at: now_ms,
            seq: 0,
        };
        self.check_target(&row.calendar_id).await?;
        let planned: Vec<(String, DateTime<Utc>, DateTime<Utc>)> = suggestions
            .into_iter()
            .map(|(s, e)| (new_id("sug"), s, e))
            .collect();
        {
            let (row, planned) = (row.clone(), planned.clone());
            self.with_change_tx(move |tx| {
                store::save(tx, &row)?;
                for (id, s, e) in &planned {
                    store::insert_suggestion(
                        tx,
                        id,
                        &row.id,
                        s.timestamp_millis(),
                        e.timestamp_millis(),
                        now_ms,
                    )?;
                }
                Ok(())
            })
            .await?;
        }
        for (suggestion_id, start, end) in &planned {
            if let Err(e) = self
                .create_placeholder(&row, suggestion_id, *start, *end)
                .await
            {
                tracing::warn!(opening = %row.id, error = %e, "placeholder failed, link discarded");
                if let Err(cleanup) = self.delete_opening(&row.id).await {
                    tracing::error!(opening = %row.id, error = %cleanup, "discarding the link failed");
                }
                return Err(e);
            }
        }
        self.opening(&row.id).await
    }

    /// Creates the placeholder of one suggestion and stores it.
    async fn create_placeholder(
        &self,
        row: &OpeningRow,
        suggestion_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<()> {
        let tz = Tz::from_str(&row.time_zone).unwrap_or(Tz::UTC);
        let local = |t: DateTime<Utc>| {
            json!({
                "date_time": t.with_timezone(&tz).format("%Y-%m-%dT%H:%M:%S").to_string(),
                "time_zone": tz.name(),
            })
        };
        let mutation = json!({
            "op": "create_event",
            "calendar_id": row.calendar_id,
            "event": {
                "title": format!("Reserviert: {}", row.title),
                "notes": format!(
                    "Platzhalter für einen Vorschlag des Buchungslinks „{}“. Lichtung entfernt \
                     ihn, sobald der Link gebucht oder gelöscht wird oder die Zeit vorbei ist.",
                    row.title
                ),
                "start": local(start),
                "end": local(end),
                "availability": "busy",
            },
        });
        let at = format_ms(start.timestamp_millis());
        let event_id = match self.apply_mutation(&Origin::Server, &mutation).await? {
            WriteOutcome::Applied { events, .. } => events
                .first()
                .and_then(|e| e["id"].as_str())
                .map(str::to_owned)
                .ok_or_else(|| AppError::Internal(anyhow::anyhow!("placeholder without event")))?,
            other => return Err(placeholder_error(other, &at)),
        };
        let (opening_id, suggestion_id, stored) =
            (row.id.clone(), suggestion_id.to_owned(), event_id.clone());
        let kept = self
            .with_change_tx(move |tx| {
                let kept = store::set_placeholder(tx, &suggestion_id, &stored)?;
                if kept {
                    store::touch(tx, &opening_id, crate::app::now_ms())?;
                }
                Ok(kept)
            })
            .await?;
        if !kept {
            // The link went away meanwhile: remove the placeholder again.
            let (opening_id, event_id) = (row.id.clone(), event_id);
            self.with_writer(move |conn| {
                Ok(store::queue_removal(
                    conn,
                    &event_id,
                    &opening_id,
                    crate::app::now_ms(),
                )?)
            })
            .await?;
            self.remove_placeholders().await?;
        }
        Ok(())
    }

    /// Changes a link (`PATCH`). A one-off link with suggestions keeps its
    /// calendar and length (its placeholders are fixed).
    pub async fn update_opening(&self, id: &str, patch: OpeningPatch) -> AppResult<Opening> {
        let title = patch.title.as_deref().map(check_title).transpose()?;
        let duration = patch.duration_minutes.map(check_duration).transpose()?;
        let time_zone = patch.time_zone.as_deref().map(check_zone).transpose()?;
        let windows = patch.windows.map(check_windows).transpose()?;
        let buffer_before = patch
            .buffer_before_minutes
            .map(|v| check_range("buffer_before_minutes", v, 0, MAX_BUFFER_MINUTES))
            .transpose()?;
        let buffer_after = patch
            .buffer_after_minutes
            .map(|v| check_range("buffer_after_minutes", v, 0, MAX_BUFFER_MINUTES))
            .transpose()?;
        let notice = patch
            .min_notice_minutes
            .map(|v| check_range("min_notice_minutes", v, 0, MAX_NOTICE_MINUTES))
            .transpose()?;
        let horizon = patch
            .horizon_days
            .map(|v| check_range("horizon_days", v, 1, MAX_HORIZON_DAYS))
            .transpose()?;
        let video_url = match &patch.video_url {
            Some(Some(u)) => Some(check_video_url(u)?),
            Some(None) => Some(None),
            None => None,
        };
        let calendar_id = patch.calendar_id.as_deref().map(str::trim);
        let (current, _) = self.opening_rows(id).await?;
        let calendar_changes = calendar_id.is_some_and(|c| c != current.calendar_id);
        let duration_changes = duration.is_some_and(|d| d != current.duration_minutes);
        if current.suggested && (calendar_changes || duration_changes) {
            return Err(invalid(
                "Bei einem Einmal-Link mit Vorschlägen lassen sich Kalender und Dauer nicht mehr \
                 ändern (die Platzhalter liegen fest). Den Link löschen und neu anlegen.",
            ));
        }
        if let Some(c) = calendar_id.filter(|_| calendar_changes) {
            self.check_target(c).await?;
        }
        let calendar_id = calendar_id.map(str::to_owned);
        let windows = windows
            .map(|w| serde_json::to_string(&w))
            .transpose()
            .map_err(|e| AppError::Internal(e.into()))?;
        let opening_id = id.to_owned();
        self.with_change_tx(move |tx| {
            let mut row = store::opening(tx, &opening_id)?.ok_or_else(not_found)?;
            let before = row.clone();
            if let Some(v) = title {
                row.title = v;
            }
            if let Some(v) = duration {
                row.duration_minutes = v;
            }
            if let Some(v) = calendar_id {
                row.calendar_id = v;
            }
            if let Some(v) = time_zone {
                row.time_zone = v;
            }
            if let Some(v) = windows {
                row.windows = v;
            }
            if let Some(v) = buffer_before {
                row.buffer_before_minutes = v;
            }
            if let Some(v) = buffer_after {
                row.buffer_after_minutes = v;
            }
            if let Some(v) = notice {
                row.min_notice_minutes = v;
            }
            if let Some(v) = horizon {
                row.horizon_days = v;
            }
            if let Some(v) = video_url {
                row.video_url = v;
            }
            if row != before {
                row.updated_at = crate::app::now_ms();
                store::save(tx, &row)?;
            }
            Ok(())
        })
        .await?;
        self.opening(id).await
    }

    /// Deletes a link and removes its placeholders. Removals the provider
    /// refuses for the moment stay queued for the next cleanup; the link is
    /// gone either way.
    pub async fn delete_opening(&self, id: &str) -> AppResult<()> {
        let id = id.to_owned();
        self.with_change_tx(move |tx| {
            let now = crate::app::now_ms();
            store::queue_placeholders(tx, &id, None, now)?;
            if store::delete(tx, &id, now)? {
                Ok(())
            } else {
                Err(not_found())
            }
        })
        .await?;
        self.remove_placeholders_logged().await;
        Ok(())
    }

    // ----- booking page (LIC-33) ---------------------------------------------------------

    /// The link behind a public token (`None` if unknown).
    pub async fn opening_by_token(&self, token: &str) -> AppResult<Option<Opening>> {
        let token = token.to_owned();
        let base = self.opening_base();
        let now = Utc::now().timestamp_millis();
        self.with_reader(move |conn| {
            let Some(row) = store::opening_by_token(conn, &token)? else {
                return Ok(None);
            };
            let suggestions = store::suggestions(conn, &row.id)?;
            // Earlier guests are none of the booking page's business.
            Ok(Some(view(&row, &suggestions, &[], &base, now)))
        })
        .await
    }

    /// The bookable slots of a link starting in `[from, to)`, by the
    /// current stored state and `now` (module docs, [`slots::bookable`]).
    /// Empty for a booked link and when the target calendar can no longer
    /// take bookings.
    pub async fn opening_slots(
        &self,
        opening_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Slot>> {
        let (row, suggestions) = self.opening_rows(opening_id).await?;
        let Some(rules) = rules_of(&row, &suggestions) else {
            tracing::warn!(opening = %row.id, zone = %row.time_zone, "opening with unknown zone");
            return Ok(Vec::new());
        };
        if rules.booked {
            return Ok(Vec::new());
        }
        if let Err(e) = self.check_target(&row.calendar_id).await {
            tracing::warn!(opening = %row.id, error = %e, "target calendar takes no bookings");
            return Ok(Vec::new());
        }
        // Only the part of the range that can hold slots.
        let from = from.max(now);
        let to = match &rules.suggestions {
            Some(list) => list
                .iter()
                .map(|(_, (s, _))| *s + Duration::milliseconds(1))
                .max()
                .map_or(from, |last| to.min(last)),
            None => to.min(now + rules.horizon),
        };
        if to <= from {
            return Ok(Vec::new());
        }
        let calendar_ids: Vec<String> = self
            .list_calendars()
            .await?
            .into_iter()
            // The target calendar always counts for its own link, so a
            // booking blocks its time even there.
            .filter(|c| c.openings_busy || c.id == row.calendar_id)
            .map(|c| c.id)
            .collect();
        let busy = self
            .busy_intervals(
                &calendar_ids,
                from - rules.buffer_before - Duration::minutes(1),
                to + rules.duration + rules.buffer_after,
                rules.tz,
                Some(&row.id),
            )
            .await?;
        if busy.truncated {
            return Err(AppError::Unavailable(
                "Im Zeitraum liegen zu viele Termine, um freie Zeiten sicher zu bestimmen".into(),
            ));
        }
        Ok(bookable(&rules, busy.intervals, from, to, now))
    }

    /// Checks one slot again right before booking: `Some` if `[start, end)`
    /// is still bookable. Hold [`App::opening_booking_guard`] from this
    /// check until [`App::mark_opening_booked`].
    pub async fn check_opening_slot(
        &self,
        opening_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<Option<Slot>> {
        let slots = self
            .opening_slots(opening_id, start, start + Duration::milliseconds(1), now)
            .await?;
        Ok(slots.into_iter().find(|s| s.start == start && s.end == end))
    }

    /// Records a booking (the booking page created `event_id`): a one-off
    /// link is used up and all its placeholders are removed (the booked
    /// suggestion's too). A template stays as it is. `409 conflict` if the
    /// one-off link was booked already. [`App::mark_opening_booked`], then
    /// [`App::remove_booked_placeholders`].
    pub async fn complete_opening_booking(
        &self,
        opening_id: &str,
        event_id: &str,
        booking: Option<(&Slot, &Guest)>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        if self
            .mark_opening_booked(opening_id, event_id, booking, now)
            .await?
        {
            self.remove_booked_placeholders().await;
        }
        Ok(())
    }

    /// The bookkeeping of a booking, quick and local: with `booking` (the
    /// booked time and what the guest entered) a row in
    /// `opening_bookings`, which only the owner sees; a one-off link is
    /// marked booked (only while it is not, else `409 conflict` and nothing
    /// is stored) and its placeholders are queued for removal. `true` if
    /// placeholders were queued: then call
    /// [`App::remove_booked_placeholders`], after releasing
    /// [`App::opening_booking_guard`] (the removal talks to the provider).
    /// A template stays bookable (`false`); it only gets the booking.
    pub async fn mark_opening_booked(
        &self,
        opening_id: &str,
        event_id: &str,
        booking: Option<(&Slot, &Guest)>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let (id, event_id) = (opening_id.to_owned(), event_id.to_owned());
        let at = now.timestamp_millis();
        let record = booking.map(|(slot, guest)| BookingRow {
            id: new_id("bkg"),
            opening_id: id.clone(),
            event_id: event_id.clone(),
            start_utc: slot.start.timestamp_millis(),
            end_utc: slot.end.timestamp_millis(),
            guest_name: guest.name.clone(),
            guest_email: guest.email.clone(),
            note: guest.note.clone(),
            created_at: at,
        });
        self.with_change_tx(move |tx| {
            let row = store::opening(tx, &id)?.ok_or_else(not_found)?;
            let queued = if row.kind == OpeningKind::Template {
                false
            } else {
                if !store::mark_booked(tx, &id, &event_id, at)? {
                    return Err(AppError::Conflict("Dieser Link ist bereits gebucht".into()));
                }
                store::queue_placeholders(tx, &id, None, at)?;
                true
            };
            if let Some(record) = &record {
                store::insert_booking(tx, record)?;
                if row.kind == OpeningKind::Template {
                    // A new `seq`: the pull shows the booking.
                    store::touch(tx, &id, at)?;
                }
            }
            Ok(queued)
        })
        .await
    }

    /// Removes the placeholders queued by [`App::mark_opening_booked`] at
    /// the provider; what fails is logged and left for the next cleanup.
    pub async fn remove_booked_placeholders(&self) {
        self.remove_placeholders_logged().await;
    }

    // ----- placeholders ---------------------------------------------------------------

    /// Deletes the queued placeholders at the provider (one at a time). A
    /// placeholder already gone counts as removed; a conflict (changed at
    /// the provider meanwhile: the provider wins and the conflict is
    /// visible) or a lasting refusal ends the attempts; an unreachable
    /// provider leaves it queued.
    pub async fn remove_placeholders(&self) -> AppResult<RemovalReport> {
        let _guard = self.placeholder_guard().await;
        let queue = self.with_reader(|conn| Ok(store::removals(conn)?)).await?;
        let mut report = RemovalReport::default();
        for item in queue {
            let event_id = item.event_id.clone();
            let event = self
                .with_reader(move |conn| Ok(events_store::event(conn, &event_id)?))
                .await?;
            let done = match event {
                None => true,
                Some(ev) => {
                    let mutation = json!({"op": "delete_event", "event_id": ev.id,
                                          "base_version": ev.version, "scope": "all"});
                    match self.apply_mutation(&Origin::Server, &mutation).await {
                        Ok(WriteOutcome::Applied { .. }) => true,
                        Ok(WriteOutcome::Conflict { conflict, .. }) => {
                            tracing::warn!(event = %item.event_id, conflict = %conflict.id,
                                "placeholder changed at the provider, kept");
                            true
                        }
                        Ok(WriteOutcome::Rejected { code, message }) => {
                            tracing::warn!(event = %item.event_id, code, reason = %message,
                                "placeholder cannot be removed");
                            true
                        }
                        Ok(WriteOutcome::Retry { message, .. }) => {
                            self.note_removal_failure(&item.event_id, &message).await?;
                            false
                        }
                        Err(e) => {
                            self.note_removal_failure(&item.event_id, &e.to_string())
                                .await?;
                            false
                        }
                    }
                }
            };
            if done {
                let event_id = item.event_id.clone();
                self.with_writer(move |conn| Ok(store::removal_done(conn, &event_id)?))
                    .await?;
                report.removed += 1;
            } else {
                report.pending += 1;
            }
        }
        if report.pending > 0 {
            tracing::warn!(
                pending = report.pending,
                "placeholders left for the next cleanup"
            );
        }
        Ok(report)
    }

    async fn note_removal_failure(&self, event_id: &str, message: &str) -> AppResult<()> {
        let (event_id, message) = (event_id.to_owned(), message.to_owned());
        self.with_writer(move |conn| Ok(store::removal_failed(conn, &event_id, &message)?))
            .await
    }

    async fn remove_placeholders_logged(&self) {
        if let Err(e) = self.remove_placeholders().await {
            tracing::error!(error = %e, "removing placeholders failed");
        }
    }

    /// The cleanup round: suggestions that have started by `now` drop out
    /// (their placeholders are queued), bookings made and ended more than
    /// [`BOOKING_RETENTION_DAYS`] ago are deleted, then the queue is worked
    /// off.
    pub async fn clean_up_openings(&self, now: DateTime<Utc>) -> AppResult<RemovalReport> {
        let at = now.timestamp_millis();
        let cutoff = (now - Duration::days(BOOKING_RETENTION_DAYS)).timestamp_millis();
        let (expired, pruned) = self
            .with_change_tx(move |tx| {
                let ids = store::openings_with_expired_suggestions(tx, at)?;
                let mut n = 0;
                for id in &ids {
                    n += store::queue_placeholders(tx, id, Some(at), at)?;
                    store::touch(tx, id, crate::app::now_ms())?;
                }
                let pruned = store::prune_bookings(tx, cutoff)?;
                for id in &pruned {
                    store::touch(tx, id, crate::app::now_ms())?;
                }
                Ok((n, pruned.len()))
            })
            .await?;
        if expired > 0 {
            tracing::info!(expired, "past suggestions removed");
        }
        if pruned > 0 {
            tracing::info!(openings = pruned, "old bookings deleted");
        }
        self.remove_placeholders().await
    }
}

fn not_found() -> AppError {
    AppError::NotFound("Buchungslink nicht gefunden".into())
}

/// Runs the cleanup at start and every [`CLEANUP_INTERVAL`] until the app
/// shuts down.
pub async fn run_cleanup(app: App) {
    let shutdown = app.shutdown_token();
    loop {
        if let Err(e) = app.clean_up_openings(Utc::now()).await {
            tracing::error!(error = %e, "openings cleanup failed");
        }
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(CLEANUP_INTERVAL) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn windows_are_checked_and_normalized() {
        let w = |d: &str, s: &str, e: &str| Window {
            weekday: d.into(),
            start: s.into(),
            end: e.into(),
        };
        assert_eq!(
            check_windows(vec![w("MO", "9:00", "17:30")]).unwrap(),
            vec![w("mo", "09:00", "17:30")]
        );
        for bad in [
            vec![],
            vec![w("xx", "09:00", "10:00")],
            vec![w("mo", "10:00", "09:00")],
            vec![w("mo", "09:00", "24:00")],
            vec![w("mo", "9 Uhr", "10:00")],
        ] {
            assert!(check_windows(bad.clone()).is_err(), "{bad:?}");
        }
        assert_eq!(default_windows().len(), 5);
    }

    #[test]
    fn suggestions_are_checked() {
        let now = t("2026-11-02T12:00:00Z");
        let half = Duration::minutes(30);
        let s = |start: &str, end: Option<&str>| SuggestionInput {
            start: start.into(),
            end: end.map(str::to_owned),
        };
        let ok = check_suggestions(
            vec![
                s("2026-11-04T10:00:00+01:00", None),
                s(
                    "2026-11-03T10:00:00+01:00",
                    Some("2026-11-03T10:30:00+01:00"),
                ),
            ],
            half,
            now,
        )
        .unwrap();
        assert_eq!(ok[0].0, t("2026-11-03T09:00:00Z"), "ordered");
        assert_eq!(ok[1].1, t("2026-11-04T09:30:00Z"), "end from the length");
        let cut = check_suggestions(
            vec![
                s("2026-11-05T10:00:00.750+01:00", None),
                s("2026-11-06T10:00:00.250Z", Some("2026-11-06T10:30:00.999Z")),
            ],
            half,
            now,
        )
        .unwrap();
        assert_eq!(
            cut,
            vec![
                (t("2026-11-05T09:00:00Z"), t("2026-11-05T09:30:00Z")),
                (t("2026-11-06T10:00:00Z"), t("2026-11-06T10:30:00Z")),
            ],
            "fractions of a second are cut"
        );
        for bad in [
            vec![s("2026-11-03T10:00:00", None)],
            vec![s(
                "2026-11-03T10:00:00+01:00",
                Some("2026-11-03T11:00:00+01:00"),
            )],
            vec![s("2026-11-01T10:00:00+01:00", None)],
            vec![
                s("2026-11-03T10:00:00+01:00", None),
                s("2026-11-03T09:00:00Z", None),
            ],
        ] {
            assert!(check_suggestions(bad, half, now).is_err());
        }
        let many = (0..=MAX_SUGGESTIONS)
            .map(|i| s(&format!("2026-11-{:02}T10:00:00Z", i + 3), None))
            .collect();
        assert!(check_suggestions(many, half, now).is_err());
    }

    #[test]
    fn public_urls() {
        let base = url::Url::parse("https://kalender.example.org").unwrap();
        assert_eq!(
            public_url(&base, "abc"),
            "https://kalender.example.org/o/abc"
        );
        let token = crate::auth::random_secret();
        assert_eq!(token.len(), 43, "256 bits, base64url");
    }

    #[test]
    fn other_fields_are_checked() {
        assert!(check_video_url("https://meet.example.org/raum")
            .unwrap()
            .is_some());
        assert_eq!(check_video_url(" ").unwrap(), None);
        assert!(check_video_url("javascript:alert(1)").is_err());
        assert!(check_zone("Mars/Olympus").is_err());
        assert_eq!(check_zone("utc").unwrap(), "UTC");
        assert!(check_duration(4).is_err() && check_duration(721).is_err());
        assert!(check_title("  ").is_err());
        assert_eq!(check_title(" Kennenlernen ").unwrap(), "Kennenlernen");
    }
}
