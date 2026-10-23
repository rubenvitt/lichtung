//! CalDAV sync engine (LIC-8): brings the stored events of every calendar
//! in line with the provider, and the scheduler that runs it.
//!
//! **One round per calendar** ([`App::sync_calendar`]):
//!
//! 1. *Listing.* With a stored sync token, `sync-collection` (RFC 6578)
//!    reports what changed. Without a token, after
//!    [`lichtung_caldav::Error::SyncTokenInvalid`], or when the server
//!    cannot do `sync-collection` (then remembered per calendar), a full
//!    comparison runs instead: ctag and sync token of the collection
//!    (`PROPFIND` depth 0, read *before* the listing), then every href with
//!    its ETag (`PROPFIND` depth 1); stored hrefs missing there are deleted.
//!    In ETag mode an unchanged ctag skips the listing.
//! 2. *Fetching.* New hrefs and hrefs whose ETag differs from the stored one
//!    are fetched with `calendar-multiget`. An href the server reports as
//!    missing (404) is deleted.
//! 3. *Applying,* in **one transaction**: upserts (new version and `seq`
//!    only if the original changed), deletions with tombstones, and only
//!    then the new sync token / ctag. Nothing is written before every
//!    remote call of the round has succeeded, so an aborted round (network,
//!    503, shutdown) leaves the old token in place and the next run repeats
//!    it: nothing lost, nothing doubled (rows are keyed by calendar + href,
//!    unchanged ETags are skipped).
//!
//! **Per account** ([`App::sync_account`]): discovery + calendar refresh
//! (vanished calendars delete their events with tombstones), then every
//! calendar. Status: `syncing` while running, then `ok` with
//! `last_sync_at`; `error` with a German message for temporary failures
//! (retried with backoff); `auth_failed` on 401, after which the scheduler
//! leaves the account alone until new credentials are stored.
//!
//! **Scheduler** ([`run_scheduler`]): at startup every account, then every
//! [`SYNC_INTERVAL`]; `POST /api/accounts/{id}/sync` runs one at once. At
//! most one run per account; runs end with the shutdown token; a failing
//! account does not hold up the others.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use lichtung_caldav::{Client, CollectionState, Error as DavError, MultigetResult, ResourceEtag};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::accounts::provider::{self, ProviderError};
use crate::accounts::store as accounts_store;
use crate::accounts::{self, AccountKind, RefreshReport, StatusState};
use crate::app::{now_ms, App};
use crate::error::{AppError, AppResult};
use crate::events::store::{self as events_store, Upsert};
use crate::events::{default_zone, derive, normalize_ics};

/// Regular sync interval per account.
pub const SYNC_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// First retry after a temporary failure; doubles per further failure.
pub const RETRY_BASE: Duration = Duration::from_secs(60);
/// Longest wait between retries.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60 * 60);
/// How often the scheduler looks for due accounts.
const SCHEDULER_TICK: Duration = Duration::from_secs(15);

/// What one calendar round changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalendarSyncReport {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
    /// Reported or fetched, but identical to what was stored.
    pub unchanged: usize,
    /// Stored although unreadable (`parse_error`).
    pub unreadable: usize,
    /// A full comparison ran (no or invalid token, no `sync-collection`).
    pub full: bool,
}

impl CalendarSyncReport {
    fn add(&mut self, other: &CalendarSyncReport) {
        self.added += other.added;
        self.updated += other.updated;
        self.deleted += other.deleted;
        self.unchanged += other.unchanged;
        self.unreadable += other.unreadable;
        self.full |= other.full;
    }
}

/// What a whole account run changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub calendars: RefreshReport,
    pub events: CalendarSyncReport,
}

/// A failed calendar round.
#[derive(Debug)]
pub enum SyncError {
    Provider(DavError),
    App(AppError),
}

impl From<DavError> for SyncError {
    fn from(e: DavError) -> Self {
        SyncError::Provider(e)
    }
}

impl From<AppError> for SyncError {
    fn from(e: AppError) -> Self {
        SyncError::App(e)
    }
}

/// Errors after which `sync-collection` (or the collection state) is taken
/// as not supported: the request was refused or not understood.
fn means_unsupported(e: &DavError) -> bool {
    matches!(
        e,
        DavError::Rejected { .. } | DavError::Forbidden { .. } | DavError::Protocol(_)
    )
}

/// Wait before the next attempt after `failures` failed runs in a row.
pub fn backoff(failures: u32) -> Duration {
    let factor = 1u32 << failures.saturating_sub(1).min(16);
    RETRY_BASE.saturating_mul(factor).min(MAX_BACKOFF)
}

