//! Persistence of blocker rules and blockers (schema version 11).

use rusqlite::{params, Connection, OptionalExtension, Row};

/// One row of `blocker_rules`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRow {
    pub id: String,
    pub source_calendar_id: String,
    pub target_calendar_id: String,
    pub title: String,
    pub all_day: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

const RULE_COLUMNS: &str =
    "id, source_calendar_id, target_calendar_id, title, all_day, created_at, updated_at";

fn row_to_rule(row: &Row<'_>) -> rusqlite::Result<RuleRow> {
    Ok(RuleRow {
        id: row.get(0)?,
        source_calendar_id: row.get(1)?,
        target_calendar_id: row.get(2)?,
        title: row.get(3)?,
        all_day: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

pub fn rules(conn: &Connection) -> rusqlite::Result<Vec<RuleRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {RULE_COLUMNS} FROM blocker_rules ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([], row_to_rule)?;
    rows.collect()
}

pub fn rule(conn: &Connection, id: &str) -> rusqlite::Result<Option<RuleRow>> {
    conn.query_row(
        &format!("SELECT {RULE_COLUMNS} FROM blocker_rules WHERE id = ?1"),
        [id],
        row_to_rule,
    )
    .optional()
}

/// The rule with this source and target, if any.
pub fn rule_for(conn: &Connection, source: &str, target: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM blocker_rules WHERE source_calendar_id = ?1 AND target_calendar_id = ?2",
        [source, target],
        |r| r.get(0),
    )
    .optional()
}

pub fn insert_rule(conn: &Connection, r: &RuleRow) -> rusqlite::Result<()> {
    conn.execute(
        &format!("INSERT INTO blocker_rules ({RULE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
        params![
            r.id,
            r.source_calendar_id,
            r.target_calendar_id,
            r.title,
            r.all_day,
            r.created_at,
            r.updated_at
        ],
    )?;
    Ok(())
}

pub fn update_rule(conn: &Connection, r: &RuleRow) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE blocker_rules SET title = ?2, all_day = ?3, updated_at = ?4 WHERE id = ?1",
        params![r.id, r.title, r.all_day, r.updated_at],
    )?;
    Ok(())
}

/// `true` if the rule existed.
pub fn delete_rule(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM blocker_rules WHERE id = ?1", [id])? > 0)
}

/// One row of `blockers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockerRow {
    pub rule_id: String,
    pub source_event_id: String,
    pub uid: Option<String>,
    pub event_id: Option<String>,
    pub source_version: i64,
    pub rule_updated_at: i64,
    pub blocker_version: Option<i64>,
    pub attempts: i64,
    pub last_error: Option<String>,
}

const BLOCKER_COLUMNS: &str = "rule_id, source_event_id, uid, event_id, source_version, \
     rule_updated_at, blocker_version, attempts, last_error";

fn row_to_blocker(row: &Row<'_>) -> rusqlite::Result<BlockerRow> {
    Ok(BlockerRow {
        rule_id: row.get(0)?,
        source_event_id: row.get(1)?,
        uid: row.get(2)?,
        event_id: row.get(3)?,
        source_version: row.get(4)?,
        rule_updated_at: row.get(5)?,
        blocker_version: row.get(6)?,
        attempts: row.get(7)?,
        last_error: row.get(8)?,
    })
}

pub fn blockers(conn: &Connection) -> rusqlite::Result<Vec<BlockerRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {BLOCKER_COLUMNS} FROM blockers ORDER BY rule_id, source_event_id"
    ))?;
    let rows = stmt.query_map([], row_to_blocker)?;
    rows.collect()
}

/// Writes the row (insert or replace) and resets the error count.
pub fn put_blocker(conn: &Connection, b: &BlockerRow, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO blockers (rule_id, source_event_id, uid, event_id, source_version, \
             rule_updated_at, blocker_version, attempts, last_error, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
         ON CONFLICT (rule_id, source_event_id) DO UPDATE SET uid = ?3, event_id = ?4, \
             source_version = ?5, rule_updated_at = ?6, blocker_version = ?7, attempts = ?8, \
             last_error = ?9",
        params![
            b.rule_id,
            b.source_event_id,
            b.uid,
            b.event_id,
            b.source_version,
            b.rule_updated_at,
            b.blocker_version,
            b.attempts,
            b.last_error,
            now
        ],
    )?;
    Ok(())
}

pub fn blocker_failed(
    conn: &Connection,
    rule_id: &str,
    source_event_id: &str,
    message: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE blockers SET attempts = attempts + 1, last_error = ?3 \
         WHERE rule_id = ?1 AND source_event_id = ?2",
        params![rule_id, source_event_id, message],
    )?;
    Ok(())
}

pub fn delete_blocker(
    conn: &Connection,
    rule_id: &str,
    source_event_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM blockers WHERE rule_id = ?1 AND source_event_id = ?2",
        [rule_id, source_event_id],
    )?;
    Ok(())
}

/// Per rule: how many blockers exist and how many could not be written
/// (last attempt failed).
pub fn counts(conn: &Connection, rule_id: &str) -> rusqlite::Result<(i64, i64)> {
    conn.query_row(
        "SELECT COUNT(event_id), COUNT(last_error) FROM blockers WHERE rule_id = ?1",
        [rule_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// The newest error of a rule's blockers.
pub fn last_error(conn: &Connection, rule_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT last_error FROM blockers WHERE rule_id = ?1 AND last_error IS NOT NULL LIMIT 1",
        [rule_id],
        |r| r.get(0),
    )
    .optional()
}

/// A possible source: an event of a source calendar that has not ended
/// before `since` (or never ends).
#[derive(Debug, Clone)]
pub struct SourceRow {
    pub id: String,
    pub calendar_id: String,
    pub version: i64,
    pub ics: String,
}

pub fn sources(
    conn: &Connection,
    calendar_id: &str,
    since: i64,
) -> rusqlite::Result<Vec<SourceRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, calendar_id, version, ics FROM events \
         WHERE calendar_id = ?1 AND parse_error IS NULL \
           AND (range_end_utc IS NULL OR range_end_utc >= ?2) ORDER BY id",
    )?;
    let rows = stmt.query_map(params![calendar_id, since], |r| {
        Ok(SourceRow {
            id: r.get(0)?,
            calendar_id: r.get(1)?,
            version: r.get(2)?,
            ics: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// Version of a stored event, `None` if it does not exist.
pub fn event_version(conn: &Connection, id: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row("SELECT version FROM events WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .optional()
}

/// The event with this UID in a calendar.
pub fn event_by_uid(
    conn: &Connection,
    calendar_id: &str,
    uid: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM events WHERE calendar_id = ?1 AND uid = ?2",
        [calendar_id, uid],
        |r| r.get(0),
    )
    .optional()
}
