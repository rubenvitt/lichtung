//! Persistence of openings, their suggestions, the queue of placeholders
//! to remove (schema version 7) and their bookings (schema version 8,
//! deleted with the opening). Every change to an opening
//! takes a fresh `seq` ([`crate::changes`]); a deletion leaves a tombstone.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::changes::{self, ObjectType};
use crate::free::Interval;

/// The two kinds of links.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpeningKind {
    /// One-off link, used up by one booking.
    Once,
    /// Fixed link, bookable again and again.
    Template,
}

impl OpeningKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OpeningKind::Once => "once",
            OpeningKind::Template => "template",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "once" => Some(OpeningKind::Once),
            "template" => Some(OpeningKind::Template),
            _ => None,
        }
    }
}

/// One row of `openings`. `windows` is the stored JSON list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningRow {
    pub id: String,
    pub kind: OpeningKind,
    pub token: String,
    pub title: String,
    pub duration_minutes: i64,
    pub calendar_id: String,
    pub time_zone: String,
    pub windows: String,
    pub buffer_before_minutes: i64,
    pub buffer_after_minutes: i64,
    pub min_notice_minutes: i64,
    pub horizon_days: i64,
    pub video_url: Option<String>,
    pub suggested: bool,
    pub booked_at: Option<i64>,
    pub booked_event_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub seq: i64,
}

const OPENING_COLUMNS: &str = "id, kind, token, title, duration_minutes, calendar_id, time_zone, \
     windows, buffer_before_minutes, buffer_after_minutes, min_notice_minutes, horizon_days, \
     video_url, suggested, booked_at, booked_event_id, created_at, updated_at, seq";

fn row_to_opening(row: &Row<'_>) -> rusqlite::Result<OpeningRow> {
    let kind: String = row.get(1)?;
    Ok(OpeningRow {
        id: row.get(0)?,
        kind: OpeningKind::parse(&kind).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                format!("unknown opening kind {kind}").into(),
            )
        })?,
        token: row.get(2)?,
        title: row.get(3)?,
        duration_minutes: row.get(4)?,
        calendar_id: row.get(5)?,
        time_zone: row.get(6)?,
        windows: row.get(7)?,
        buffer_before_minutes: row.get(8)?,
        buffer_after_minutes: row.get(9)?,
        min_notice_minutes: row.get(10)?,
        horizon_days: row.get(11)?,
        video_url: row.get(12)?,
        suggested: row.get(13)?,
        booked_at: row.get(14)?,
        booked_event_id: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
        seq: row.get(18)?,
    })
}

/// Inserts or replaces an opening with a fresh `seq`; returns it.
pub fn save(conn: &Connection, o: &OpeningRow) -> rusqlite::Result<i64> {
    let seq = changes::bump(conn)?;
    conn.execute(
        "INSERT INTO openings (id, kind, token, title, duration_minutes, calendar_id, time_zone,
             windows, buffer_before_minutes, buffer_after_minutes, min_notice_minutes,
             horizon_days, video_url, suggested, booked_at, booked_event_id, created_at,
             updated_at, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
         ON CONFLICT (id) DO UPDATE SET title = excluded.title,
             duration_minutes = excluded.duration_minutes, calendar_id = excluded.calendar_id,
             time_zone = excluded.time_zone, windows = excluded.windows,
             buffer_before_minutes = excluded.buffer_before_minutes,
             buffer_after_minutes = excluded.buffer_after_minutes,
             min_notice_minutes = excluded.min_notice_minutes,
             horizon_days = excluded.horizon_days, video_url = excluded.video_url,
             booked_at = excluded.booked_at, booked_event_id = excluded.booked_event_id,
             updated_at = excluded.updated_at, seq = excluded.seq",
        params![
            o.id,
            o.kind.as_str(),
            o.token,
            o.title,
            o.duration_minutes,
            o.calendar_id,
            o.time_zone,
            o.windows,
            o.buffer_before_minutes,
            o.buffer_after_minutes,
            o.min_notice_minutes,
            o.horizon_days,
            o.video_url,
            o.suggested,
            o.booked_at,
            o.booked_event_id,
            o.created_at,
            o.updated_at,
            seq
        ],
    )?;
    Ok(seq)
}

