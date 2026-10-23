//! SQLite persistence: connection setup, migrations and all queries.
//!
//! Every function takes a plain [`Connection`]; the single-writer discipline
//! is enforced one level up in [`crate::app`]. Timestamps are Unix
//! milliseconds, IDs UUID v7 strings.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

/// Embedded migrations; index + 1 is the schema version (`user_version`).
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_accounts.sql"),
];

/// The schema version this binary expects.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Opens a connection with the pragmas Lichtung relies on (WAL, foreign
/// keys, busy timeout) and brings the schema up to date.
pub fn open(path: &Path) -> anyhow::Result<Connection> {
    let mut conn = Connection::open(path)
        .with_context(|| format!("Datenbank {} lässt sich nicht öffnen", path.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        bail!("SQLite WAL-Modus nicht verfügbar (journal_mode = {mode})");
    }
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Applies all migrations newer than the database's `user_version`, each in
/// its own transaction.
pub fn migrate(conn: &mut Connection) -> anyhow::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current > SCHEMA_VERSION {
        bail!(
            "Datenbank hat Schema-Version {current}, dieser Server kennt nur bis {SCHEMA_VERSION}"
        );
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let version = index as i64 + 1;
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("Migration {version} fehlgeschlagen"))?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        tracing::info!(version, "database migrated");
    }
    Ok(())
}

// ----- users ----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub subject: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
}

fn row_to_user(row: &Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: parse_uuid_col(row, 0)?,
        subject: row.get(1)?,
        email: row.get(2)?,
        name: row.get(3)?,
    })
}

/// The owner of this instance: the first user, created if none exists yet.
pub fn ensure_owner(conn: &Connection, now: i64) -> anyhow::Result<User> {
    let existing = conn
        .query_row(
            "SELECT id, oidc_subject, email, name FROM users ORDER BY created_at, rowid LIMIT 1",
            [],
            row_to_user,
        )
        .optional()?;
    if let Some(user) = existing {
        return Ok(user);
    }
    let id = Uuid::now_v7();
    conn.execute(
        "INSERT INTO users (id, created_at) VALUES (?1, ?2)",
        params![id.to_string(), now],
    )?;
    Ok(User {
        id,
        subject: None,
        email: None,
        name: None,
    })
}

pub fn user(conn: &Connection, id: Uuid) -> anyhow::Result<Option<User>> {
    Ok(conn
        .query_row(
            "SELECT id, oidc_subject, email, name FROM users WHERE id = ?1",
            [id.to_string()],
            row_to_user,
        )
        .optional()?)
}

/// Stores the identity (OIDC subject, email, name) of a user.
pub fn update_user_identity(
    conn: &Connection,
    id: Uuid,
    subject: Option<&str>,
    email: Option<&str>,
    name: Option<&str>,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE users SET oidc_subject = ?2, email = ?3, name = ?4 WHERE id = ?1",
        params![id.to_string(), subject, email, name],
    )?;
    Ok(())
}

// ----- API tokens -------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

fn row_to_token(row: &Row<'_>) -> rusqlite::Result<TokenRow> {
    Ok(TokenRow {
        id: parse_uuid_col(row, 0)?,
        user_id: parse_uuid_col(row, 1)?,
        name: row.get(2)?,
        created_at: row.get(3)?,
        last_used_at: row.get(4)?,
    })
}

pub fn insert_token(conn: &Connection, token: &TokenRow, hash: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO api_tokens (id, user_id, name, token_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            token.id.to_string(),
            token.user_id.to_string(),
            token.name,
            hash,
            token.created_at
        ],
    )?;
    Ok(())
}

/// Active (not revoked) tokens of a user, oldest first.
pub fn list_tokens(conn: &Connection, user_id: Uuid) -> anyhow::Result<Vec<TokenRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, user_id, name, created_at, last_used_at FROM api_tokens
         WHERE user_id = ?1 AND revoked_at IS NULL ORDER BY created_at, rowid",
    )?;
    let rows = stmt.query_map([user_id.to_string()], row_to_token)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Revokes an active token of the user. Returns whether one was revoked.
