//! Sync API for the apps (LIC-11, `docs/api/sync-api.md` sections 4–6):
//! pull with cursor, push of mutations, cleanup of the change history.
//! SSE lives in [`crate::http::sync`] and forwards
//! [`App::subscribe_changes`].
//!
//! **Pull.** Every synced object carries the `seq` of its last change;
//! deletions leave tombstones ([`crate::changes`]). A page is all objects
//! and tombstones with `seq > since`, ordered by `seq`, read in one
//! transaction (a consistent snapshot). Each object appears once, in its
//! newest state, because it only has one row. The cursor of the last page
//! is the counter itself (also covers values whose object changed again).
//!
//! **Push.** Mutations run in order, one at a time
//! ([`App::write_guard`]). A mutation `id` that already has a final result
//! (applied, conflict, rejected) gets that result again without a second
//! write; results are kept [`RESULT_RETENTION_MS`]. After the first
//! `retry` every further mutation answers `retry` with `not_attempted`.
//!
//! **Cleanup** (nightly with the backup): tombstones older than
//! [`TOMBSTONE_RETENTION_MS`] and results older than
//! [`RESULT_RETENTION_MS`] are removed. A cursor below the highest removed
//! tombstone can no longer be served: `410 cursor_expired`.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

use crate::accounts::{store as accounts_store, Account, Calendar, CalendarSet};
use crate::app::{now_ms, App};
use crate::conflicts::{self, Conflict};
use crate::error::{AppError, AppResult};
use crate::events::store::{self as events_store};
use crate::events::{event_json, EventContext};
use crate::writes::Origin;

/// Default page size of a pull.
pub const DEFAULT_PULL_LIMIT: u32 = 500;
/// Largest page size of a pull.
pub const MAX_PULL_LIMIT: u32 = 2000;
/// Most mutations per push.
pub const MAX_MUTATIONS: usize = 100;
/// Longest mutation ID.
pub const MAX_MUTATION_ID_CHARS: usize = 200;
/// After this long, a push stops starting new mutations (the rest answer
/// `retry`/`not_attempted`); a provider call itself may take up to 30 s.
pub const PUSH_TIME_BUDGET: Duration = Duration::from_secs(25);
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Tombstones are kept this long (the contract promises at least 90 days).
pub const TOMBSTONE_RETENTION_MS: i64 = 90 * DAY_MS;
/// Mutation results are kept this long (contract: at least 30 days).
pub const RESULT_RETENTION_MS: i64 = 30 * DAY_MS;

/// One page of a pull.
#[derive(Debug, Clone, Serialize)]
pub struct PullPage {
    pub cursor: i64,
    pub has_more: bool,
    pub changes: Vec<Value>,
}

/// What the cleanup removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Purged {
    pub tombstones: usize,
    pub results: usize,
}

/// `(seq, type, id, deleted)` of everything changed after `since`.
fn changed_since(
    conn: &Connection,
    since: i64,
    limit: u32,
) -> rusqlite::Result<Vec<(i64, String, String, bool)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT seq, type, id, deleted FROM (
             SELECT seq, 'account' AS type, id, 0 AS deleted FROM accounts WHERE seq > ?1
             UNION ALL SELECT seq, 'calendar', id, 0 FROM calendars WHERE seq > ?1
             UNION ALL SELECT seq, 'calendar_set', id, 0 FROM calendar_sets WHERE seq > ?1
             UNION ALL SELECT seq, 'event', id, 0 FROM events WHERE seq > ?1
             UNION ALL SELECT seq, 'conflict', id, 0 FROM conflicts WHERE seq > ?1
             UNION ALL SELECT seq, type, id, 1 FROM tombstones WHERE seq > ?1
         ) ORDER BY seq LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![since, i64::from(limit)], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })?;
    rows.collect()
}

/// The full object of a change, `None` if it is gone.
fn object(
    conn: &Connection,
    kind: &str,
    id: &str,
    contexts: &mut HashMap<String, EventContext>,
) -> AppResult<Option<Value>> {
    let value = match kind {
        "account" => {
            accounts_store::account(conn, id)?.map(|r| serde_json::to_value(Account::from(&r)))
        }
        "calendar" => {
            accounts_store::calendar(conn, id)?.map(|r| serde_json::to_value(Calendar::from(&r)))
        }
        "calendar_set" => {
            accounts_store::set(conn, id)?.map(|r| serde_json::to_value(CalendarSet::from(&r)))
        }
        "conflict" => {
            conflicts::conflict(conn, id)?.map(|r| serde_json::to_value(Conflict::from(&r)))
        }
        "event" => match events_store::event(conn, id)? {
            Some(row) => {
                if !contexts.contains_key(&row.calendar_id) {
                    let ctx = events_store::context_for_calendar(conn, &row.calendar_id)?;
                    contexts.insert(row.calendar_id.clone(), ctx);
                }
                Some(Ok(event_json(&row, &contexts[&row.calendar_id])))
            }
            None => None,
        },
        _ => None,
    };
    value.transpose().map_err(|e| AppError::Internal(e.into()))
}

/// Current cursor and highest removed tombstone.
fn counter(conn: &Connection) -> rusqlite::Result<(i64, i64)> {
    conn.query_row(
        "SELECT seq, purged_through FROM change_counter WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

fn stored_result(conn: &Connection, id: &str) -> rusqlite::Result<Option<Value>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT result FROM mutation_results WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|r| serde_json::from_str(&r).ok()))
}

fn not_attempted(id: &str, message: &str) -> Value {
    json!({"id": id, "status": "retry",
           "error": {"code": "not_attempted", "message": message}})
}

