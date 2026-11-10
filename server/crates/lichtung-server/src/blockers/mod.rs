//! Blockers (LIC-75): the server mirrors the events of one calendar as
//! busy "Blockiert" events into another, and keeps them in step.
//!
//! A **rule** names a source and a target calendar, the blocker title
//! (default [`DEFAULT_TITLE`]) and whether all-day events block too
//! (default no). Source and target are fixed; to change them, delete the
//! rule and create a new one. The target must be writable for Lichtung;
//! the source may be any calendar, ICS subscriptions included.
//!
//! **Blockers** carry only the times of their source ([`build`]): a series
//! becomes a series with the same rule, moved and cancelled occurrences
//! included. Nothing else of the source reaches the target calendar. The
//! blocker names its source in `X-LICHTUNG-BLOCKER-OF`; the event JSON
//! shows it as `blocker_of`, so the apps can hide it behind its source.
//!
//! **Reconciliation** ([`App::reconcile_blockers`]) runs after every
//! change (debounced by [`DEBOUNCE`]), when a rule changes and every
//! [`INTERVAL`]. For each rule it looks at the source events that have not
//! ended more than a day ago and
//! - creates the blocker if there is none (its UID is noted before the
//!   write, so a write whose answer got lost is found again),
//! - writes it again when the source, the rule or the blocker itself
//!   changed (a blocker edited or deleted elsewhere is put back: it always
//!   follows its source),
//! - deletes it when the source no longer blocks (cancelled, free,
//!   declined, all-day without `all_day`).
//!
//! Blockers whose source event or rule is gone are deleted; blockers of
//! past events stay as they are. Writes go through the normal write path
//! (origin `server`, [`crate::writes::ServerWrite`]) but record no
//! conflicts: a provider with a newer state wins and the next round
//! writes again. A write that fails is retried every round; the rule shows
//! the last error. Events that are blockers themselves are never mirrored,
//! so rules in both directions do not feed each other.

pub mod build;
pub mod store;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::accounts::store as accounts_store;
use crate::accounts::AccountKind;
use crate::app::{format_ms, now_ms, App};
use crate::error::{AppError, AppResult};
use crate::events::store as events_store;
use crate::events::EventContext;
use crate::writes::ServerWrite;
use build::{build_blocker, BlockerSpec};
use store::{BlockerRow, RuleRow, SourceRow};

/// Title of a blocker unless the rule names another.
pub const DEFAULT_TITLE: &str = "Blockiert";
pub const MAX_TITLE_CHARS: usize = 200;
/// Wait after a change before reconciling (changes come in bursts).
pub const DEBOUNCE: Duration = Duration::from_secs(2);
/// Reconcile at least this often (retries failed writes).
pub const INTERVAL: Duration = Duration::from_secs(10 * 60);
/// Sources that ended longer ago than this are left alone.
const PAST_SLACK_MS: i64 = 24 * 60 * 60 * 1000;

/// A rule as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockerRule {
    pub id: String,
    pub source_calendar_id: String,
    pub target_calendar_id: String,
    pub title: String,
    pub all_day: bool,
    /// Blockers written for this rule.
    pub blocker_count: i64,
    /// Blockers whose last write failed, and the newest reason.
    pub failed_count: i64,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// `POST /api/blocker-rules`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewBlockerRule {
    pub source_calendar_id: String,
    pub target_calendar_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub all_day: Option<bool>,
}

/// `PATCH /api/blocker-rules/{id}`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockerRulePatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub all_day: Option<bool>,
}

/// What one reconciliation did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockerReport {
    pub written: usize,
    pub deleted: usize,
    pub failed: usize,
}

fn not_found() -> AppError {
    AppError::NotFound("Blocker-Regel nicht gefunden".into())
}

fn check_title(raw: Option<String>) -> AppResult<String> {
    let title = raw.map(|t| t.trim().to_owned()).unwrap_or_default();
    if title.is_empty() {
        return Ok(DEFAULT_TITLE.to_owned());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(AppError::bad_request(format!(
            "title ist zu lang (höchstens {MAX_TITLE_CHARS} Zeichen)"
        )));
    }
    if title.chars().any(char::is_control) {
        return Err(AppError::bad_request(
            "title darf keine Steuerzeichen enthalten",
        ));
    }
    Ok(title)
}

