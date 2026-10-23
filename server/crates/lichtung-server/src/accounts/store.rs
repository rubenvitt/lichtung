//! SQL for accounts, calendars and calendar sets. Every function that
//! changes a synced object takes a fresh `seq` from [`changes::bump`] (or
//! leaves a tombstone) itself, so callers cannot forget it.

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::provider::RemoteCalendar;
use super::{AccountKind, StatusState};
use crate::changes::{self, ObjectType};

// ----- accounts -------------------------------------------------------------------

/// An account row. `credentials` stays encrypted; [`super`] decrypts it
/// only to talk to the provider.
#[derive(Clone)]
pub struct AccountRow {
    pub id: String,
    pub kind: AccountKind,
    pub name: String,
    pub username: String,
    pub base_url: String,
    pub principal_url: Option<String>,
    pub home_url: Option<String>,
    pub auto_schedule: bool,
    /// `calendar-user-address-set` (lower-case e-mail addresses).
    pub user_addresses: Vec<String>,
    pub credentials: Option<Vec<u8>>,
    pub status_state: StatusState,
    pub status_message: Option<String>,
    pub last_sync_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub seq: i64,
}

impl std::fmt::Debug for AccountRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountRow")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("username", &self.username)
            .field("base_url", &self.base_url)
            .field("credentials", &self.credentials.as_ref().map(|_| "***"))
            .field("status_state", &self.status_state)
            .field("seq", &self.seq)
            .finish_non_exhaustive()
    }
}

const ACCOUNT_COLUMNS: &str = "id, kind, name, username, base_url, principal_url, home_url, \
     auto_schedule, credentials, status_state, status_message, last_sync_at, created_at, \
     updated_at, seq, user_addresses";

fn text_enum<T>(row: &Row<'_>, idx: usize, parse: fn(&str) -> Option<T>) -> rusqlite::Result<T> {
    let raw: String = row.get(idx)?;
    parse(&raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            format!("unbekannter Wert {raw:?}").into(),
        )
    })
}

fn row_to_account(row: &Row<'_>) -> rusqlite::Result<AccountRow> {
    Ok(AccountRow {
        id: row.get(0)?,
        kind: text_enum(row, 1, AccountKind::parse)?,
        name: row.get(2)?,
        username: row.get(3)?,
        base_url: row.get(4)?,
        principal_url: row.get(5)?,
        home_url: row.get(6)?,
        auto_schedule: row.get(7)?,
        credentials: row.get(8)?,
        status_state: text_enum(row, 9, StatusState::parse)?,
        status_message: row.get(10)?,
        last_sync_at: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        seq: row.get(14)?,
        user_addresses: addresses_from_json(&row.get::<_, String>(15)?),
    })
}

fn addresses_from_json(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn addresses_to_json(addresses: &[String]) -> String {
    serde_json::to_string(addresses).unwrap_or_else(|_| "[]".into())
}

pub fn insert_account(conn: &Connection, a: &AccountRow) -> rusqlite::Result<i64> {
    let seq = changes::bump(conn)?;
    conn.execute(
        &format!(
            "INSERT INTO accounts ({ACCOUNT_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"
        ),
        params![
            a.id,
            a.kind.as_str(),
            a.name,
            a.username,
            a.base_url,
            a.principal_url,
            a.home_url,
            a.auto_schedule,
            a.credentials,
            a.status_state.as_str(),
            a.status_message,
            a.last_sync_at,
            a.created_at,
            a.updated_at,
            seq,
            addresses_to_json(&a.user_addresses)
        ],
    )?;
    Ok(seq)
}

/// Writes every field of an existing account with a fresh `seq`.
pub fn update_account(conn: &Connection, a: &AccountRow) -> rusqlite::Result<i64> {
    let seq = changes::bump(conn)?;
    conn.execute(
        "UPDATE accounts SET name = ?2, username = ?3, base_url = ?4, principal_url = ?5,
             home_url = ?6, auto_schedule = ?7, credentials = ?8, status_state = ?9,
             status_message = ?10, last_sync_at = ?11, updated_at = ?12, seq = ?13,
             user_addresses = ?14
         WHERE id = ?1",
        params![
            a.id,
            a.name,
            a.username,
            a.base_url,
            a.principal_url,
            a.home_url,
            a.auto_schedule,
            a.credentials,
            a.status_state.as_str(),
            a.status_message,
            a.last_sync_at,
            a.updated_at,
            seq,
            addresses_to_json(&a.user_addresses)
        ],
    )?;
    Ok(seq)
}

pub fn account(conn: &Connection, id: &str) -> rusqlite::Result<Option<AccountRow>> {
    conn.query_row(
        &format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?1"),
        [id],
        row_to_account,
    )
    .optional()
}

pub fn accounts(conn: &Connection) -> rusqlite::Result<Vec<AccountRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([], row_to_account)?;
    rows.collect()
}

