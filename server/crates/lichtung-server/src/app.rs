//! Application service: the single entry point for everything that reads or
//! changes data. HTTP handlers and the OAuth server go through [`App`].
//!
//! Concurrency model (correctness first, as in Zunder):
//!
//! - One writer connection behind a mutex; every write runs in
//!   `spawn_blocking` while holding it, so no two writes interleave.
//! - One reader connection (WAL allows reading while writing) for lookups.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use anyhow::Context;
use rusqlite::{Connection, Transaction};
use serde::Serialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::auth::{self, Principal, PrincipalKind};
use crate::changes;
use crate::config::Config;
use crate::crypto::{self, Cipher};
use crate::db::{self, TokenRow, User};
use crate::error::{AppError, AppResult};

/// Number of nightly backups kept in `<data>/backups`.
pub const BACKUPS_TO_KEEP: usize = 14;
/// Maximum length of an API token name.
pub const MAX_TOKEN_NAME_CHARS: usize = 64;

/// An API token without its secret.
#[derive(Debug, Clone, Serialize)]
pub struct TokenInfo {
    pub id: Uuid,
    pub name: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

/// A newly created API token; `token` is shown exactly once.
#[derive(Debug, Clone, Serialize)]
pub struct CreatedToken {
    #[serde(flatten)]
    pub info: TokenInfo,
    pub token: String,
}

/// A newly created session.
#[derive(Debug, Clone)]
pub struct NewSession {
    /// The raw session ID for the cookie (never stored).
    pub id: String,
    pub expires_at: i64,
}

/// A successfully authenticated session.
#[derive(Debug, Clone)]
pub struct SessionAuth {
    pub principal: Principal,
    /// New expiry when the sliding window was extended (cookie must be
    /// re-sent).
    pub renewed_until: Option<i64>,
}

/// Identity claims of a successful OIDC login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone)]
pub struct App {
    inner: Arc<Inner>,
}

struct Inner {
    config: Config,
    owner_id: Uuid,
    writer: Mutex<Connection>,
    reader: Mutex<Connection>,
    shutdown: CancellationToken,
    bootstrap_hash: Option<String>,
    /// Encrypts provider credentials (`LICHTUNG_SECRET_KEY`).
    cipher: Cipher,
    /// Latest change cursor, published after each committed change.
    changes: watch::Sender<i64>,
    /// Accounts with a sync running in the background.
    syncing: Mutex<HashSet<String>>,
    /// Wakes the sync scheduler (new account, new credentials).
    sync_wakeup: tokio::sync::Notify,
    /// One lock per calendar: a sync round and a write to the provider
    /// never overlap on the same calendar (see [`App::lock_calendars`]).
    calendar_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Serializes mutations (push, MCP): idempotency checks and the
    /// version check before a write see every earlier write.
    write_lock: Arc<tokio::sync::Mutex<()>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The current time in Unix milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Formats Unix milliseconds as RFC 3339 (UTC).
pub fn format_ms(ms: i64) -> String {
    let t = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    t.format(&time::format_description::well_known::Rfc3339)
        .expect("valid timestamp formats")
}

impl App {
    /// Opens (and migrates) the database in `config.data_dir` and makes
    /// sure the owner exists.
    pub async fn open(config: Config) -> anyhow::Result<App> {
        tokio::task::spawn_blocking(move || Self::open_blocking(config)).await?
    }

    fn open_blocking(config: Config) -> anyhow::Result<App> {
        std::fs::create_dir_all(&config.data_dir).with_context(|| {
            format!(
                "Datenverzeichnis {} lässt sich nicht anlegen",
                config.data_dir.display()
            )
        })?;
        let path = config.db_path();
        let conn = db::open(&path)?;
        let owner = db::ensure_owner(&conn, now_ms())?;
        if config.dev_auth && owner.name.is_none() && owner.email.is_none() {
            // Placeholder identity for local development; a later OIDC login
            // replaces it.
            db::update_user_identity(
                &conn,
                owner.id,
                owner.subject.as_deref(),
                Some("dev@localhost"),
                Some("Entwicklung"),
            )?;
        }
        let cipher = Cipher::new(&config.secret_key);
        crypto::verify_or_init_key_check(&conn, &cipher)?;
        crate::accounts::reset_stale_sync_states(&conn)?;
        let cursor = changes::current(&conn)?;
        let reader = db::open(&path)?;
        let bootstrap_hash = config.bootstrap_token.as_deref().map(auth::sha256_hex);
        Ok(App {
            inner: Arc::new(Inner {
                owner_id: owner.id,
                writer: Mutex::new(conn),
                reader: Mutex::new(reader),
                shutdown: CancellationToken::new(),
                bootstrap_hash,
                cipher,
                changes: watch::channel(cursor).0,
                syncing: Mutex::new(HashSet::new()),
                sync_wakeup: tokio::sync::Notify::new(),
                calendar_locks: Mutex::new(HashMap::new()),
                write_lock: Arc::new(tokio::sync::Mutex::new(())),
                config,
            }),
        })
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// The owner of this instance (single-user server).
    pub fn owner_id(&self) -> Uuid {
        self.inner.owner_id
    }

    /// Cancelled on shutdown; long-running streams and tasks stop then.
    pub fn shutdown_token(&self) -> CancellationToken {
        self.inner.shutdown.clone()
    }

    async fn blocking<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Inner) -> AppResult<T> + Send + 'static,
    {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || f(&inner))
            .await
            .map_err(|e| AppError::Internal(e.into()))?
    }