pub fn revoke_token(conn: &Connection, user_id: Uuid, id: Uuid, now: i64) -> anyhow::Result<bool> {
    let n = conn.execute(
        "UPDATE api_tokens SET revoked_at = ?3
         WHERE id = ?1 AND user_id = ?2 AND revoked_at IS NULL",
        params![id.to_string(), user_id.to_string(), now],
    )?;
    Ok(n > 0)
}

/// The active token with this hash.
pub fn find_token_by_hash(conn: &Connection, hash: &str) -> anyhow::Result<Option<TokenRow>> {
    Ok(conn
        .query_row(
            "SELECT id, user_id, name, created_at, last_used_at FROM api_tokens
             WHERE token_hash = ?1 AND revoked_at IS NULL",
            [hash],
            row_to_token,
        )
        .optional()?)
}

pub fn touch_token(conn: &Connection, id: Uuid, now: i64) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE api_tokens SET last_used_at = ?2 WHERE id = ?1",
        params![id.to_string(), now],
    )?;
    Ok(())
}

// ----- sessions -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub user_id: Uuid,
    pub expires_at: i64,
    pub last_seen_at: i64,
}

pub fn insert_session(
    conn: &Connection,
    id_hash: &str,
    user_id: Uuid,
    now: i64,
    expires_at: i64,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO sessions (id_hash, user_id, created_at, expires_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?3)",
        params![id_hash, user_id.to_string(), now, expires_at],
    )?;
    Ok(())
}

/// The session with this hash, if it has not expired at `now`.
pub fn find_session(
    conn: &Connection,
    id_hash: &str,
    now: i64,
) -> anyhow::Result<Option<SessionRow>> {
    Ok(conn
        .query_row(
            "SELECT user_id, expires_at, last_seen_at FROM sessions
             WHERE id_hash = ?1 AND expires_at > ?2",
            params![id_hash, now],
            |row| {
                Ok(SessionRow {
                    user_id: parse_uuid_col(row, 0)?,
                    expires_at: row.get(1)?,
                    last_seen_at: row.get(2)?,
                })
            },
        )
        .optional()?)
}

pub fn extend_session(
    conn: &Connection,
    id_hash: &str,
    now: i64,
    expires_at: i64,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE sessions SET last_seen_at = ?2, expires_at = ?3 WHERE id_hash = ?1",
        params![id_hash, now, expires_at],
    )?;
    Ok(())
}

pub fn delete_session(conn: &Connection, id_hash: &str) -> anyhow::Result<()> {
    conn.execute("DELETE FROM sessions WHERE id_hash = ?1", [id_hash])?;
    Ok(())
}

/// Removes expired sessions; returns how many.
pub fn purge_expired_sessions(conn: &Connection, now: i64) -> anyhow::Result<usize> {
    Ok(conn.execute("DELETE FROM sessions WHERE expires_at <= ?1", [now])?)
}

// ----- OAuth (built-in authorization server for /mcp) ----------------------------

/// A dynamically registered OAuth client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClientRow {
    pub id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

fn row_to_oauth_client(row: &Row<'_>) -> rusqlite::Result<OAuthClientRow> {
    let uris: String = row.get(2)?;
    Ok(OAuthClientRow {
        id: row.get(0)?,
        name: row.get(1)?,
        redirect_uris: serde_json::from_str(&uris).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
        })?,
        created_at: row.get(3)?,
        last_used_at: row.get(4)?,
        revoked_at: row.get(5)?,
    })
}

const OAUTH_CLIENT_COLUMNS: &str =
    "c.id, c.name, c.redirect_uris_json, c.created_at, c.last_used_at, c.revoked_at";

pub fn insert_oauth_client(conn: &Connection, client: &OAuthClientRow) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO oauth_clients (id, name, redirect_uris_json, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![
            client.id,
            client.name,
            serde_json::to_string(&client.redirect_uris)?,
            client.created_at
        ],
    )?;
    Ok(())
}