/// Ends the "sync running" mark of an account when dropped (also when the
/// run is cancelled).
struct Claim {
    app: App,
    id: String,
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.app.end_sync(&self.id);
    }
}

/// Runs the sync of an account already marked with `begin_sync` in the
/// background; ends with the shutdown token.
pub(crate) fn spawn_claimed(
    app: App,
    id: String,
) -> tokio::task::JoinHandle<Option<AppResult<SyncReport>>> {
    tokio::spawn(async move {
        let _claim = Claim {
            app: app.clone(),
            id: id.clone(),
        };
        let shutdown = app.shutdown_token();
        let result = tokio::select! {
            r = app.run_account_sync(&id) => Some(r),
            () = shutdown.cancelled() => None,
        };
        if let Some(Err(e)) = &result {
            tracing::debug!(account = %id, error = %e, "background sync failed");
        }
        result
    })
}

impl App {
    /// Syncs an account now (calendars and events), unless a run for it is
    /// already going (`409 conflict`). See the module docs.
    pub async fn sync_account(&self, id: &str) -> AppResult<SyncReport> {
        if !self.begin_sync(id) {
            return Err(AppError::Conflict(
                "Für dieses Konto läuft bereits ein Abgleich".into(),
            ));
        }
        let _claim = Claim {
            app: self.clone(),
            id: id.to_owned(),
        };
        self.run_account_sync(id).await
    }

    async fn run_account_sync(&self, id: &str) -> AppResult<SyncReport> {
        let row = {
            let id = id.to_owned();
            self.with_reader(move |conn| {
                accounts_store::account(conn, &id)?
                    .ok_or_else(|| AppError::NotFound("Konto nicht gefunden".into()))
            })
            .await?
        };
        let kind = row.kind;
        let Some(credentials) = self.decrypt_credentials(&row) else {
            let message = "Zugangsdaten fehlen, bitte das Konto neu verbinden".to_owned();
            self.set_status(id, StatusState::AuthFailed, Some(message.clone()))
                .await?;
            return Err(AppError::InvalidCredentials(message));
        };
        self.set_status(id, StatusState::Syncing, None).await?;

        let fail = |e: ProviderError| async move {
            let (state, message) = e.status();
            tracing::warn!(account = %id, error = %e.error, "sync failed");
            self.set_status(id, state, Some(message)).await?;
            Err::<SyncReport, _>(e.into_app_error())
        };
        let client =
            match provider::client(kind, &row.base_url, &row.username, &credentials.password) {
                Ok(c) => c,
                Err(e) => return fail(e).await,
            };
        let remote = match provider::fetch_with(kind, &client).await {
            Ok(r) => r,
            Err(e) => return fail(e).await,
        };

        // Calendars (vanished ones delete their events with tombstones).
        let calendars_report = {
            let id = id.to_owned();
            self.with_change_tx(move |tx| {
                let mut row = accounts_store::account(tx, &id)?
                    .ok_or_else(|| AppError::NotFound("Konto nicht gefunden".into()))?;
                let report = accounts::reconcile(tx, &row, &remote.calendars, now_ms())?;
                let before = (
                    row.principal_url.clone(),
                    row.home_url.clone(),
                    row.auto_schedule,
                    row.user_addresses.clone(),
                );
                accounts::apply_remote_identity(&mut row, &remote);
                let after = (
                    row.principal_url.clone(),
                    row.home_url.clone(),
                    row.auto_schedule,
                    row.user_addresses.clone(),
                );
                if before != after {
                    row.updated_at = now_ms();
                    accounts_store::update_account(tx, &row)?;
                }
                Ok(report)
            })
            .await?
        };

        let calendars = {
            let id = id.to_owned();
            self.with_reader(move |conn| Ok(accounts_store::calendars(conn, Some(&id))?))
                .await?
        };
        let mut events = CalendarSyncReport::default();
        let mut first_error: Option<(String, DavError)> = None;
        for calendar in &calendars {
            match self.sync_calendar(&client, &calendar.id).await {
                Ok(r) => events.add(&r),
                Err(SyncError::Provider(DavError::Unauthorized)) => {
                    return fail(ProviderError {
                        kind,
                        error: DavError::Unauthorized,
                    })
                    .await;
                }
                Err(SyncError::Provider(e)) => {
                    tracing::warn!(
                        account = %id, calendar = %calendar.id, error = %e,
                        "calendar sync failed"
                    );
                    if first_error.is_none() {
                        first_error = Some((calendar.name.clone(), e));
                    }
                }
                Err(SyncError::App(e)) => {
                    self.set_status(
                        id,
                        StatusState::Error,
                        Some("Interner Fehler beim Abgleich, siehe Server-Log".into()),
                    )
                    .await?;
                    return Err(e);
                }
            }
        }

        let report = SyncReport {
            calendars: calendars_report,
            events,
        };
        let id_owned = id.to_owned();
        match first_error {
            None => {
                self.with_change_tx(move |tx| {
                    let mut row = accounts_store::account(tx, &id_owned)?
                        .ok_or_else(|| AppError::NotFound("Konto nicht gefunden".into()))?;
                    let now = now_ms();
                    row.status_state = StatusState::Ok;
                    row.status_message = None;
                    row.last_sync_at = Some(now);
                    row.updated_at = now;
                    accounts_store::update_account(tx, &row)?;
                    Ok(())
                })
                .await?;
                tracing::info!(
                    account = %id,
                    added = report.events.added,
                    updated = report.events.updated,
                    deleted = report.events.deleted,
                    "account synced"
                );
                Ok(report)
            }
            Some((calendar, error)) => {
                let detail = ProviderError { kind, error }.status().1;
                let message = format!("Kalender „{calendar}“: {detail}");
                self.set_status(id, StatusState::Error, Some(message.clone()))
                    .await?;
                Err(AppError::ProviderUnavailable(message))
            }
        }
    }