    /// Runs `f` with the writer connection (serialized with every other
    /// write) on the blocking pool.
    pub(crate) async fn with_writer<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    {
        self.blocking(move |inner| f(&mut lock(&inner.writer)))
            .await
    }

    /// Runs `f` in a write transaction (serialized with every other write)
    /// and, once it has committed, publishes the new change cursor if `f`
    /// recorded changes ([`changes::bump`], [`changes::tombstone`]).
    pub(crate) async fn with_change_tx<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> AppResult<T> + Send + 'static,
    {
        self.blocking(move |inner| {
            let mut conn = lock(&inner.writer);
            let tx = conn.transaction()?;
            let out = f(&tx)?;
            let cursor = changes::current(&tx)?;
            tx.commit()?;
            // Still under the writer lock, so cursors are published in order.
            inner.changes.send_if_modified(|current| {
                if cursor > *current {
                    *current = cursor;
                    true
                } else {
                    false
                }
            });
            Ok(out)
        })
        .await
    }

    /// Receives the change cursor after every committed change (for SSE,
    /// LIC-11). Several changes may collapse into one notification.
    pub fn subscribe_changes(&self) -> watch::Receiver<i64> {
        self.inner.changes.subscribe()
    }

    /// The latest published change cursor.
    pub fn change_cursor(&self) -> i64 {
        *self.inner.changes.borrow()
    }

    pub(crate) fn cipher(&self) -> &Cipher {
        &self.inner.cipher
    }

    /// Marks a background sync of `account_id` as running; `false` if one
    /// already runs.
    pub(crate) fn begin_sync(&self, account_id: &str) -> bool {
        lock(&self.inner.syncing).insert(account_id.to_owned())
    }

    pub(crate) fn end_sync(&self, account_id: &str) {
        lock(&self.inner.syncing).remove(account_id);
    }

    pub(crate) fn is_syncing(&self, account_id: &str) -> bool {
        lock(&self.inner.syncing).contains(account_id)
    }

    /// Asks the sync scheduler to look at the accounts now.
    pub(crate) fn wake_sync_scheduler(&self) {
        self.inner.sync_wakeup.notify_one();
    }

    /// Resolves when [`App::wake_sync_scheduler`] was called.
    pub(crate) async fn sync_wakeup(&self) {
        self.inner.sync_wakeup.notified().await;
    }

    /// Locks the given calendars (sorted, so two callers never deadlock)
    /// until the guards are dropped. A sync round holds its calendar's lock
    /// from listing to storing, a write from reading the stored version to
    /// storing the result; so neither stores data the other has already
    /// replaced.
    pub(crate) async fn lock_calendars(
        &self,
        ids: &[&str],
    ) -> Vec<tokio::sync::OwnedMutexGuard<()>> {
        let mut ids: Vec<&str> = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let locks: Vec<Arc<tokio::sync::Mutex<()>>> = {
            let mut map = lock(&self.inner.calendar_locks);
            // Drop locks nobody holds or waits for, so the map stays small.
            map.retain(|_, l| Arc::strong_count(l) > 1);
            ids.iter()
                .map(|id| map.entry((*id).to_owned()).or_default().clone())
                .collect()
        };
        let mut guards = Vec::with_capacity(locks.len());
        for l in locks {
            guards.push(l.lock_owned().await);
        }
        guards
    }

    /// Held while one mutation runs (see `Inner::write_lock`).
    pub(crate) async fn write_guard(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.inner.write_lock.clone().lock_owned().await
    }

