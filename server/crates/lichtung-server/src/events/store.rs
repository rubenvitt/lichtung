//! SQL for events and the per-calendar sync state. Every function that
//! changes an event takes a fresh `seq` from [`changes::bump`] (or leaves a
//! tombstone) itself, so callers cannot forget it.

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use super::{Derivation, Derived, EventContext};
use crate::accounts::store as accounts_store;
use crate::changes::{self, ObjectType};

/// A stored event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub id: String,
    pub calendar_id: String,
    /// Normalized absolute URL of the resource at the provider.
    pub href: String,
    pub etag: Option<String>,
    pub uid: Option<String>,
    /// The iCalendar original (CRLF line endings).
    pub ics: String,
    /// Why the original could not be read; `None` if it could.
    pub parse_error: Option<String>,
    pub version: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub seq: i64,
}

const EVENT_COLUMNS: &str =
    "id, calendar_id, href, etag, uid, ics, parse_error, version, created_at, updated_at, seq";

fn row_to_event(row: &Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        id: row.get(0)?,
        calendar_id: row.get(1)?,
        href: row.get(2)?,
        etag: row.get(3)?,
        uid: row.get(4)?,
        ics: row.get(5)?,
        parse_error: row.get(6)?,
        version: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        seq: row.get(10)?,
    })
}

pub fn new_event_id() -> String {
    format!("ev_{}", Uuid::now_v7().simple())
}

pub fn event(conn: &Connection, id: &str) -> rusqlite::Result<Option<EventRow>> {
    conn.query_row(
        &format!("SELECT {EVENT_COLUMNS} FROM events WHERE id = ?1"),
        [id],
        row_to_event,
    )
    .optional()
}

pub fn event_by_href(
    conn: &Connection,
    calendar_id: &str,
    href: &str,
) -> rusqlite::Result<Option<EventRow>> {
    conn.query_row(
        &format!("SELECT {EVENT_COLUMNS} FROM events WHERE calendar_id = ?1 AND href = ?2"),
        params![calendar_id, href],
        row_to_event,
    )
    .optional()
}

/// Events of a calendar by start (unreadable ones last).
pub fn events_of_calendar(conn: &Connection, calendar_id: &str) -> rusqlite::Result<Vec<EventRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {EVENT_COLUMNS} FROM events WHERE calendar_id = ?1
         ORDER BY start_utc IS NULL, start_utc, id"
    ))?;
    let rows = stmt.query_map([calendar_id], row_to_event)?;
    rows.collect()
}

/// `href → (id, etag)` of a calendar's events (what the sync compares).
pub fn etags_of_calendar(
    conn: &Connection,
    calendar_id: &str,
) -> rusqlite::Result<Vec<(String, String, Option<String>)>> {
    let mut stmt =
        conn.prepare_cached("SELECT href, id, etag FROM events WHERE calendar_id = ?1")?;
    let rows = stmt.query_map([calendar_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    rows.collect()
}

/// Readable events of the given calendars whose occurrences may overlap
/// `[from_ms, to_ms)`. The stored range is widened by a day on both sides
/// (floating times and dates were computed in the server zone).
pub fn candidates(
    conn: &Connection,
    calendar_ids: &[String],
    from_ms: i64,
    to_ms: i64,
) -> rusqlite::Result<Vec<EventRow>> {
    const DAY_MS: i64 = 86_400_000;
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {EVENT_COLUMNS} FROM events
         WHERE calendar_id = ?1 AND parse_error IS NULL
           AND start_utc < ?3 AND (range_end_utc IS NULL OR range_end_utc >= ?2)
         ORDER BY start_utc, id"
    ))?;
    let mut out = Vec::new();
    for calendar_id in calendar_ids {
        let rows = stmt.query_map(
            params![
                calendar_id,
                from_ms.saturating_sub(DAY_MS),
                to_ms.saturating_add(DAY_MS)
            ],
            row_to_event,
        )?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

/// What [`upsert`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upsert {
    /// New event (version 1).
    Inserted,
    /// The original changed: version + 1, fresh `seq`.
    Updated,
    /// Same original under a new ETag: only the ETag was stored, no new
    /// version and no change for the clients.
    EtagOnly,
    /// Same ETag and same original: nothing written.
    Unchanged,
}

/// Inserts or updates the event at `href` of a calendar with a (normalized)
/// original.
pub fn upsert(
    conn: &Connection,
    calendar_id: &str,
    href: &str,
    etag: Option<&str>,
    ics: &str,
    derived: &Derivation,
    now: i64,
) -> rusqlite::Result<(Upsert, String)> {
    let existing = event_by_href(conn, calendar_id, href)?;
    if let Some(row) = &existing {
        if row.ics == ics {
            if row.etag.as_deref() == etag {
                return Ok((Upsert::Unchanged, row.id.clone()));
            }
            conn.execute(
                "UPDATE events SET etag = ?2 WHERE id = ?1",
                params![row.id, etag],
            )?;
            return Ok((Upsert::EtagOnly, row.id.clone()));
        }
    }
    let empty = Derived::default();
    let (d, parse_error) = match derived {
        Ok(d) => (d, None),
        Err(e) => (&empty, Some(e.as_str())),
    };
    let uid = d.uid.clone().or_else(|| super::sniff_uid(ics));
    let seq = changes::bump(conn)?;
    match existing {
        Some(row) => {
            conn.execute(
                "UPDATE events SET etag = ?2, uid = ?3, ics = ?4, parse_error = ?5, title = ?6,
                     start_utc = ?7, range_end_utc = ?8, is_recurring = ?9, all_day = ?10,
                     status = ?11, transparency = ?12, version = version + 1,
                     updated_at = ?13, seq = ?14
                 WHERE id = ?1",
                params![
                    row.id,
                    etag,
                    uid,
                    ics,
                    parse_error,
                    d.title,
                    d.start_utc,
                    d.range_end_utc,
                    d.is_recurring,
                    d.all_day,
                    d.status,
                    d.transparency,
                    now,
                    seq
                ],
            )?;
            Ok((Upsert::Updated, row.id))
        }
        None => {
            let id = new_event_id();
            conn.execute(
                "INSERT INTO events (id, calendar_id, href, etag, uid, ics, parse_error, title,
                     start_utc, range_end_utc, is_recurring, all_day, status, transparency,
                     version, created_at, updated_at, seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 1, ?15,
                     ?15, ?16)",
                params![
                    id,
                    calendar_id,
                    href,
                    etag,
                    uid,
                    ics,
                    parse_error,
                    d.title,
                    d.start_utc,
                    d.range_end_utc,
                    d.is_recurring,
                    d.all_day,
                    d.status,
                    d.transparency,
                    now,
                    seq
                ],
            )?;
            Ok((Upsert::Inserted, id))
        }
    }
}

