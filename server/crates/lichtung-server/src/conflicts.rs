//! Conflicts of the write path (sync API contract, section 3, `Conflict`):
//! an own change the server did not apply because the provider had a
//! different version by then. The provider wins; the dropped change stays
//! visible until someone dismisses it.
//!
//! Every conflict takes a `seq` (pull), dismissing leaves a tombstone. When
//! the event of a conflict is deleted, the conflict loses its `event_id`
//! and takes a fresh `seq` ([`detach_event`], called by the event store).

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::app::{format_ms, now_ms, App};
use crate::changes::{self, ObjectType};
use crate::error::{AppError, AppResult};

/// A stored conflict.
#[derive(Debug, Clone, PartialEq)]
pub struct ConflictRow {
    pub id: String,
    pub event_id: Option<String>,
    pub calendar_id: Option<String>,
    /// `app:<device>`, `web`, `api:<token>`, `mcp:<token>` or `server`.
    pub origin: String,
    /// German, for people.
    pub message: String,
    /// The dropped mutation as it was sent.
    pub mutation: Value,
    pub created_at: i64,
    pub seq: i64,
}

/// `Conflict` of the sync API.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conflict {
    pub id: String,
    pub event_id: Option<String>,
    pub calendar_id: Option<String>,
    pub origin: String,
    pub created_at: String,
    pub message: String,
    pub mutation: Value,
}

impl From<&ConflictRow> for Conflict {
    fn from(row: &ConflictRow) -> Self {
        Conflict {
            id: row.id.clone(),
            event_id: row.event_id.clone(),
            calendar_id: row.calendar_id.clone(),
            origin: row.origin.clone(),
            created_at: format_ms(row.created_at),
            message: row.message.clone(),
            mutation: row.mutation.clone(),
        }
    }
}

const COLUMNS: &str = "id, event_id, calendar_id, origin, message, mutation, created_at, seq";

fn row_to_conflict(row: &Row<'_>) -> rusqlite::Result<ConflictRow> {
    let mutation: String = row.get(5)?;
    Ok(ConflictRow {
        id: row.get(0)?,
        event_id: row.get(1)?,
        calendar_id: row.get(2)?,
        origin: row.get(3)?,
        message: row.get(4)?,
        mutation: serde_json::from_str(&mutation).unwrap_or(Value::Null),
        created_at: row.get(6)?,
        seq: row.get(7)?,
    })
}

pub fn new_conflict_id() -> String {
    format!("cf_{}", Uuid::now_v7().simple())
}

/// Stores a new conflict with a fresh `seq`.
pub fn insert(conn: &Connection, row: &ConflictRow) -> rusqlite::Result<i64> {
    let seq = changes::bump(conn)?;
    conn.execute(
        "INSERT INTO conflicts (id, event_id, calendar_id, origin, message, mutation, created_at, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            row.id,
            row.event_id,
            row.calendar_id,
            row.origin,
            row.message,
            row.mutation.to_string(),
            row.created_at,
            seq
        ],
    )?;
    Ok(seq)
}

pub fn conflict(conn: &Connection, id: &str) -> rusqlite::Result<Option<ConflictRow>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM conflicts WHERE id = ?1"),
        [id],
        row_to_conflict,
    )
    .optional()
}

/// All conflicts, newest first.
pub fn conflicts(conn: &Connection) -> rusqlite::Result<Vec<ConflictRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM conflicts ORDER BY created_at DESC, id DESC"
    ))?;
    let rows = stmt.query_map([], row_to_conflict)?;
    rows.collect()
}

/// The event of these conflicts is gone: `event_id` becomes NULL, each
/// conflict takes a fresh `seq` so clients learn about it.
pub fn detach_event(conn: &Connection, event_id: &str) -> rusqlite::Result<usize> {
    let ids: Vec<String> = {
        let mut stmt = conn.prepare_cached("SELECT id FROM conflicts WHERE event_id = ?1")?;
        let rows = stmt.query_map([event_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for id in &ids {
        let seq = changes::bump(conn)?;
        conn.execute(
            "UPDATE conflicts SET event_id = NULL, seq = ?2 WHERE id = ?1",
            params![id, seq],
        )?;
    }
    Ok(ids.len())
}

/// Deletes a conflict with a tombstone; returns whether it existed.
pub fn delete(conn: &Connection, id: &str, now: i64) -> rusqlite::Result<bool> {
    let n = conn.execute("DELETE FROM conflicts WHERE id = ?1", [id])?;
    if n > 0 {
        changes::tombstone(conn, ObjectType::Conflict, id, now)?;
    }
    Ok(n > 0)
}

impl App {
    /// All open conflicts, newest first.
    pub async fn list_conflicts(&self) -> AppResult<Vec<Conflict>> {
        self.with_reader(|conn| Ok(conflicts(conn)?.iter().map(Conflict::from).collect()))
            .await
    }

    pub async fn conflict(&self, id: &str) -> AppResult<Conflict> {
        let id = id.to_owned();
        self.with_reader(move |conn| {
            conflict(conn, &id)?
                .as_ref()
                .map(Conflict::from)
                .ok_or_else(|| AppError::NotFound("Konflikt nicht gefunden".into()))
        })
        .await
    }

    /// Dismisses a conflict (it appears as `delete` in the pull).
    pub async fn dismiss_conflict(&self, id: &str) -> AppResult<()> {
        let id = id.to_owned();
        self.with_change_tx(move |tx| {
            if delete(tx, &id, now_ms())? {
                Ok(())
            } else {
                Err(AppError::NotFound("Konflikt nicht gefunden".into()))
            }
        })
        .await
    }
}
