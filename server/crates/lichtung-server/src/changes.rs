//! Change log for the sync API (pull with cursor, SSE; LIC-11).
//!
//! One global, strictly increasing counter (`change_counter.seq`). Whoever
//! changes a synced object (account, calendar, calendar set, later event and
//! conflict) takes a fresh value with [`bump`] and stores it in the object's
//! `seq` column, in the same transaction. Deletions leave a [`tombstone`].
//! A pull then reads all objects and tombstones with `seq > cursor`, ordered
//! by `seq`; each object appears once, in its newest state.
//!
//! After the transaction commits, [`crate::App`] publishes the new cursor on
//! a `tokio::sync::watch` channel ([`crate::App::subscribe_changes`]), which
//! the SSE endpoint will forward. Use [`crate::App`]'s change transactions
//! (`with_change_tx`) so the notification is never sent for a rolled-back
//! change.

use rusqlite::{params, Connection};

/// Types of synced objects (the `type` of a pull change).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Account,
    Calendar,
    CalendarSet,
    Event,
    Conflict,
}

impl ObjectType {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectType::Account => "account",
            ObjectType::Calendar => "calendar",
            ObjectType::CalendarSet => "calendar_set",
            ObjectType::Event => "event",
            ObjectType::Conflict => "conflict",
        }
    }
}

/// The current cursor (highest value handed out so far).
pub fn current(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT seq FROM change_counter WHERE id = 1", [], |r| {
        r.get(0)
    })
}

/// Takes the next value of the counter. Call inside the transaction that
/// makes the change and store the result in the object's `seq` column.
pub fn bump(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "UPDATE change_counter SET seq = seq + 1 WHERE id = 1 RETURNING seq",
        [],
        |r| r.get(0),
    )
}

/// Records the deletion of an object; returns its `seq`.
pub fn tombstone(conn: &Connection, kind: ObjectType, id: &str, now: i64) -> rusqlite::Result<i64> {
    let seq = bump(conn)?;
    conn.execute(
        "INSERT INTO tombstones (seq, type, id, deleted_at) VALUES (?1, ?2, ?3, ?4)",
        params![seq, kind.as_str(), id, now],
    )?;
    Ok(seq)
}

/// A recorded deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    pub seq: i64,
    pub kind: String,
    pub id: String,
    pub deleted_at: i64,
}

/// Tombstones newer than `since`, oldest first (building block for the pull).
pub fn tombstones_since(conn: &Connection, since: i64) -> rusqlite::Result<Vec<Tombstone>> {
    let mut stmt = conn.prepare_cached(
        "SELECT seq, type, id, deleted_at FROM tombstones WHERE seq > ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map([since], |r| {
        Ok(Tombstone {
            seq: r.get(0)?,
            kind: r.get(1)?,
            id: r.get(2)?,
            deleted_at: r.get(3)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_increases_strictly_and_tombstones_take_a_value() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&dir.path().join("lichtung.db")).unwrap();
        assert_eq!(current(&conn).unwrap(), 0);
        assert_eq!(bump(&conn).unwrap(), 1);
        assert_eq!(bump(&conn).unwrap(), 2);
        assert_eq!(
            tombstone(&conn, ObjectType::Calendar, "cal_x", 5).unwrap(),
            3
        );
        assert_eq!(current(&conn).unwrap(), 3);
        let t = tombstones_since(&conn, 2).unwrap();
        assert_eq!(
            t,
            vec![Tombstone {
                seq: 3,
                kind: "calendar".into(),
                id: "cal_x".into(),
                deleted_at: 5
            }]
        );
        assert!(tombstones_since(&conn, 3).unwrap().is_empty());
    }
}