/// Whether Lichtung can write into a calendar (as for openings).
fn writable(calendar: &accounts_store::CalendarRow, kind: Option<AccountKind>) -> bool {
    calendar.writable && !matches!(kind, None | Some(AccountKind::Ics | AccountKind::Microsoft))
}

fn rule_view(conn: &rusqlite::Connection, r: &RuleRow) -> rusqlite::Result<BlockerRule> {
    let (blocker_count, failed_count) = store::counts(conn, &r.id)?;
    Ok(BlockerRule {
        id: r.id.clone(),
        source_calendar_id: r.source_calendar_id.clone(),
        target_calendar_id: r.target_calendar_id.clone(),
        title: r.title.clone(),
        all_day: r.all_day,
        blocker_count,
        failed_count,
        last_error: store::last_error(conn, &r.id)?,
        created_at: format_ms(r.created_at),
        updated_at: format_ms(r.updated_at),
    })
}

/// What a round needs, read in one go.
struct Snapshot {
    rules: Vec<RuleRow>,
    /// Rules whose target Lichtung can write into.
    writable_targets: HashSet<String>,
    rows: HashMap<(String, String), BlockerRow>,
    /// Per rule, its possible sources.
    sources: HashMap<String, Vec<SourceRow>>,
    contexts: HashMap<String, EventContext>,
    /// Versions of the events the rows name (sources and blockers).
    versions: HashMap<String, i64>,
}

fn snapshot(conn: &rusqlite::Connection, since: i64) -> AppResult<Snapshot> {
    let rules = store::rules(conn)?;
    let mut writable_targets = HashSet::new();
    let mut sources = HashMap::new();
    let mut contexts = HashMap::new();
    for r in &rules {
        if let Some(cal) = accounts_store::calendar(conn, &r.target_calendar_id)? {
            let kind = accounts_store::account(conn, &cal.account_id)?.map(|a| a.kind);
            if writable(&cal, kind) {
                writable_targets.insert(r.id.clone());
            }
        }
        if !contexts.contains_key(&r.source_calendar_id) {
            let ctx = events_store::context_for_calendar(conn, &r.source_calendar_id)?;
            contexts.insert(r.source_calendar_id.clone(), ctx);
        }
        sources.insert(
            r.id.clone(),
            store::sources(conn, &r.source_calendar_id, since)?,
        );
    }
    let mut rows = HashMap::new();
    let mut versions = HashMap::new();
    for b in store::blockers(conn)? {
        for id in [Some(&b.source_event_id), b.event_id.as_ref()]
            .into_iter()
            .flatten()
        {
            if let Some(v) = store::event_version(conn, id)? {
                versions.insert(id.clone(), v);
            }
        }
        rows.insert((b.rule_id.clone(), b.source_event_id.clone()), b);
    }
    Ok(Snapshot {
        rules,
        writable_targets,
        rows,
        sources,
        contexts,
        versions,
    })
}

impl App {
    // ----- management (app) ------------------------------------------------------------