    /// One sync round of a calendar (see the module docs). Writes nothing
    /// unless every remote call succeeded.
    pub async fn sync_calendar(
        &self,
        client: &Client,
        calendar_id: &str,
    ) -> Result<CalendarSyncReport, SyncError> {
        let (href, state, stored) = {
            let calendar_id = calendar_id.to_owned();
            self.with_reader(move |conn| {
                let calendar = accounts_store::calendar(conn, &calendar_id)?
                    .ok_or_else(|| AppError::NotFound("Kalender nicht gefunden".into()))?;
                let state = events_store::sync_state(conn, &calendar_id)?.unwrap_or_default();
                let stored: HashMap<String, Option<String>> =
                    events_store::etags_of_calendar(conn, &calendar_id)?
                        .into_iter()
                        .map(|(href, _, etag)| (href, etag))
                        .collect();
                Ok((calendar.remote_href, state, stored))
            })
            .await?
        };
        let mut report = CalendarSyncReport::default();
        let mut new_state = state.clone();

        // 1. Listing.
        let delta = match (&state.sync_token, state.sync_unsupported) {
            (Some(token), false) => match client.sync_collection(&href, Some(token)).await {
                Ok(r) => Some(r),
                Err(DavError::SyncTokenInvalid) => {
                    tracing::info!(calendar = %calendar_id, "sync token invalid, full comparison");
                    None
                }
                Err(e) if means_unsupported(&e) => {
                    tracing::info!(
                        calendar = %calendar_id, error = %e,
                        "sync-collection not supported, comparing ETags"
                    );
                    new_state.sync_unsupported = true;
                    None
                }
                Err(e) => return Err(e.into()),
            },
            _ => None,
        };
        let (changed, deleted): (Vec<ResourceEtag>, Vec<String>) = match delta {
            Some(r) => {
                new_state.sync_token = Some(r.sync_token);
                (
                    r.changed,
                    r.deleted.into_iter().map(|u| u.to_string()).collect(),
                )
            }
            None => {
                report.full = true;
                let collection = match client.collection_state(&href).await {
                    Ok(s) => s,
                    Err(e) if means_unsupported(&e) => CollectionState::default(),
                    Err(e) => return Err(e.into()),
                };
                if state.sync_token.is_none()
                    && collection.ctag.is_some()
                    && collection.ctag == state.ctag
                {
                    // ETag mode and nothing changed since the last round.
                    report.unchanged = stored.len();
                    return Ok(report);
                }
                let listing = client.list_etags(&href).await?;
                let present: HashSet<String> = listing.iter().map(|r| r.href.to_string()).collect();
                let gone = stored
                    .keys()
                    .filter(|h| !present.contains(*h))
                    .cloned()
                    .collect();
                new_state.ctag = collection.ctag;
                new_state.sync_token = if new_state.sync_unsupported {
                    None
                } else {
                    collection.sync_token
                };
                (listing, gone)
            }
        };

        // 2. What to fetch: new hrefs and changed ETags.
        let mut to_fetch: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        for r in changed {
            let key = r.href.to_string();
            if !seen.insert(key.clone()) {
                continue;
            }
            match stored.get(&key) {
                Some(Some(etag)) if r.etag.as_deref() == Some(etag.as_str()) => {
                    report.unchanged += 1;
                }
                _ => to_fetch.push(key),
            }
        }
        let mut to_delete: Vec<String> = deleted
            .into_iter()
            .filter(|h| stored.contains_key(h))
            .collect();

        let fetched = if to_fetch.is_empty() {
            MultigetResult::default()
        } else {
            client.multiget(&href, &to_fetch).await?
        };
        for missing in &fetched.missing {
            let key = missing.to_string();
            if stored.contains_key(&key) && !to_delete.contains(&key) {
                to_delete.push(key);
            }
        }

        // 3. Apply everything in one transaction, the new state last.
        let zone = default_zone();
        let objects: Vec<(String, Option<String>, String)> = fetched
            .found
            .into_iter()
            .map(|o| (o.href.to_string(), o.etag, normalize_ics(&o.data)))
            .collect();
        let calendar_id = calendar_id.to_owned();
        let applied = self
            .with_change_tx(move |tx| {
                if events_store::sync_state(tx, &calendar_id)?.is_none() {
                    // The calendar vanished meanwhile.
                    return Ok(report);
                }
                let now = now_ms();
                for href in &to_delete {
                    if events_store::delete_by_href(tx, &calendar_id, href, now)?.is_some() {
                        report.deleted += 1;
                    }
                }
                for (href, etag, ics) in &objects {
                    let derived = derive(ics, zone);
                    let (what, _) = events_store::upsert(
                        tx,
                        &calendar_id,
                        href,
                        etag.as_deref(),
                        ics,
                        &derived,
                        now,
                    )?;
                    match what {
                        Upsert::Inserted => report.added += 1,
                        Upsert::Updated => report.updated += 1,
                        Upsert::EtagOnly | Upsert::Unchanged => report.unchanged += 1,
                    }
                    if derived.is_err() && matches!(what, Upsert::Inserted | Upsert::Updated) {
                        report.unreadable += 1;
                        tracing::warn!(calendar = %calendar_id, href = %href, "unreadable event stored");
                    }
                }
                events_store::set_sync_state(tx, &calendar_id, &new_state)?;
                Ok(report)
            })
            .await?;
        Ok(applied)
    }
}