/// The client with this ID (also revoked ones).
pub fn oauth_client(conn: &Connection, id: &str) -> anyhow::Result<Option<OAuthClientRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {OAUTH_CLIENT_COLUMNS} FROM oauth_clients c WHERE c.id = ?1"),
            [id],
            row_to_oauth_client,
        )
        .optional()?)
}

/// Number of clients registered at or after `since` (all clients for 0).
pub fn count_oauth_clients_since(conn: &Connection, since: i64) -> anyhow::Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM oauth_clients WHERE created_at >= ?1",
        [since],
        |r| r.get(0),
    )?)
}

/// Clients of the user that hold a live grant (a token that is neither
/// revoked nor expired), oldest first.
pub fn connected_oauth_clients(
    conn: &Connection,
    user_id: Uuid,
    now: i64,
) -> anyhow::Result<Vec<OAuthClientRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {OAUTH_CLIENT_COLUMNS} FROM oauth_clients c
         WHERE c.revoked_at IS NULL AND EXISTS (
             SELECT 1 FROM oauth_tokens t
             WHERE t.client_id = c.id AND t.user_id = ?1
               AND t.revoked_at IS NULL AND t.expires_at > ?2)
         ORDER BY c.created_at, c.rowid"
    ))?;
    let rows = stmt.query_map(params![user_id.to_string(), now], row_to_oauth_client)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Revokes a client that holds a grant of the user, together with all its
/// tokens. Returns whether a client was revoked.
pub fn revoke_oauth_client(
    conn: &mut Connection,
    user_id: Uuid,
    id: &str,
    now: i64,
) -> anyhow::Result<bool> {
    let tx = conn.transaction()?;
    let owned: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM oauth_tokens WHERE client_id = ?1 AND user_id = ?2)",
        params![id, user_id.to_string()],
        |r| r.get(0),
    )?;
    let n = if owned {
        tx.execute(
            "UPDATE oauth_clients SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
            params![id, now],
        )?
    } else {
        0
    };
    if n > 0 {
        tx.execute(
            "UPDATE oauth_tokens SET revoked_at = ?2 WHERE client_id = ?1 AND revoked_at IS NULL",
            params![id, now],
        )?;
    }
    tx.commit()?;
    Ok(n > 0)
}

pub fn touch_oauth_client(conn: &Connection, id: &str, now: i64) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE oauth_clients SET last_used_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

/// A stored authorization code (looked up by its hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCodeRow {
    pub client_id: String,
    pub user_id: Uuid,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub resource: String,
    pub scope: Option<String>,
    pub family_id: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub used_at: Option<i64>,
}

pub fn insert_oauth_code(conn: &Connection, hash: &str, code: &OAuthCodeRow) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO oauth_codes (code_hash, client_id, user_id, redirect_uri, code_challenge,
             resource, scope, family_id, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            hash,
            code.client_id,
            code.user_id.to_string(),
            code.redirect_uri,
            code.code_challenge,
            code.resource,
            code.scope,
            code.family_id,
            code.created_at,
            code.expires_at
        ],
    )?;
    Ok(())
}

pub fn oauth_code(conn: &Connection, hash: &str) -> anyhow::Result<Option<OAuthCodeRow>> {
    Ok(conn
        .query_row(
            "SELECT client_id, user_id, redirect_uri, code_challenge, resource, scope, family_id,
                    created_at, expires_at, used_at
             FROM oauth_codes WHERE code_hash = ?1",
            [hash],
            |row| {
                Ok(OAuthCodeRow {
                    client_id: row.get(0)?,
                    user_id: parse_uuid_col(row, 1)?,
                    redirect_uri: row.get(2)?,
                    code_challenge: row.get(3)?,
                    resource: row.get(4)?,
                    scope: row.get(5)?,
                    family_id: row.get(6)?,
                    created_at: row.get(7)?,
                    expires_at: row.get(8)?,
                    used_at: row.get(9)?,
                })
            },
        )
        .optional()?)
}

/// Marks a code as used; `false` if it already was (single use).
pub fn mark_oauth_code_used(conn: &Connection, hash: &str, now: i64) -> anyhow::Result<bool> {
    let n = conn.execute(
        "UPDATE oauth_codes SET used_at = ?2 WHERE code_hash = ?1 AND used_at IS NULL",
        params![hash, now],
    )?;
    Ok(n > 0)
}