    /// Runs `f` with the reader connection on the blocking pool.
    pub(crate) async fn with_reader<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> AppResult<T> + Send + 'static,
    {
        self.blocking(move |inner| f(&lock(&inner.reader))).await
    }

    // ----- users ------------------------------------------------------------------

    pub async fn user(&self, id: Uuid) -> AppResult<Option<User>> {
        self.with_reader(move |conn| Ok(db::user(conn, id)?)).await
    }

    /// Binds a successful OIDC login (already checked against the allowed
    /// person) to the owner. The first login stores the identity; later
    /// logins refresh email and name.
    pub async fn sign_in(&self, identity: Identity) -> AppResult<User> {
        self.blocking(move |inner| {
            let conn = lock(&inner.writer);
            let owner =
                db::user(&conn, inner.owner_id)?.context("Besitzer fehlt in der Datenbank")?;
            if let Some(previous) = owner.subject.as_deref() {
                if previous != identity.subject {
                    tracing::warn!("OIDC subject of the owner changed");
                }
            }
            db::update_user_identity(
                &conn,
                inner.owner_id,
                Some(&identity.subject),
                identity.email.as_deref(),
                identity.name.as_deref().or(owner.name.as_deref()),
            )?;
            Ok(db::user(&conn, inner.owner_id)?.context("Besitzer fehlt")?)
        })
        .await
    }