/// Deletes the event at `href` with a tombstone (its conflicts lose their
/// `event_id`); returns its ID if it existed.
pub fn delete_by_href(
    conn: &Connection,
    calendar_id: &str,
    href: &str,
    now: i64,
) -> rusqlite::Result<Option<String>> {
    let id: Option<String> = conn
        .query_row(
            "DELETE FROM events WHERE calendar_id = ?1 AND href = ?2 RETURNING id",
            params![calendar_id, href],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = &id {
        changes::tombstone(conn, ObjectType::Event, id, now)?;
        crate::conflicts::detach_event(conn, id)?;
    }
    Ok(id)
}

/// Deletes all events of a calendar, each with a tombstone.
pub fn delete_calendar_events(
    conn: &Connection,
    calendar_id: &str,
    now: i64,
) -> rusqlite::Result<usize> {
    let ids: Vec<String> = {
        let mut stmt = conn.prepare_cached("SELECT id FROM events WHERE calendar_id = ?1")?;
        let rows = stmt.query_map([calendar_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for id in &ids {
        conn.execute("DELETE FROM events WHERE id = ?1", [id])?;
        changes::tombstone(conn, ObjectType::Event, id, now)?;
        crate::conflicts::detach_event(conn, id)?;
    }
    Ok(ids.len())
}

// ----- sync state ---------------------------------------------------------------------

/// What the sync remembers per calendar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncState {
    pub sync_token: Option<String>,
    pub ctag: Option<String>,
    /// The server refused `sync-collection` before.
    pub sync_unsupported: bool,
}

pub fn sync_state(conn: &Connection, calendar_id: &str) -> rusqlite::Result<Option<SyncState>> {
    conn.query_row(
        "SELECT sync_token, ctag, sync_unsupported FROM calendars WHERE id = ?1",
        [calendar_id],
        |r| {
            Ok(SyncState {
                sync_token: r.get(0)?,
                ctag: r.get(1)?,
                sync_unsupported: r.get(2)?,
            })
        },
    )
    .optional()
}

/// Stores the sync state. Not a change of the calendar object (the sync
/// API does not show it), so no `seq`.
pub fn set_sync_state(
    conn: &Connection,
    calendar_id: &str,
    state: &SyncState,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE calendars SET sync_token = ?2, ctag = ?3, sync_unsupported = ?4 WHERE id = ?1",
        params![
            calendar_id,
            state.sync_token,
            state.ctag,
            state.sync_unsupported
        ],
    )?;
    Ok(())
}

// ----- context ------------------------------------------------------------------------

/// The JSON context of a calendar's events: the account's own addresses.
pub fn context_for_calendar(
    conn: &Connection,
    calendar_id: &str,
) -> rusqlite::Result<EventContext> {
    let account_id: Option<String> = conn
        .query_row(
            "SELECT account_id FROM calendars WHERE id = ?1",
            [calendar_id],
            |r| r.get(0),
        )
        .optional()?;
    let account = match account_id {
        Some(id) => accounts_store::account(conn, &id)?,
        None => None,
    };
    Ok(match account {
        Some(a) => EventContext::for_account(&a),
        None => EventContext::default(),
    })
}