/// Kind of an OAuth token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthTokenKind {
    Access,
    Refresh,
}

impl OAuthTokenKind {
    fn as_str(self) -> &'static str {
        match self {
            OAuthTokenKind::Access => "access",
            OAuthTokenKind::Refresh => "refresh",
        }
    }
}

/// A stored OAuth token (looked up by its hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthTokenRow {
    pub id: String,
    pub kind: OAuthTokenKind,
    pub client_id: String,
    pub user_id: Uuid,
    pub family_id: String,
    pub resource: String,
    pub scope: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

pub fn insert_oauth_token(
    conn: &Connection,
    hash: &str,
    token: &OAuthTokenRow,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO oauth_tokens (id, token_hash, kind, client_id, user_id, family_id, resource,
             scope, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            token.id,
            hash,
            token.kind.as_str(),
            token.client_id,
            token.user_id.to_string(),
            token.family_id,
            token.resource,
            token.scope,
            token.created_at,
            token.expires_at
        ],
    )?;
    Ok(())
}

/// The token with this hash (in any state) together with its client.
pub fn oauth_token(
    conn: &Connection,
    hash: &str,
) -> anyhow::Result<Option<(OAuthTokenRow, OAuthClientRow)>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT t.id, t.kind, t.client_id, t.user_id, t.family_id, t.resource, t.scope,
                        t.created_at, t.expires_at, t.used_at, t.revoked_at, {OAUTH_CLIENT_COLUMNS}
                 FROM oauth_tokens t JOIN oauth_clients c ON c.id = t.client_id
                 WHERE t.token_hash = ?1"
            ),
            [hash],
            |row| {
                let kind: String = row.get(1)?;
                let token = OAuthTokenRow {
                    id: row.get(0)?,
                    kind: if kind == "refresh" {
                        OAuthTokenKind::Refresh
                    } else {
                        OAuthTokenKind::Access
                    },
                    client_id: row.get(2)?,
                    user_id: parse_uuid_col(row, 3)?,
                    family_id: row.get(4)?,
                    resource: row.get(5)?,
                    scope: row.get(6)?,
                    created_at: row.get(7)?,
                    expires_at: row.get(8)?,
                    used_at: row.get(9)?,
                    revoked_at: row.get(10)?,
                };
                let client = OAuthClientRow {
                    id: row.get(11)?,
                    name: row.get(12)?,
                    redirect_uris: serde_json::from_str(&row.get::<_, String>(13)?).map_err(
                        |e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                13,
                                rusqlite::types::Type::Text,
                                Box::new(e),
                            )
                        },
                    )?,
                    created_at: row.get(14)?,
                    last_used_at: row.get(15)?,
                    revoked_at: row.get(16)?,
                };
                Ok((token, client))
            },
        )
        .optional()?)
}

/// Marks a refresh token as rotated; `false` if it already was.
pub fn mark_oauth_token_used(conn: &Connection, id: &str, now: i64) -> anyhow::Result<bool> {
    let n = conn.execute(
        "UPDATE oauth_tokens SET used_at = ?2 WHERE id = ?1 AND used_at IS NULL",
        params![id, now],
    )?;
    Ok(n > 0)
}

pub fn revoke_oauth_token(conn: &Connection, id: &str, now: i64) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE oauth_tokens SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
        params![id, now],
    )?;
    Ok(())
}

/// Revokes every token of a family, including a device logged in with it;
/// returns how many were still active.
pub fn revoke_oauth_family(conn: &Connection, family_id: &str, now: i64) -> anyhow::Result<usize> {
    let tokens = conn.execute(
        "UPDATE oauth_tokens SET revoked_at = ?2 WHERE family_id = ?1 AND revoked_at IS NULL",
        params![family_id, now],
    )?;
    let devices = conn.execute(
        "UPDATE devices SET revoked_at = ?2 WHERE family_id = ?1 AND revoked_at IS NULL",
        params![family_id, now],
    )?;
    Ok(tokens + devices)
}