    pub async fn list_blocker_rules(&self) -> AppResult<Vec<BlockerRule>> {
        self.with_reader(|conn| {
            let rules = store::rules(conn)?;
            Ok(rules
                .iter()
                .map(|r| rule_view(conn, r))
                .collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    pub async fn blocker_rule(&self, id: &str) -> AppResult<BlockerRule> {
        let id = id.to_owned();
        self.with_reader(move |conn| {
            let r = store::rule(conn, &id)?.ok_or_else(not_found)?;
            Ok(rule_view(conn, &r)?)
        })
        .await
    }

    /// Creates a rule; the blockers follow in the next round (started at
    /// once).
    pub async fn create_blocker_rule(&self, input: NewBlockerRule) -> AppResult<BlockerRule> {
        let title = check_title(input.title)?;
        let (source, target) = (input.source_calendar_id, input.target_calendar_id);
        if source == target {
            return Err(AppError::bad_request(
                "Quell- und Zielkalender müssen verschieden sein",
            ));
        }
        let now = now_ms();
        let row = RuleRow {
            id: format!("blr_{}", Uuid::now_v7().simple()),
            source_calendar_id: source,
            target_calendar_id: target,
            title,
            all_day: input.all_day.unwrap_or(false),
            created_at: now,
            updated_at: now,
        };
        let stored = row.clone();
        let view = self
            .with_writer(move |conn| {
                let missing = |id: &str| AppError::BadRequest {
                    code: "calendar_not_found",
                    message: format!(
                        "Den Kalender „{id}“ gibt es nicht. Gültige IDs liefert GET /api/calendars."
                    ),
                };
                if accounts_store::calendar(conn, &stored.source_calendar_id)?.is_none() {
                    return Err(missing(&stored.source_calendar_id));
                }
                let target = accounts_store::calendar(conn, &stored.target_calendar_id)?
                    .ok_or_else(|| missing(&stored.target_calendar_id))?;
                let kind = accounts_store::account(conn, &target.account_id)?.map(|a| a.kind);
                if !writable(&target, kind) {
                    return Err(AppError::BadRequest {
                        code: "calendar_read_only",
                        message: format!(
                            "Der Kalender „{}“ ist nur lesbar. Blocker brauchen einen \
                             Zielkalender, in den Lichtung schreiben kann.",
                            target.name
                        ),
                    });
                }
                if store::rule_for(conn, &stored.source_calendar_id, &stored.target_calendar_id)?
                    .is_some()
                {
                    return Err(AppError::Conflict(
                        "Für diese beiden Kalender gibt es schon eine Blocker-Regel".into(),
                    ));
                }
                store::insert_rule(conn, &stored)?;
                Ok(rule_view(conn, &stored)?)
            })
            .await?;
        tracing::info!(rule = %row.id, "blocker rule created");
        self.wake_blockers();
        Ok(view)
    }

    /// Changes title or `all_day`; every blocker of the rule is written
    /// again in the next round.
    pub async fn update_blocker_rule(
        &self,
        id: &str,
        patch: BlockerRulePatch,
    ) -> AppResult<BlockerRule> {
        let title = match patch.title {
            Some(t) => Some(check_title(Some(t))?),
            None => None,
        };
        let id = id.to_owned();
        let view = self
            .with_writer(move |conn| {
                let mut row = store::rule(conn, &id)?.ok_or_else(not_found)?;
                let before = row.clone();
                if let Some(t) = title {
                    row.title = t;
                }
                if let Some(a) = patch.all_day {
                    row.all_day = a;
                }
                if row != before {
                    // Strictly increasing, so the blockers see the change.
                    row.updated_at = now_ms().max(before.updated_at + 1);
                    store::update_rule(conn, &row)?;
                }
                Ok(rule_view(conn, &row)?)
            })
            .await?;
        self.wake_blockers();
        Ok(view)
    }

    /// Deletes a rule; its blockers are removed in the next round.
    pub async fn delete_blocker_rule(&self, id: &str) -> AppResult<()> {
        let id = id.to_owned();
        let existed = self
            .with_writer(move |conn| Ok(store::delete_rule(conn, &id)?))
            .await?;
        if !existed {
            return Err(not_found());
        }
        self.wake_blockers();
        Ok(())
    }

    // ----- reconciliation --------------------------------------------------------------

    /// One round (module docs). Rounds never overlap.
    pub async fn reconcile_blockers(&self, now: DateTime<Utc>) -> AppResult<BlockerReport> {
        let _guard = self.blocker_guard().await;
        let since = now.timestamp_millis() - PAST_SLACK_MS;
        let snap = self.with_reader(move |conn| snapshot(conn, since)).await?;
        let mut report = BlockerReport::default();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        // Events that are blockers of some rule: never sources.
        let blocker_ids: HashSet<&String> = snap
            .rows
            .values()
            .filter_map(|b| b.event_id.as_ref())
            .collect();

        for rule in &snap.rules {
            let can_write = snap.writable_targets.contains(&rule.id);
            let ctx = snap
                .contexts
                .get(&rule.source_calendar_id)
                .cloned()
                .unwrap_or_default();
            for src in snap.sources.get(&rule.id).into_iter().flatten() {
                if blocker_ids.contains(&src.id) {
                    continue;
                }
                let key = (rule.id.clone(), src.id.clone());
                seen.insert(key.clone());
                let row = snap.rows.get(&key);
                if !can_write || row.is_some_and(|b| up_to_date(b, src, rule, &snap.versions)) {
                    continue;
                }
                let uid = row
                    .and_then(|b| b.uid.clone())
                    .unwrap_or_else(|| format!("{}@lichtung", Uuid::now_v7()));
                let spec = BlockerSpec {
                    uid: &uid,
                    title: &rule.title,
                    all_day: rule.all_day,
                    source_event_id: &src.id,
                };
                let desired = build_blocker(&src.ics, &ctx, &spec, now);
                self.apply_blocker(rule, src, row, uid, desired, &snap.versions, &mut report)
                    .await?;
            }
        }

        // Rows of gone rules and gone sources: remove the blocker, then the row.
        let rule_ids: HashSet<&String> = snap.rules.iter().map(|r| &r.id).collect();
        for (key, row) in &snap.rows {
            if seen.contains(key) {
                continue;
            }
            let gone = !rule_ids.contains(&row.rule_id)
                || !snap.versions.contains_key(&row.source_event_id);
            if !gone {
                continue; // a past source, or the target is not writable: left alone
            }
            match self.remove_blocker(row).await? {
                Some(ServerWrite::Deleted) => {
                    let (r, s) = key.clone();
                    self.with_writer(move |conn| Ok(store::delete_blocker(conn, &r, &s)?))
                        .await?;
                    report.deleted += 1;
                }
                Some(ServerWrite::Failed(message)) => {
                    self.note_blocker_failure(row, &message).await?;
                    report.failed += 1;
                }
                _ => {}
            }
        }
        if report != BlockerReport::default() {
            tracing::info!(
                written = report.written,
                deleted = report.deleted,
                failed = report.failed,
                "blockers reconciled"
            );
        }
        Ok(report)
    }

    /// Creates, rewrites or deletes the blocker of one source.
    #[allow(clippy::too_many_arguments)]
    async fn apply_blocker(
        &self,
        rule: &RuleRow,
        src: &SourceRow,
        row: Option<&BlockerRow>,
        uid: String,
        desired: Option<String>,
        versions: &HashMap<String, i64>,
        report: &mut BlockerReport,
    ) -> AppResult<()> {
        let mut next = BlockerRow {
            rule_id: rule.id.clone(),
            source_event_id: src.id.clone(),
            uid: None,
            event_id: None,
            source_version: src.version,
            rule_updated_at: rule.updated_at,
            blocker_version: None,
            attempts: 0,
            last_error: None,
        };
        let Some(ics) = desired else {
            // The source blocks nothing (any more).
            if let Some(b) = row.filter(|b| b.uid.is_some()) {
                match self.remove_blocker(b).await? {
                    Some(ServerWrite::Deleted) => report.deleted += 1,
                    Some(ServerWrite::Failed(message)) => {
                        self.note_blocker_failure(b, &message).await?;
                        report.failed += 1;
                        return Ok(());
                    }
                    _ => return Ok(()), // stale: next round
                }
            }
            return self.save_blocker_row(next).await;
        };
        // The stored blocker, if it still exists; else by its UID (an
        // earlier write whose answer got lost).
        let mut existing = row
            .and_then(|b| b.event_id.clone())
            .filter(|id| versions.contains_key(id));
        if existing.is_none() && row.is_some_and(|b| b.uid.is_some()) {
            let (cal, u) = (rule.target_calendar_id.clone(), uid.clone());
            existing = self
                .with_reader(move |conn| Ok(store::event_by_uid(conn, &cal, &u)?))
                .await?;
        }
        if row.is_none_or(|b| b.uid.is_none()) {
            // Note the UID before the write (module docs); version 0 keeps
            // the row out of date until the write is stored.
            self.save_blocker_row(BlockerRow {
                uid: Some(uid.clone()),
                source_version: 0,
                ..next.clone()
            })
            .await?;
        }
        let outcome = self
            .put_server_object(&rule.target_calendar_id, existing.as_deref(), &ics)
            .await?;
        match outcome {
            ServerWrite::Written(event_id) => {
                let id = event_id.clone();
                let version = self
                    .with_reader(move |conn| Ok(store::event_version(conn, &id)?))
                    .await?;
                next.uid = Some(uid);
                next.event_id = Some(event_id);
                next.blocker_version = version;
                self.save_blocker_row(next).await?;
                report.written += 1;
            }
            ServerWrite::Failed(message) => {
                let failing = BlockerRow {
                    uid: Some(uid),
                    ..next
                };
                self.note_blocker_failure(&failing, &message).await?;
                report.failed += 1;
            }
            ServerWrite::Stale | ServerWrite::Deleted => {}
        }
        Ok(())
    }

    /// Deletes the blocker of a row at the provider. `None`: there is none.
    async fn remove_blocker(&self, row: &BlockerRow) -> AppResult<Option<ServerWrite>> {
        let mut event_id = row.event_id.clone();
        if let (None, Some(uid)) = (&event_id, &row.uid) {
            // Created, but the answer got lost: find it by its UID.
            let (rule_id, uid) = (row.rule_id.clone(), uid.clone());
            event_id = self
                .with_reader(move |conn| {
                    let Some(rule) = store::rule(conn, &rule_id)? else {
                        return Ok(None);
                    };
                    Ok(store::event_by_uid(conn, &rule.target_calendar_id, &uid)?)
                })
                .await?;
        }
        match event_id {
            Some(id) => Ok(Some(self.delete_server_object(&id).await?)),
            None if row.uid.is_some() => Ok(Some(ServerWrite::Deleted)),
            None => Ok(None),
        }
    }

    async fn save_blocker_row(&self, row: BlockerRow) -> AppResult<()> {
        self.with_writer(move |conn| Ok(store::put_blocker(conn, &row, now_ms())?))
            .await
    }

    async fn note_blocker_failure(&self, row: &BlockerRow, message: &str) -> AppResult<()> {
        tracing::warn!(rule = %row.rule_id, source = %row.source_event_id, reason = %message,
            "blocker not written");
        let (row, message) = (row.clone(), message.to_owned());
        self.with_writer(move |conn| {
            let exists = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM blockers WHERE rule_id = ?1 AND source_event_id = ?2)",
                [&row.rule_id, &row.source_event_id],
                |r| r.get::<_, bool>(0),
            )?;
            if !exists {
                store::put_blocker(conn, &row, now_ms())?;
            }
            Ok(store::blocker_failed(conn, &row.rule_id, &row.source_event_id, &message)?)
        })
        .await
    }
}

/// The row mirrors this source, this rule and the stored blocker.
fn up_to_date(
    b: &BlockerRow,
    src: &SourceRow,
    rule: &RuleRow,
    versions: &HashMap<String, i64>,
) -> bool {
    if b.source_version != src.version || b.rule_updated_at != rule.updated_at {
        return false;
    }
    if b.last_error.is_some() {
        return false;
    }
    match (&b.uid, &b.event_id) {
        (None, _) => true,
        (Some(_), Some(id)) => versions
            .get(id)
            .is_some_and(|v| Some(*v) == b.blocker_version),
        (Some(_), None) => false,
    }
}

/// Reconciles at start, after changes ([`DEBOUNCE`]), when a rule changes
/// and every [`INTERVAL`], until the app shuts down.
pub async fn run_reconciler(app: App) {
    let shutdown = app.shutdown_token();
    let mut changes = app.subscribe_changes();
    loop {
        if let Err(e) = app.reconcile_blockers(Utc::now()).await {
            tracing::error!(error = %e, "blocker reconciliation failed");
        }
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = app.blocker_wakeup() => {}
            _ = tokio::time::sleep(INTERVAL) => {}
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(DEBOUNCE) => {}
                }
                // Changes during the wait are covered by this round.
                changes.borrow_and_update();
            }
        }
    }
}