/// Marks an opening as changed (its suggestions changed).
pub fn touch(conn: &Connection, id: &str, now: i64) -> rusqlite::Result<()> {
    let seq = changes::bump(conn)?;
    conn.execute(
        "UPDATE openings SET updated_at = ?2, seq = ?3 WHERE id = ?1",
        params![id, now, seq],
    )?;
    Ok(())
}

pub fn opening(conn: &Connection, id: &str) -> rusqlite::Result<Option<OpeningRow>> {
    conn.query_row(
        &format!("SELECT {OPENING_COLUMNS} FROM openings WHERE id = ?1"),
        [id],
        row_to_opening,
    )
    .optional()
}

pub fn opening_by_token(conn: &Connection, token: &str) -> rusqlite::Result<Option<OpeningRow>> {
    conn.query_row(
        &format!("SELECT {OPENING_COLUMNS} FROM openings WHERE token = ?1"),
        [token],
        row_to_opening,
    )
    .optional()
}

/// All openings, newest first.
pub fn openings(conn: &Connection) -> rusqlite::Result<Vec<OpeningRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {OPENING_COLUMNS} FROM openings ORDER BY created_at DESC, id DESC"
    ))?;
    let rows = stmt.query_map([], row_to_opening)?;
    rows.collect()
}

/// Deletes an opening (its suggestions go with it) and leaves a tombstone.
/// Queue its placeholders first ([`queue_placeholders`]).
pub fn delete(conn: &Connection, id: &str, now: i64) -> rusqlite::Result<bool> {
    let n = conn.execute("DELETE FROM openings WHERE id = ?1", [id])?;
    if n > 0 {
        changes::tombstone(conn, ObjectType::Opening, id, now)?;
    }
    Ok(n > 0)
}

/// Marks a one-off link as booked; `false` if it already was.
pub fn mark_booked(
    conn: &Connection,
    id: &str,
    event_id: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    let seq = changes::bump(conn)?;
    let n = conn.execute(
        "UPDATE openings SET booked_at = ?2, booked_event_id = ?3, updated_at = ?2, seq = ?4
         WHERE id = ?1 AND booked_at IS NULL",
        params![id, now, event_id, seq],
    )?;
    Ok(n > 0)
}

// ----- suggestions --------------------------------------------------------------------

/// A suggestion with the state of its placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionRow {
    pub id: String,
    pub opening_id: String,
    pub start_utc: i64,
    pub end_utc: i64,
    pub event_id: Option<String>,
    /// The placeholder event is stored (it was not deleted at the
    /// provider).
    pub placeholder_exists: bool,
}

impl SuggestionRow {
    pub fn interval(&self) -> Interval {
        (ms_to_utc(self.start_utc), ms_to_utc(self.end_utc))
    }
}

pub fn ms_to_utc(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

pub fn insert_suggestion(
    conn: &Connection,
    id: &str,
    opening_id: &str,
    start_utc: i64,
    end_utc: i64,
    now: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO opening_suggestions (id, opening_id, start_utc, end_utc, event_id, created_at)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
        params![id, opening_id, start_utc, end_utc, now],
    )?;
    Ok(())
}

/// Stores the placeholder of a suggestion; `false` if the suggestion is
/// gone (the link was deleted meanwhile).
pub fn set_placeholder(
    conn: &Connection,
    suggestion_id: &str,
    event_id: &str,
) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE opening_suggestions SET event_id = ?2 WHERE id = ?1",
        params![suggestion_id, event_id],
    )?;
    Ok(n > 0)
}

/// The suggestions of an opening, earliest first.
pub fn suggestions(conn: &Connection, opening_id: &str) -> rusqlite::Result<Vec<SuggestionRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.id, s.opening_id, s.start_utc, s.end_utc, s.event_id, e.id IS NOT NULL
         FROM opening_suggestions s LEFT JOIN events e ON e.id = s.event_id
         WHERE s.opening_id = ?1 ORDER BY s.start_utc, s.id",
    )?;
    let rows = stmt.query_map([opening_id], |r| {
        Ok(SuggestionRow {
            id: r.get(0)?,
            opening_id: r.get(1)?,
            start_utc: r.get(2)?,
            end_utc: r.get(3)?,
            event_id: r.get(4)?,
            placeholder_exists: r.get(5)?,
        })
    })?;
    rows.collect()
}