    /// The owner (dev auth logs in as the owner).
    pub async fn owner(&self) -> AppResult<User> {
        let id = self.inner.owner_id;
        self.user(id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Besitzer fehlt in der Datenbank")))
    }

    // ----- sessions -----------------------------------------------------------------

    fn session_key(&self) -> &[u8] {
        self.inner.config.session_secret.as_bytes()
    }

    pub async fn create_session(&self, user_id: Uuid) -> AppResult<NewSession> {
        let id = auth::random_secret();
        let hash = auth::session_hash(self.session_key(), &id);
        let now = now_ms();
        let expires_at = now + auth::SESSION_TTL_MS;
        self.with_writer(move |conn| {
            db::insert_session(conn, &hash, user_id, now, expires_at)?;
            Ok(())
        })
        .await?;
        Ok(NewSession { id, expires_at })
    }

    /// Looks up a session by its raw ID and extends it (sliding expiry).
    pub async fn authenticate_session(&self, raw: &str) -> AppResult<Option<SessionAuth>> {
        let hash = auth::session_hash(self.session_key(), raw);
        let now = now_ms();
        self.blocking(move |inner| {
            let found = db::find_session(&lock(&inner.reader), &hash, now)?;
            let Some(session) = found else {
                return Ok(None);
            };
            let renewed_until = if now - session.last_seen_at >= auth::SESSION_RENEW_AFTER_MS {
                let expires_at = now + auth::SESSION_TTL_MS;
                db::extend_session(&lock(&inner.writer), &hash, now, expires_at)?;
                Some(expires_at)
            } else {
                None
            };
            Ok(Some(SessionAuth {
                principal: Principal {
                    user_id: session.user_id,
                    kind: PrincipalKind::Session,
                },
                renewed_until,
            }))
        })
        .await
    }

    pub async fn end_session(&self, raw: &str) -> AppResult<()> {
        let hash = auth::session_hash(self.session_key(), raw);
        self.with_writer(move |conn| Ok(db::delete_session(conn, &hash)?))
            .await
    }

    // ----- API and device tokens ---------------------------------------------------------

    /// Authenticates a bearer token for `/api`: API token, bootstrap token or
    /// device token. OAuth access tokens are not accepted here (only `/mcp`
    /// takes them, see [`crate::oauth::authenticate_mcp`]).
    pub async fn authenticate_token(&self, raw: &str) -> AppResult<Option<Principal>> {
        let hash = auth::sha256_hex(raw);
        if let Some(bootstrap) = &self.inner.bootstrap_hash {
            if auth::constant_time_eq(bootstrap.as_bytes(), hash.as_bytes()) {
                return Ok(Some(Principal {
                    user_id: self.inner.owner_id,
                    kind: PrincipalKind::Bootstrap,
                }));
            }
        }
        if raw.starts_with(auth::DEVICE_TOKEN_PREFIX) {
            return self.authenticate_device(hash).await;
        }
        if !raw.starts_with(auth::TOKEN_PREFIX) {
            return Ok(None);
        }
        let now = now_ms();
        self.blocking(move |inner| {
            let found = db::find_token_by_hash(&lock(&inner.reader), &hash)?;
            let Some(token) = found else {
                return Ok(None);
            };
            let stale = token
                .last_used_at
                .is_none_or(|t| now - t >= auth::TOKEN_TOUCH_INTERVAL_MS);
            if stale {
                db::touch_token(&lock(&inner.writer), token.id, now)?;
            }
            Ok(Some(Principal {
                user_id: token.user_id,
                kind: PrincipalKind::Token {
                    id: token.id,
                    name: token.name,
                },
            }))
        })
        .await
    }

    async fn authenticate_device(&self, hash: String) -> AppResult<Option<Principal>> {
        let now = now_ms();
        self.blocking(move |inner| {
            let Some(device) = db::find_device_by_hash(&lock(&inner.reader), &hash)? else {
                return Ok(None);
            };
            if device
                .last_used_at
                .is_none_or(|t| now - t >= auth::TOKEN_TOUCH_INTERVAL_MS)
            {
                db::touch_device(&lock(&inner.writer), device.id, now)?;
            }
            Ok(Some(Principal {
                user_id: device.user_id,
                kind: PrincipalKind::Device {
                    id: device.id,
                    name: device.name,
                },
            }))
        })
        .await
    }

    pub async fn list_tokens(&self, user_id: Uuid) -> AppResult<Vec<TokenInfo>> {
        self.with_reader(move |conn| {
            let rows = db::list_tokens(conn, user_id)?;
            Ok(rows.iter().map(token_info).collect())
        })
        .await
    }

    /// Creates an API token. Names are trimmed, 1–64 characters and unique
    /// among the user's active tokens.
    pub async fn create_token(&self, user_id: Uuid, name: &str) -> AppResult<CreatedToken> {
        let name = name.trim().to_owned();
        let len = name.chars().count();
        if len == 0 || len > MAX_TOKEN_NAME_CHARS {
            return Err(AppError::bad_request(format!(
                "Der Name eines Tokens muss 1 bis {MAX_TOKEN_NAME_CHARS} Zeichen lang sein"
            )));
        }
        if name.chars().any(char::is_control) {
            return Err(AppError::bad_request(
                "Der Name eines Tokens darf keine Steuerzeichen enthalten",
            ));
        }
        if name == auth::BOOTSTRAP_TOKEN_NAME {
            return Err(AppError::bad_request(
                "Der Name „bootstrap“ ist für den Start-Token reserviert",
            ));
        }
        let raw = auth::new_api_token();
        let hash = auth::sha256_hex(&raw);
        let row = TokenRow {
            id: Uuid::now_v7(),
            user_id,
            name,
            created_at: now_ms(),
            last_used_at: None,
        };
        self.with_writer(move |conn| {
            let active = db::list_tokens(conn, user_id)?;
            if active.iter().any(|t| t.name == row.name) {
                return Err(AppError::Conflict(format!(
                    "Es gibt bereits einen Token mit dem Namen „{}“",
                    row.name
                )));
            }
            db::insert_token(conn, &row, &hash)?;
            Ok(CreatedToken {
                info: token_info(&row),
                token: raw,
            })
        })
        .await
    }

    /// Revokes a token of the user; `NotFound` if there is no such active
    /// token.
    pub async fn revoke_token(&self, user_id: Uuid, id: Uuid) -> AppResult<()> {
        self.with_writer(move |conn| {
            if db::revoke_token(conn, user_id, id, now_ms())? {
                Ok(())
            } else {
                Err(AppError::NotFound("Token nicht gefunden".into()))
            }
        })
        .await
    }

    // ----- maintenance ----------------------------------------------------------------

    /// Checks that the database answers.
    pub async fn health(&self) -> AppResult<()> {
        self.with_reader(|conn| {
            conn.query_row("SELECT 1", [], |_| Ok(()))?;
            Ok(())
        })
        .await
    }

    /// Writes a backup (`VACUUM INTO`) to `<data>/backups/lichtung-<local
    /// timestamp>.db` and keeps the newest [`BACKUPS_TO_KEEP`]. Also removes
    /// expired sessions, stale OAuth rows and old sync history (tombstones
    /// after 90 days, mutation results after 30, see [`crate::sync_api`]),
    /// whether or not the backup succeeds.
    pub async fn backup_now(&self) -> AppResult<PathBuf> {
        self.blocking(|inner| {
            let purged = (|| -> AppResult<(usize, usize, crate::sync_api::Purged)> {
                let mut conn = lock(&inner.writer);
                let now = now_ms();
                Ok((
                    db::purge_expired_sessions(&conn, now)?,
                    db::purge_oauth(
                        &conn,
                        now,
                        now - crate::oauth::STALE_CLIENT_MS,
                        now - crate::oauth::UNUSED_CLIENT_MS,
                    )?,
                    crate::sync_api::purge(&mut conn, now)?,
                ))
            })();
            match &purged {
                Ok((sessions, clients, history)) => tracing::info!(
                    purged_sessions = sessions,
                    purged_oauth_clients = clients,
                    purged_tombstones = history.tombstones,
                    purged_mutation_results = history.results,
                    "expired sessions, OAuth rows and old sync history removed"
                ),
                Err(e) => tracing::error!(error = %e, "removing expired sessions failed"),
            }
            let dir = inner.config.backup_dir();
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
            let conn = Connection::open(inner.config.db_path()).context("Datenbank öffnen")?;
            let dest = write_backup(&conn, &dir, &stamp)?;
            tracing::info!(path = %dest.display(), "backup written");
            Ok(dest)
        })
        .await
    }
}

/// Writes `lichtung-<stamp>.db` into `dir` and prunes old backups. The copy
/// is written under a temporary name and renamed only when complete, so a
/// failed `VACUUM INTO` (disk full, I/O error) never leaves a truncated
/// file that rotation would count as a backup.
fn write_backup(conn: &Connection, dir: &std::path::Path, stamp: &str) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir).context("Backup-Verzeichnis anlegen")?;
    let mut dest = dir.join(format!("lichtung-{stamp}.db"));
    let mut n = 1;
    while dest.exists() {
        dest = dir.join(format!("lichtung-{stamp}-{n}.db"));
        n += 1;
    }
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("lichtung");
    let tmp = dir.join(format!(".{name}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let written = db::vacuum_into(conn, &tmp)
        .and_then(|()| std::fs::rename(&tmp, &dest).context("Backup umbenennen"));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    prune_backups(dir, BACKUPS_TO_KEEP)?;
    Ok(dest)
}

fn token_info(row: &TokenRow) -> TokenInfo {
    TokenInfo {
        id: row.id,
        name: row.name.clone(),
        created_at: format_ms(row.created_at),
        last_used_at: row.last_used_at.map(format_ms),
    }
}

/// Deletes all but the `keep` newest `lichtung-*.db` files in `dir` (names
/// sort chronologically), and leftover temporary files of interrupted
/// backups.
fn prune_backups(dir: &std::path::Path, keep: usize) -> anyhow::Result<()> {
    let name_of = |p: &PathBuf| p.file_name().and_then(|n| n.to_str()).map(str::to_owned);
    let paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    for stale in paths.iter().filter(|p| {
        name_of(p).is_some_and(|n| n.starts_with(".lichtung-") && n.ends_with(".db.tmp"))
    }) {
        let _ = std::fs::remove_file(stale);
    }
    let mut files: Vec<PathBuf> = paths
        .into_iter()
        .filter(|p| name_of(p).is_some_and(|n| n.starts_with("lichtung-") && n.ends_with(".db")))
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(keep);
    for old in &files[..excess] {
        std::fs::remove_file(old)
            .with_context(|| format!("altes Backup {} löschen", old.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_milliseconds_as_rfc3339() {
        assert_eq!(format_ms(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_ms(1_792_756_800_123), "2026-10-23T12:00:00.123Z");
    }

    #[tokio::test]
    async fn backups_rotate_and_skip_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(Config::for_data_dir(dir.path().join("data")))
            .await
            .unwrap();
        let backups = app.config().backup_dir();
        std::fs::create_dir_all(&backups).unwrap();
        for i in 0..BACKUPS_TO_KEEP {
            std::fs::write(
                backups.join(format!("lichtung-20261001-0300{i:02}.db")),
                b"alt",
            )
            .unwrap();
        }
        std::fs::write(backups.join(".lichtung-20261002-030000.db.tmp"), b"halb").unwrap();
        std::fs::write(backups.join("fremd.txt"), b"bleibt").unwrap();

        let written = app.backup_now().await.unwrap();
        assert!(written.is_file());
        let check = Connection::open(&written).unwrap();
        let tables: i64 = check
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'devices'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1, "the backup is a complete database");

        let names: Vec<String> = std::fs::read_dir(&backups)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        let dbs = names
            .iter()
            .filter(|n| n.starts_with("lichtung-") && n.ends_with(".db"))
            .count();
        assert_eq!(dbs, BACKUPS_TO_KEEP);
        assert!(!names.iter().any(|n| n.ends_with(".tmp")), "{names:?}");
        assert!(names.contains(&"fremd.txt".to_owned()));
        assert!(
            !backups.join("lichtung-20261001-030000.db").exists(),
            "oldest pruned"
        );
    }
}