/// Removes expired codes and tokens and clients that can no longer be used:
/// registered before `stale_before` without a live token, or registered
/// before `unused_before` and never used (no code, no token). Returns the
/// number of removed clients.
pub fn purge_oauth(
    conn: &Connection,
    now: i64,
    stale_before: i64,
    unused_before: i64,
) -> anyhow::Result<usize> {
    conn.execute("DELETE FROM oauth_codes WHERE expires_at <= ?1", [now])?;
    conn.execute("DELETE FROM oauth_tokens WHERE expires_at <= ?1", [now])?;
    let stale = conn.execute(
        "DELETE FROM oauth_clients
         WHERE created_at < ?2 AND NOT EXISTS (
             SELECT 1 FROM oauth_tokens t
             WHERE t.client_id = oauth_clients.id AND t.revoked_at IS NULL AND t.expires_at > ?1)",
        params![now, stale_before],
    )?;
    let unused = conn.execute(
        "DELETE FROM oauth_clients
         WHERE created_at < ?1 AND last_used_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM oauth_codes c WHERE c.client_id = oauth_clients.id)
           AND NOT EXISTS (SELECT 1 FROM oauth_tokens t WHERE t.client_id = oauth_clients.id)",
        [unused_before],
    )?;
    Ok(stale + unused)
}

// ----- devices (iOS/macOS app logged in via OAuth) ---------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub family_id: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

const DEVICE_COLUMNS: &str = "id, user_id, name, family_id, created_at, last_used_at";

fn row_to_device(row: &Row<'_>) -> rusqlite::Result<DeviceRow> {
    Ok(DeviceRow {
        id: parse_uuid_col(row, 0)?,
        user_id: parse_uuid_col(row, 1)?,
        name: row.get(2)?,
        family_id: row.get(3)?,
        created_at: row.get(4)?,
        last_used_at: row.get(5)?,
    })
}

pub fn insert_device(conn: &Connection, device: &DeviceRow, hash: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO devices (id, user_id, name, token_hash, family_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            device.id.to_string(),
            device.user_id.to_string(),
            device.name,
            hash,
            device.family_id,
            device.created_at
        ],
    )?;
    Ok(())
}

/// Active (not revoked) devices of a user, oldest first.
pub fn list_devices(conn: &Connection, user_id: Uuid) -> anyhow::Result<Vec<DeviceRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {DEVICE_COLUMNS} FROM devices
         WHERE user_id = ?1 AND revoked_at IS NULL ORDER BY created_at, rowid"
    ))?;
    let rows = stmt.query_map([user_id.to_string()], row_to_device)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The active device with this token hash.
pub fn find_device_by_hash(conn: &Connection, hash: &str) -> anyhow::Result<Option<DeviceRow>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {DEVICE_COLUMNS} FROM devices WHERE token_hash = ?1 AND revoked_at IS NULL"
            ),
            [hash],
            row_to_device,
        )
        .optional()?)
}

/// Revokes an active device of the user. Returns whether one was revoked.
pub fn revoke_device(conn: &Connection, user_id: Uuid, id: Uuid, now: i64) -> anyhow::Result<bool> {
    let n = conn.execute(
        "UPDATE devices SET revoked_at = ?3 WHERE id = ?1 AND user_id = ?2 AND revoked_at IS NULL",
        params![id.to_string(), user_id.to_string(), now],
    )?;
    Ok(n > 0)
}

/// Revokes the device with this token hash (token revocation, RFC 7009).
pub fn revoke_device_by_hash(conn: &Connection, hash: &str, now: i64) -> anyhow::Result<bool> {
    let n = conn.execute(
        "UPDATE devices SET revoked_at = ?2 WHERE token_hash = ?1 AND revoked_at IS NULL",
        params![hash, now],
    )?;
    Ok(n > 0)
}

pub fn touch_device(conn: &Connection, id: Uuid, now: i64) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE devices SET last_used_at = ?2 WHERE id = ?1",
        params![id.to_string(), now],
    )?;
    Ok(())
}

// ----- backup ---------------------------------------------------------------------