/// For the free-time rules: the placeholder event IDs of `own` (they do
/// not count for their own link) and the times of all other links'
/// suggestions that touch `[from, to)` (they count everywhere).
pub fn reservations(
    conn: &Connection,
    own: Option<&str>,
    from: i64,
    to: i64,
) -> rusqlite::Result<(HashSet<String>, Vec<Interval>)> {
    let mut stmt = conn.prepare_cached(
        "SELECT opening_id, start_utc, end_utc, event_id FROM opening_suggestions
         WHERE opening_id = ?1 OR (start_utc < ?3 AND end_utc > ?2)",
    )?;
    let own_id = own.unwrap_or("");
    let rows = stmt.query_map(params![own_id, from, to], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut mine = HashSet::new();
    let mut others = Vec::new();
    for row in rows {
        let (opening_id, start, end, event_id) = row?;
        if own == Some(opening_id.as_str()) {
            mine.extend(event_id);
        } else {
            others.push((ms_to_utc(start), ms_to_utc(end)));
        }
    }
    Ok((mine, others))
}

/// Moves the placeholders of the suggestions of `opening_id` (only those
/// starting at or before `started_by`, all if `None`) into the removal
/// queue and deletes those suggestions. Returns how many suggestions went.
pub fn queue_placeholders(
    conn: &Connection,
    opening_id: &str,
    started_by: Option<i64>,
    now: i64,
) -> rusqlite::Result<usize> {
    let limit = started_by.unwrap_or(i64::MAX);
    conn.execute(
        "INSERT OR IGNORE INTO placeholder_removals (event_id, opening_id, created_at)
         SELECT event_id, opening_id, ?3 FROM opening_suggestions
         WHERE opening_id = ?1 AND start_utc <= ?2 AND event_id IS NOT NULL",
        params![opening_id, limit, now],
    )?;
    conn.execute(
        "DELETE FROM opening_suggestions WHERE opening_id = ?1 AND start_utc <= ?2",
        params![opening_id, limit],
    )
}

/// Queues one placeholder for removal.
pub fn queue_removal(
    conn: &Connection,
    event_id: &str,
    opening_id: &str,
    now: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO placeholder_removals (event_id, opening_id, created_at)
         VALUES (?1, ?2, ?3)",
        params![event_id, opening_id, now],
    )?;
    Ok(())
}

/// The openings that have suggestions starting at or before `now`.
pub fn openings_with_expired_suggestions(
    conn: &Connection,
    now: i64,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT opening_id FROM opening_suggestions WHERE start_utc <= ?1 ORDER BY opening_id",
    )?;
    let rows = stmt.query_map([now], |r| r.get(0))?;
    rows.collect()
}

// ----- placeholder attempts (schema version 9) ----------------------------------------

/// Notes the UID of a placeholder before it is written at the provider.
pub fn begin_attempt(
    conn: &Connection,
    uid: &str,
    calendar_id: &str,
    opening_id: &str,
    suggestion_id: &str,
    now: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO placeholder_attempts
             (uid, calendar_id, opening_id, suggestion_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![uid, calendar_id, opening_id, suggestion_id, now],
    )?;
    Ok(())
}

/// Forgets an attempt: its placeholder is stored at its suggestion or
/// queued for removal.
pub fn end_attempt(conn: &Connection, uid: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM placeholder_attempts WHERE uid = ?1", [uid])?;
    Ok(())
}