/// The ID of an account with the same identity (kind, server, username).
pub fn find_account(
    conn: &Connection,
    kind: AccountKind,
    base_url: &str,
    username: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM accounts WHERE kind = ?1 AND base_url = ?2 AND username = ?3",
        params![kind.as_str(), base_url, username],
        |r| r.get(0),
    )
    .optional()
}

/// Deletes an account with its calendars and events, leaving tombstones for
/// every event, every calendar and the account itself. Returns whether
/// the account existed.
pub fn delete_account(conn: &Connection, id: &str, now: i64) -> rusqlite::Result<bool> {
    let calendar_ids: Vec<String> = {
        let mut stmt = conn.prepare_cached("SELECT id FROM calendars WHERE account_id = ?1")?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for calendar_id in &calendar_ids {
        delete_calendar(conn, calendar_id, now)?;
    }
    let n = conn.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
    if n > 0 {
        changes::tombstone(conn, ObjectType::Account, id, now)?;
    }
    Ok(n > 0)
}

// ----- calendars ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarRow {
    pub id: String,
    pub account_id: String,
    pub remote_href: String,
    pub name: String,
    pub description: Option<String>,
    pub provider_color: Option<String>,
    pub color: Option<String>,
    pub writable: bool,
    pub can_invite: bool,
    pub shared: bool,
    pub hidden: bool,
    pub mcp_write: bool,
    pub openings_busy: bool,
    pub all_day_blocks: bool,
    pub position: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub seq: i64,
}

const CALENDAR_COLUMNS: &str = "id, account_id, remote_href, name, description, provider_color, \
     color, writable, can_invite, shared, hidden, mcp_write, openings_busy, all_day_blocks, \
     position, created_at, updated_at, seq";

fn row_to_calendar(row: &Row<'_>) -> rusqlite::Result<CalendarRow> {
    Ok(CalendarRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        remote_href: row.get(2)?,
        name: row.get(3)?,
        description: row.get(4)?,
        provider_color: row.get(5)?,
        color: row.get(6)?,
        writable: row.get(7)?,
        can_invite: row.get(8)?,
        shared: row.get(9)?,
        hidden: row.get(10)?,
        mcp_write: row.get(11)?,
        openings_busy: row.get(12)?,
        all_day_blocks: row.get(13)?,
        position: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        seq: row.get(17)?,
    })
}

pub fn calendar(conn: &Connection, id: &str) -> rusqlite::Result<Option<CalendarRow>> {
    conn.query_row(
        &format!("SELECT {CALENDAR_COLUMNS} FROM calendars WHERE id = ?1"),
        [id],
        row_to_calendar,
    )
    .optional()
}

/// All calendars, or those of one account, by position.
pub fn calendars(
    conn: &Connection,
    account_id: Option<&str>,
) -> rusqlite::Result<Vec<CalendarRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {CALENDAR_COLUMNS} FROM calendars
         WHERE ?1 IS NULL OR account_id = ?1 ORDER BY position, name, id"
    ))?;
    let rows = stmt.query_map([account_id], row_to_calendar)?;
    rows.collect()
}

/// The next free position at the end of the calendar list.
pub fn next_calendar_position(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM calendars",
        [],
        |r| r.get(0),
    )
}

pub fn insert_calendar(conn: &Connection, c: &CalendarRow) -> rusqlite::Result<i64> {
    let seq = changes::bump(conn)?;
    conn.execute(
        &format!(
            "INSERT INTO calendars ({CALENDAR_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"
        ),
        params![
            c.id,
            c.account_id,
            c.remote_href,
            c.name,
            c.description,
            c.provider_color,
            c.color,
            c.writable,
            c.can_invite,
            c.shared,
            c.hidden,
            c.mcp_write,
            c.openings_busy,
            c.all_day_blocks,
            c.position,
            c.created_at,
            c.updated_at,
            seq
        ],
    )?;
    Ok(seq)
}

/// Writes provider fields and settings of an existing calendar with a
/// fresh `seq` (sync token and ctag stay untouched).
pub fn update_calendar(conn: &Connection, c: &CalendarRow) -> rusqlite::Result<i64> {
    let seq = changes::bump(conn)?;
    conn.execute(
        "UPDATE calendars SET name = ?2, description = ?3, provider_color = ?4, color = ?5,
             writable = ?6, can_invite = ?7, shared = ?8, hidden = ?9, mcp_write = ?10,
             openings_busy = ?11, all_day_blocks = ?12, position = ?13, updated_at = ?14,
             seq = ?15
         WHERE id = ?1",
        params![
            c.id,
            c.name,
            c.description,
            c.provider_color,
            c.color,
            c.writable,
            c.can_invite,
            c.shared,
            c.hidden,
            c.mcp_write,
            c.openings_busy,
            c.all_day_blocks,
            c.position,
            c.updated_at,
            seq
        ],
    )?;
    Ok(seq)
}