/// Writes a consistent copy of the database to `dest` (`VACUUM INTO`).
pub fn vacuum_into(conn: &Connection, dest: &Path) -> anyhow::Result<()> {
    let dest = dest
        .to_str()
        .ok_or_else(|| anyhow!("Backup-Pfad ist kein UTF-8: {}", dest.display()))?;
    conn.execute("VACUUM INTO ?1", [dest])?;
    Ok(())
}

// ----- parsing helpers ------------------------------------------------------------

fn parse_uuid_col(row: &Row<'_>, idx: usize) -> rusqlite::Result<Uuid> {
    let s: String = row.get(idx)?;
    Uuid::parse_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("lichtung.db")).unwrap();
        (dir, conn)
    }

    #[test]
    fn migrations_set_user_version_and_are_idempotent() {
        let (dir, conn) = temp_db();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
        drop(conn);
        // reopening does not re-run migrations
        open(&dir.path().join("lichtung.db")).unwrap();
    }

    #[test]
    fn refuses_newer_schema() {
        let (dir, conn) = temp_db();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(conn);
        assert!(open(&dir.path().join("lichtung.db")).is_err());
    }

    #[test]
    fn ensure_owner_is_stable() {
        let (_dir, conn) = temp_db();
        let a = ensure_owner(&conn, 1).unwrap();
        let b = ensure_owner(&conn, 2).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn tokens_and_devices_are_revoked_per_user() {
        let (_dir, conn) = temp_db();
        let owner = ensure_owner(&conn, 1).unwrap().id;
        let token = TokenRow {
            id: Uuid::now_v7(),
            user_id: owner,
            name: "werkzeug".into(),
            created_at: 1,
            last_used_at: None,
        };
        insert_token(&conn, &token, "hash-t").unwrap();
        assert_eq!(
            find_token_by_hash(&conn, "hash-t").unwrap(),
            Some(token.clone())
        );
        assert!(!revoke_token(&conn, Uuid::now_v7(), token.id, 2).unwrap());
        assert!(revoke_token(&conn, owner, token.id, 2).unwrap());
        assert!(!revoke_token(&conn, owner, token.id, 3).unwrap());
        assert_eq!(find_token_by_hash(&conn, "hash-t").unwrap(), None);
        assert!(list_tokens(&conn, owner).unwrap().is_empty());

        let device = DeviceRow {
            id: Uuid::now_v7(),
            user_id: owner,
            name: "Testgerät".into(),
            family_id: "familie".into(),
            created_at: 1,
            last_used_at: None,
        };
        insert_device(&conn, &device, "hash-d").unwrap();
        assert_eq!(list_devices(&conn, owner).unwrap(), vec![device.clone()]);
        // Replaying the authorization code revokes the device of its family.
        assert_eq!(revoke_oauth_family(&conn, "familie", 2).unwrap(), 1);
        assert_eq!(find_device_by_hash(&conn, "hash-d").unwrap(), None);
    }

    #[test]
    fn unused_oauth_clients_are_purged_early() {
        let (_dir, conn) = temp_db();
        let client = |id: &str, created_at: i64| OAuthClientRow {
            id: id.into(),
            name: id.into(),
            redirect_uris: vec!["https://claude.ai/api/mcp/auth_callback".into()],
            created_at,
            last_used_at: None,
            revoked_at: None,
        };
        insert_oauth_client(&conn, &client("old-unused", 1_000)).unwrap();
        insert_oauth_client(&conn, &client("old-used", 1_000)).unwrap();
        insert_oauth_client(&conn, &client("new-unused", 5_000)).unwrap();
        touch_oauth_client(&conn, "old-used", 2_000).unwrap();

        // stale_before far in the past: only the early rule applies
        assert_eq!(purge_oauth(&conn, 6_000, 0, 4_000).unwrap(), 1);
        assert!(oauth_client(&conn, "old-unused").unwrap().is_none());
        assert!(oauth_client(&conn, "old-used").unwrap().is_some());
        assert!(oauth_client(&conn, "new-unused").unwrap().is_some());
    }
}