/// Attempts whose suggestion is gone: the placeholder the provider may
/// have written anyway is queued for removal if it is stored (found by
/// calendar and UID), and the attempt is forgotten. Attempts whose
/// placeholder is not stored wait for the next sync, and are forgotten
/// when made before `give_up_before`. Returns how many were queued.
pub fn queue_failed_attempts(
    conn: &Connection,
    now: i64,
    give_up_before: i64,
) -> rusqlite::Result<usize> {
    let orphaned = "NOT EXISTS (SELECT 1 FROM opening_suggestions s WHERE s.id = a.suggestion_id)";
    let queued = conn.execute(
        &format!(
            "INSERT OR IGNORE INTO placeholder_removals (event_id, opening_id, created_at)
             SELECT e.id, a.opening_id, ?1
             FROM placeholder_attempts a
             JOIN events e ON e.calendar_id = a.calendar_id AND e.uid = a.uid
             WHERE {orphaned}"
        ),
        [now],
    )?;
    conn.execute(
        &format!(
            "DELETE FROM placeholder_attempts AS a WHERE {orphaned} AND (a.created_at < ?1
                 OR EXISTS (SELECT 1 FROM events e
                            WHERE e.calendar_id = a.calendar_id AND e.uid = a.uid))"
        ),
        [give_up_before],
    )?;
    Ok(queued)
}

// ----- removal queue --------------------------------------------------------------------

/// A placeholder still to be deleted at the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalRow {
    pub event_id: String,
    pub opening_id: String,
    pub attempts: i64,
}

pub fn removals(conn: &Connection) -> rusqlite::Result<Vec<RemovalRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT event_id, opening_id, attempts FROM placeholder_removals ORDER BY created_at, event_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(RemovalRow {
            event_id: r.get(0)?,
            opening_id: r.get(1)?,
            attempts: r.get(2)?,
        })
    })?;
    rows.collect()
}

pub fn removal_done(conn: &Connection, event_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM placeholder_removals WHERE event_id = ?1",
        [event_id],
    )?;
    Ok(())
}

pub fn removal_failed(conn: &Connection, event_id: &str, error: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE placeholder_removals SET attempts = attempts + 1, last_error = ?2
         WHERE event_id = ?1",
        params![event_id, error],
    )?;
    Ok(())
}

// ----- bookings -------------------------------------------------------------------------

/// One booking of a link with what the guest entered (schema version 8).
/// Never part of the booked event: only the owner sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookingRow {
    pub id: String,
    pub opening_id: String,
    pub event_id: String,
    pub start_utc: i64,
    pub end_utc: i64,
    pub guest_name: String,
    pub guest_email: String,
    pub note: Option<String>,
    pub created_at: i64,
}

pub fn insert_booking(conn: &Connection, b: &BookingRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO opening_bookings (id, opening_id, event_id, start_utc, end_utc, guest_name,
             guest_email, note, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            b.id,
            b.opening_id,
            b.event_id,
            b.start_utc,
            b.end_utc,
            b.guest_name,
            b.guest_email,
            b.note,
            b.created_at
        ],
    )?;
    Ok(())
}

/// The newest `limit` bookings of an opening, newest first.
pub fn bookings(
    conn: &Connection,
    opening_id: &str,
    limit: usize,
) -> rusqlite::Result<Vec<BookingRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, opening_id, event_id, start_utc, end_utc, guest_name, guest_email, note,
             created_at
         FROM opening_bookings WHERE opening_id = ?1
         ORDER BY created_at DESC, id DESC LIMIT ?2",
    )?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map(params![opening_id, limit], |r| {
        Ok(BookingRow {
            id: r.get(0)?,
            opening_id: r.get(1)?,
            event_id: r.get(2)?,
            start_utc: r.get(3)?,
            end_utc: r.get(4)?,
            guest_name: r.get(5)?,
            guest_email: r.get(6)?,
            note: r.get(7)?,
            created_at: r.get(8)?,
        })
    })?;
    rows.collect()
}

/// Deletes the bookings made and ended before `cutoff`; returns the
/// openings that lost bookings.
pub fn prune_bookings(conn: &Connection, cutoff: i64) -> rusqlite::Result<Vec<String>> {
    let ids = {
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT opening_id FROM opening_bookings
             WHERE created_at < ?1 AND end_utc < ?1 ORDER BY opening_id",
        )?;
        let rows = stmt.query_map([cutoff], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()?
    };
    if !ids.is_empty() {
        conn.execute(
            "DELETE FROM opening_bookings WHERE created_at < ?1 AND end_utc < ?1",
            [cutoff],
        )?;
    }
    Ok(ids)
}