impl App {
    /// One page of changes after `since` (see the module docs).
    pub async fn pull(&self, since: i64, limit: u32) -> AppResult<PullPage> {
        if since < 0 {
            return Err(AppError::bad_request("since darf nicht negativ sein"));
        }
        if limit == 0 {
            return Err(AppError::bad_request("limit muss mindestens 1 sein"));
        }
        let limit = limit.min(MAX_PULL_LIMIT);
        self.with_reader(move |conn| {
            // One snapshot for the whole page.
            let tx = conn.unchecked_transaction()?;
            let (current, purged_through) = counter(&tx)?;
            if since > current || (since > 0 && since < purged_through) {
                return Err(AppError::CursorExpired(
                    "Der Cursor ist zu alt (oder unbekannt). Bitte den Cache leeren und mit \
                     since=0 neu laden."
                        .into(),
                ));
            }
            let mut rows = changed_since(&tx, since, limit + 1)?;
            let has_more = rows.len() > limit as usize;
            rows.truncate(limit as usize);
            let cursor = match (has_more, rows.last()) {
                (true, Some((seq, ..))) => *seq,
                _ => current,
            };
            // Each object once: the last entry wins (rows are by seq).
            let mut seen = HashSet::new();
            let mut keep = vec![false; rows.len()];
            for (i, (_, kind, id, _)) in rows.iter().enumerate().rev() {
                keep[i] = seen.insert((kind.clone(), id.clone()));
            }
            let mut contexts = HashMap::new();
            let mut changes = Vec::with_capacity(rows.len());
            for ((_, kind, id, deleted), keep) in rows.iter().zip(keep) {
                if !keep {
                    continue;
                }
                if *deleted {
                    changes.push(json!({"type": kind, "op": "delete", "id": id}));
                    continue;
                }
                if let Some(data) = object(&tx, kind, id, &mut contexts)? {
                    changes.push(json!({"type": kind, "op": "upsert", "data": data}));
                }
            }
            Ok(PullPage {
                cursor,
                has_more,
                changes,
            })
        })
        .await
    }

    /// `POST /api/sync/push`: applies mutations in order and answers one
    /// result per mutation (see the module docs). Every mutation must be
    /// an object with a string `id`.
    pub async fn push(&self, origin: &Origin, mutations: Vec<Value>) -> AppResult<Vec<Value>> {
        if mutations.len() > MAX_MUTATIONS {
            return Err(AppError::PayloadTooLarge(format!(
                "Höchstens {MAX_MUTATIONS} Mutationen pro Aufruf, geschickt: {}",
                mutations.len()
            )));
        }
        let mut ids = Vec::with_capacity(mutations.len());
        for (i, m) in mutations.iter().enumerate() {
            let id = m
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty() && id.chars().count() <= MAX_MUTATION_ID_CHARS)
                .ok_or_else(|| {
                    AppError::bad_request(format!(
                        "mutations[{i}].id fehlt oder ist ungültig (String, 1 bis \
                         {MAX_MUTATION_ID_CHARS} Zeichen)"
                    ))
                })?;
            ids.push(id.to_owned());
        }
        let started = Instant::now();
        let mut stopped = false;
        let mut results = Vec::with_capacity(mutations.len());
        for (id, mutation) in ids.iter().zip(&mutations) {
            if stopped {
                results.push(not_attempted(
                    id,
                    "Nicht versucht, weil eine vorherige Änderung warten muss.",
                ));
                continue;
            }
            let _guard = self.write_guard().await;
            let stored = {
                let id = id.clone();
                self.with_reader(move |conn| Ok(stored_result(conn, &id)?))
                    .await?
            };
            if let Some(result) = stored {
                results.push(result);
                continue;
            }
            if started.elapsed() > PUSH_TIME_BUDGET {
                stopped = true;
                results.push(not_attempted(
                    id,
                    "Nicht versucht: Zeitlimit des Aufrufs erreicht. Bitte erneut senden.",
                ));
                continue;
            }
            let outcome = self.apply_mutation_locked(origin, mutation).await?;
            let result = outcome.to_json(id);
            if outcome.is_retry() {
                stopped = true;
            } else {
                let (id, label, stored) = (id.clone(), origin.label(), result.to_string());
                self.with_writer(move |conn| {
                    conn.execute(
                        "INSERT OR IGNORE INTO mutation_results (id, origin, result, created_at)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![id, label, stored, now_ms()],
                    )?;
                    Ok(())
                })
                .await?;
            }
            results.push(result);
        }
        Ok(results)
    }

    /// Removes tombstones older than [`TOMBSTONE_RETENTION_MS`] and
    /// mutation results older than [`RESULT_RETENTION_MS`] (relative to
    /// `now`, Unix ms).
    pub async fn purge_sync_history(&self, now: i64) -> AppResult<Purged> {
        self.with_writer(move |conn| Ok(purge(conn, now)?)).await
    }
}

/// See [`App::purge_sync_history`].
pub(crate) fn purge(conn: &mut Connection, now: i64) -> rusqlite::Result<Purged> {
    let tx = conn.transaction()?;
    let cutoff = now - TOMBSTONE_RETENTION_MS;
    let removed_through: Option<i64> = tx.query_row(
        "SELECT MAX(seq) FROM tombstones WHERE deleted_at < ?1",
        [cutoff],
        |r| r.get(0),
    )?;
    let tombstones = tx.execute("DELETE FROM tombstones WHERE deleted_at < ?1", [cutoff])?;
    if let Some(seq) = removed_through {
        tx.execute(
            "UPDATE change_counter SET purged_through = MAX(purged_through, ?1) WHERE id = 1",
            [seq],
        )?;
    }
    let results = tx.execute(
        "DELETE FROM mutation_results WHERE created_at < ?1",
        [now - RESULT_RETENTION_MS],
    )?;
    tx.commit()?;
    Ok(Purged {
        tombstones,
        results,
    })
}