/// Background scheduler, started with the server (see the module docs).
pub async fn run_scheduler(app: App) {
    let shutdown = app.shutdown_token();
    // Account → (next run, failures in a row). Missing = due now.
    let mut plan: HashMap<String, (Instant, u32)> = HashMap::new();
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<(String, bool)>();
    loop {
        match app
            .with_reader(|conn| Ok(accounts_store::accounts(conn)?))
            .await
        {
            Ok(accounts) => {
                let now = Instant::now();
                let ids: HashSet<String> = accounts.iter().map(|a| a.id.clone()).collect();
                plan.retain(|id, _| ids.contains(id));
                for account in accounts {
                    if account.status_state == StatusState::AuthFailed {
                        // Waits for new credentials (status leaves
                        // auth_failed); then due at once.
                        plan.remove(&account.id);
                        continue;
                    }
                    if matches!(account.kind, AccountKind::Ics | AccountKind::Microsoft) {
                        continue;
                    }
                    let due = plan.get(&account.id).is_none_or(|(at, _)| *at <= now);
                    if !due || !app.begin_sync(&account.id) {
                        continue;
                    }
                    let failures = plan.get(&account.id).map_or(0, |(_, f)| *f);
                    // Not due again before this run reports back.
                    plan.insert(account.id.clone(), (now + MAX_BACKOFF, failures));
                    let handle = spawn_claimed(app.clone(), account.id.clone());
                    let done = done_tx.clone();
                    let id = account.id.clone();
                    tokio::spawn(async move {
                        let ok = matches!(handle.await, Ok(Some(Ok(_))));
                        let _ = done.send((id, ok));
                    });
                }
            }
            Err(e) => tracing::error!(error = %e, "sync scheduler cannot read accounts"),
        }
        tokio::select! {
            () = shutdown.cancelled() => return,
            Some((id, ok)) = done_rx.recv() => {
                let failures = if ok { 0 } else { plan.get(&id).map_or(0, |(_, f)| *f) + 1 };
                let wait = if ok { SYNC_INTERVAL } else { backoff(failures) };
                plan.insert(id, (Instant::now() + wait, failures));
            }
            () = app.sync_wakeup() => {}
            () = tokio::time::sleep(SCHEDULER_TICK) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_an_hour() {
        assert_eq!(backoff(1), Duration::from_secs(60));
        assert_eq!(backoff(2), Duration::from_secs(120));
        assert_eq!(backoff(4), Duration::from_secs(480));
        assert_eq!(backoff(7), MAX_BACKOFF);
        assert_eq!(backoff(40), MAX_BACKOFF);
    }
}