/// Deletes a calendar with its events, takes it out of every set (those
/// sets change) and leaves tombstones for the calendar and every event.
pub fn delete_calendar(conn: &Connection, id: &str, now: i64) -> rusqlite::Result<bool> {
    let set_ids: Vec<String> = {
        let mut stmt =
            conn.prepare_cached("SELECT set_id FROM calendar_set_members WHERE calendar_id = ?1")?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    // Events first, so each leaves a tombstone (the foreign key would
    // remove them silently).
    crate::events::store::delete_calendar_events(conn, id, now)?;
    let n = conn.execute("DELETE FROM calendars WHERE id = ?1", [id])?;
    if n == 0 {
        return Ok(false);
    }
    for set_id in &set_ids {
        touch_set(conn, set_id, now)?;
    }
    changes::tombstone(conn, ObjectType::Calendar, id, now)?;
    Ok(true)
}

/// Applies the provider's view of a calendar to a stored one. Returns
/// whether anything visible changed.
pub fn apply_remote(row: &mut CalendarRow, remote: &RemoteCalendar) -> bool {
    let before = row.clone();
    row.name.clone_from(&remote.name);
    row.description.clone_from(&remote.description);
    row.provider_color.clone_from(&remote.color);
    row.writable = remote.writable;
    row.can_invite = remote.can_invite;
    row.shared = remote.shared;
    // A share that became read-only takes Claude's write permission along.
    row.mcp_write &= row.writable;
    *row != before
}

// ----- calendar sets --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetRow {
    pub id: String,
    pub name: String,
    pub calendar_ids: Vec<String>,
    pub position: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub seq: i64,
}

fn set_members(conn: &Connection, set_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT calendar_id FROM calendar_set_members WHERE set_id = ?1 ORDER BY position",
    )?;
    let rows = stmt.query_map([set_id], |r| r.get(0))?;
    rows.collect()
}

fn row_to_set(conn: &Connection, row: &Row<'_>) -> rusqlite::Result<SetRow> {
    let id: String = row.get(0)?;
    Ok(SetRow {
        calendar_ids: set_members(conn, &id)?,
        id,
        name: row.get(1)?,
        position: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        seq: row.get(5)?,
    })
}

pub fn set(conn: &Connection, id: &str) -> rusqlite::Result<Option<SetRow>> {
    conn.query_row(
        "SELECT id, name, position, created_at, updated_at, seq FROM calendar_sets WHERE id = ?1",
        [id],
        |r| row_to_set(conn, r),
    )
    .optional()
}

pub fn sets(conn: &Connection) -> rusqlite::Result<Vec<SetRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, name, position, created_at, updated_at, seq FROM calendar_sets
         ORDER BY position, name, id",
    )?;
    let rows = stmt.query_map([], |r| row_to_set(conn, r))?;
    rows.collect()
}

pub fn next_set_position(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM calendar_sets",
        [],
        |r| r.get(0),
    )
}

/// Inserts or replaces a set with its members, with a fresh `seq`.
pub fn save_set(conn: &Connection, s: &SetRow) -> rusqlite::Result<i64> {
    let seq = changes::bump(conn)?;
    conn.execute(
        "INSERT INTO calendar_sets (id, name, position, created_at, updated_at, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, position = excluded.position,
             updated_at = excluded.updated_at, seq = excluded.seq",
        params![s.id, s.name, s.position, s.created_at, s.updated_at, seq],
    )?;
    conn.execute(
        "DELETE FROM calendar_set_members WHERE set_id = ?1",
        [&s.id],
    )?;
    for (position, calendar_id) in s.calendar_ids.iter().enumerate() {
        conn.execute(
            "INSERT INTO calendar_set_members (set_id, calendar_id, position) VALUES (?1, ?2, ?3)",
            params![s.id, calendar_id, position as i64],
        )?;
    }
    Ok(seq)
}

/// Marks a set as changed (its members changed elsewhere).
fn touch_set(conn: &Connection, id: &str, now: i64) -> rusqlite::Result<()> {
    let seq = changes::bump(conn)?;
    conn.execute(
        "UPDATE calendar_sets SET updated_at = ?2, seq = ?3 WHERE id = ?1",
        params![id, now, seq],
    )?;
    Ok(())
}

pub fn delete_set(conn: &Connection, id: &str, now: i64) -> rusqlite::Result<bool> {
    let n = conn.execute("DELETE FROM calendar_sets WHERE id = ?1", [id])?;
    if n > 0 {
        changes::tombstone(conn, ObjectType::CalendarSet, id, now)?;
    }
    Ok(n > 0)
}
