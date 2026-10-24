//! Calendar accounts, their calendars and calendar sets (sync API contract
//! `docs/api/sync-api.md`, sections 3 and 7).
//!
//! - Creating or reconnecting an account checks the credentials with the
//!   provider first (discovery + calendar listing) and stores them only when
//!   that worked, encrypted with [`crate::crypto`].
//! - [`App::refresh_calendars`] aligns the stored calendars with the
//!   provider: new ones are added with default settings, vanished ones are
//!   deleted (tombstone), names, colors and rights follow the provider. It is
//!   the building block for the regular sync.
//! - Every change takes a fresh `seq` and is published through
//!   [`crate::changes`].

pub mod provider;
pub mod store;

use std::collections::{HashMap, HashSet};

use anyhow::Context;
use rusqlite::Connection;
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;
use uuid::Uuid;

use crate::app::{format_ms, now_ms, App};
use crate::config::Config;
use crate::crypto::{self, Cipher, Secret, StoredCredentials};
use crate::db;
use crate::error::{AppError, AppResult};
use provider::{ProviderError, Remote, RemoteCalendar};
use store::{AccountRow, CalendarRow, SetRow};

/// Maximum length of account, calendar set names.
pub const MAX_NAME_CHARS: usize = 100;
/// Maximum length of a username.
pub const MAX_USERNAME_CHARS: usize = 320;
/// Maximum length of a password.
pub const MAX_PASSWORD_CHARS: usize = 1024;
/// Maximum length of a server URL.
pub const MAX_URL_CHARS: usize = 2000;
/// Highest allowed `position`.
pub const MAX_POSITION: i64 = 1_000_000;
/// Maximum number of calendars in a set.
pub const MAX_SET_CALENDARS: usize = 1000;

/// Colors for calendars without a provider color and without an override,
/// so `color` is always set.
const FALLBACK_COLORS: [&str; 8] = [
    "#3B82F6", "#10B981", "#F59E0B", "#EF4444", "#8B5CF6", "#EC4899", "#14B8A6", "#6366F1",
];

// ----- kinds and states -------------------------------------------------------------

/// Kind of a calendar account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccountKind {
    Fastmail,
    Icloud,
    /// Any CalDAV server with an explicit URL (self-hosted, Radicale in tests).
    Caldav,
    /// ICS subscription (LIC-14).
    Ics,
    /// Microsoft Graph (E4).
    Microsoft,
}

impl AccountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountKind::Fastmail => "fastmail",
            AccountKind::Icloud => "icloud",
            AccountKind::Caldav => "caldav",
            AccountKind::Ics => "ics",
            AccountKind::Microsoft => "microsoft",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "fastmail" => AccountKind::Fastmail,
            "icloud" => AccountKind::Icloud,
            "caldav" => AccountKind::Caldav,
            "ics" => AccountKind::Ics,
            "microsoft" => AccountKind::Microsoft,
            _ => return None,
        })
    }

    /// The provider as named in messages.
    pub fn provider_name(self) -> &'static str {
        match self {
            AccountKind::Fastmail => "Fastmail",
            AccountKind::Icloud => "iCloud",
            AccountKind::Caldav => "Der CalDAV-Server",
            AccountKind::Ics => "Der ICS-Server",
            AccountKind::Microsoft => "Microsoft",
        }
    }
}

/// `status.state` of an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusState {
    Ok,
    Syncing,
    Error,
    AuthFailed,
}

impl StatusState {
    pub fn as_str(self) -> &'static str {
        match self {
            StatusState::Ok => "ok",
            StatusState::Syncing => "syncing",
            StatusState::Error => "error",
            StatusState::AuthFailed => "auth_failed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "ok" => StatusState::Ok,
            "syncing" => StatusState::Syncing,
            "error" => StatusState::Error,
            "auth_failed" => StatusState::AuthFailed,
            _ => return None,
        })
    }
}

// ----- API objects ------------------------------------------------------------------

/// `Account` of the sync API. Never contains the password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Account {
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub username: String,
    /// Server URL; only for `kind: caldav` (addition to the contract).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub status: AccountStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountStatus {
    pub state: &'static str,
    pub message: Option<String>,
    pub last_sync_at: Option<String>,
}

impl From<&AccountRow> for Account {
    fn from(row: &AccountRow) -> Self {
        Account {
            id: row.id.clone(),
            kind: row.kind.as_str(),
            name: row.name.clone(),
            username: row.username.clone(),
            url: (row.kind == AccountKind::Caldav).then(|| row.base_url.clone()),
            status: AccountStatus {
                state: row.status_state.as_str(),
                message: row.status_message.clone(),
                last_sync_at: row.last_sync_at.map(format_ms),
            },
        }
    }
}

/// `Calendar` of the sync API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Calendar {
    pub id: String,
    pub account_id: String,
    pub name: String,
    /// Effective color: the override, else the provider color, else a
    /// fixed fallback.
    pub color: String,
    pub provider_color: Option<String>,
    pub writable: bool,
    pub can_invite: bool,
    pub hidden: bool,
    pub mcp_write: bool,
    pub openings_busy: bool,
    pub all_day_blocks: bool,
    pub position: i64,
}

impl From<&CalendarRow> for Calendar {
    fn from(row: &CalendarRow) -> Self {
        let color = row
            .color
            .clone()
            .or_else(|| row.provider_color.clone())
            .unwrap_or_else(|| fallback_color(&row.id).to_owned());
        Calendar {
            id: row.id.clone(),
            account_id: row.account_id.clone(),
            name: row.name.clone(),
            color,
            provider_color: row.provider_color.clone(),
            writable: row.writable,
            can_invite: row.can_invite,
            hidden: row.hidden,
            mcp_write: row.mcp_write,
            openings_busy: row.openings_busy,
            all_day_blocks: row.all_day_blocks,
            position: row.position,
        }
    }
}

fn fallback_color(id: &str) -> &'static str {
    let sum: usize = id.bytes().map(usize::from).sum();
    FALLBACK_COLORS[sum % FALLBACK_COLORS.len()]
}

/// `CalendarSet` of the sync API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalendarSet {
    pub id: String,
    pub name: String,
    pub calendar_ids: Vec<String>,
    pub position: i64,
}

impl From<&SetRow> for CalendarSet {
    fn from(row: &SetRow) -> Self {
        CalendarSet {
            id: row.id.clone(),
            name: row.name.clone(),
            calendar_ids: row.calendar_ids.clone(),
            position: row.position,
        }
    }
}

/// What a calendar refresh changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefreshReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

// ----- inputs -------------------------------------------------------------------------

/// `POST /api/accounts`. `Debug` redacts the password.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewAccount {
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<Secret>,
    /// Server URL for `kind: caldav` (and later `ics`).
    #[serde(default)]
    pub url: Option<String>,
}

/// `PATCH /api/accounts/{id}`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<Secret>,
    #[serde(default)]
    pub url: Option<String>,
}

/// `PATCH /api/calendars/{id}`. `color: null` returns to the provider color.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarPatch {
    #[serde(default, deserialize_with = "explicit_null")]
    pub color: Option<Option<String>>,
    #[serde(default)]
    pub hidden: Option<bool>,
    #[serde(default)]
    pub mcp_write: Option<bool>,
    #[serde(default)]
    pub openings_busy: Option<bool>,
    #[serde(default)]
    pub all_day_blocks: Option<bool>,
    #[serde(default)]
    pub position: Option<i64>,
}

/// `POST /api/calendar-sets`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewCalendarSet {
    pub name: String,
    #[serde(default)]
    pub calendar_ids: Vec<String>,
    #[serde(default)]
    pub position: Option<i64>,
}

/// `PATCH /api/calendar-sets/{id}`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarSetPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub calendar_ids: Option<Vec<String>>,
    #[serde(default)]
    pub position: Option<i64>,
}

/// Missing field → `None`, `null` → `Some(None)`, value → `Some(Some(v))`.
fn explicit_null<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

// ----- validation ---------------------------------------------------------------------

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::now_v7().simple())
}

pub(crate) fn validate_name(raw: &str, what: &str) -> AppResult<String> {
    let name = raw.trim().to_owned();
    let len = name.chars().count();
    if len == 0 || len > MAX_NAME_CHARS {
        return Err(AppError::bad_request(format!(
            "{what} muss 1 bis {MAX_NAME_CHARS} Zeichen lang sein"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(AppError::bad_request(format!(
            "{what} darf keine Steuerzeichen enthalten"
        )));
    }
    Ok(name)
}

fn validate_username(raw: Option<&str>) -> AppResult<String> {
    let username = raw.map(str::trim).unwrap_or_default();
    if username.is_empty() {
        return Err(AppError::bad_request("username fehlt"));
    }
    if username.chars().count() > MAX_USERNAME_CHARS || username.chars().any(char::is_control) {
        return Err(AppError::bad_request("username ist ungültig"));
    }
    Ok(username.to_owned())
}

fn validate_password(kind: AccountKind, password: Option<&Secret>) -> AppResult<Secret> {
    let Some(password) = password else {
        return Err(AppError::bad_request("password fehlt"));
    };
    // A generic CalDAV server may run without authentication (Radicale with
    // `auth = none`); provider app passwords are never empty.
    if password.expose().is_empty() && kind != AccountKind::Caldav {
        return Err(AppError::bad_request("password darf nicht leer sein"));
    }
    if password.expose().chars().count() > MAX_PASSWORD_CHARS {
        return Err(AppError::bad_request("password ist zu lang"));
    }
    Ok(password.clone())
}

/// A server URL given by the user: http(s), no credentials, no fragment.
fn validate_url(raw: &str) -> AppResult<String> {
    let raw = raw.trim();
    if raw.chars().count() > MAX_URL_CHARS {
        return Err(AppError::bad_request("url ist zu lang"));
    }
    let url = Url::parse(raw)
        .map_err(|_| AppError::bad_request(format!("url ist keine gültige URL: {raw}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::bad_request("url muss eine http(s)-URL sein"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::bad_request(
            "url darf keine Zugangsdaten enthalten (dafür username und password)",
        ));
    }
    if url.fragment().is_some() {
        return Err(AppError::bad_request("url darf kein #Fragment enthalten"));
    }
    Ok(url.to_string())
}

fn validate_position(position: i64) -> AppResult<i64> {
    if (0..=MAX_POSITION).contains(&position) {
        Ok(position)
    } else {
        Err(AppError::bad_request(format!(
            "position muss zwischen 0 und {MAX_POSITION} liegen"
        )))
    }
}

/// `#RRGGBB` (any case) → upper-case `#RRGGBB`.
pub fn validate_color(raw: &str) -> AppResult<String> {
    let hex = raw.strip_prefix('#').unwrap_or("");
    if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(format!("#{}", hex.to_ascii_uppercase()))
    } else {
        Err(AppError::bad_request(format!(
            "color muss die Form #RRGGBB haben (z. B. #3B82F6), nicht {raw:?}"
        )))
    }
}

fn unsupported_kind(kind: AccountKind) -> Option<AppError> {
    match kind {
        AccountKind::Microsoft => Some(AppError::bad_request(
            "Microsoft-Konten werden noch nicht unterstützt (kommt mit der Graph-Anbindung)",
        )),
        _ => None,
    }
}

fn not_found_account() -> AppError {
    AppError::NotFound("Konto nicht gefunden".into())
}

fn not_found_calendar() -> AppError {
    AppError::NotFound("Kalender nicht gefunden".into())
}

fn not_found_set() -> AppError {
    AppError::NotFound("Kalender-Set nicht gefunden".into())
}

fn duplicate_account() -> AppError {
    AppError::Conflict("Dieses Konto ist bereits verbunden".into())
}

// ----- reconciliation -----------------------------------------------------------------

/// `mcp_write` of a new calendar: Claude may write where Lichtung may
/// write, except in calendars of the work account (spec section 7: the
/// work calendar is locked for MCP by default). The work calendar comes as
/// an ICS feed until the Graph approval, then as a Microsoft account; both
/// kinds start locked, even if a provider reported them writable.
pub fn default_mcp_write(kind: AccountKind, writable: bool) -> bool {
    writable && !matches!(kind, AccountKind::Ics | AccountKind::Microsoft)
}

/// Brings the stored calendars of `account` in line with `remote`.
pub(crate) fn reconcile(
    conn: &Connection,
    account: &AccountRow,
    remote: &[RemoteCalendar],
    now: i64,
) -> rusqlite::Result<RefreshReport> {
    let mut report = RefreshReport::default();
    let mut existing: HashMap<String, CalendarRow> = store::calendars(conn, Some(&account.id))?
        .into_iter()
        .map(|c| (c.remote_href.clone(), c))
        .collect();
    let mut next_position = store::next_calendar_position(conn)?;
    for rc in remote {
        match existing.remove(&rc.href) {
            Some(mut row) => {
                if store::apply_remote(&mut row, rc) {
                    row.updated_at = now;
                    store::update_calendar(conn, &row)?;
                    report.updated += 1;
                }
            }
            None => {
                let row = CalendarRow {
                    id: new_id("cal"),
                    account_id: account.id.clone(),
                    remote_href: rc.href.clone(),
                    name: rc.name.clone(),
                    description: rc.description.clone(),
                    provider_color: rc.color.clone(),
                    color: None,
                    writable: rc.writable,
                    can_invite: rc.can_invite,
                    shared: rc.shared,
                    hidden: false,
                    mcp_write: default_mcp_write(account.kind, rc.writable),
                    openings_busy: true,
                    all_day_blocks: false,
                    position: next_position,
                    created_at: now,
                    updated_at: now,
                    seq: 0,
                };
                next_position += 1;
                store::insert_calendar(conn, &row)?;
                report.added += 1;
            }
        }
    }
    for gone in existing.values() {
        store::delete_calendar(conn, &gone.id, now)?;
        report.removed += 1;
    }
    Ok(report)
}

/// Takes over what discovery found (principal, home, scheduling, the
/// user's addresses); the status stays as it is.
pub(crate) fn apply_remote_identity(row: &mut AccountRow, remote: &Remote) {
    row.principal_url = Some(remote.principal.to_string());
    row.home_url = Some(remote.home.to_string());
    row.auto_schedule = remote.auto_schedule;
    row.user_addresses.clone_from(&remote.addresses);
}

fn apply_remote_to_account(row: &mut AccountRow, remote: &Remote, now: i64) {
    apply_remote_identity(row, remote);
    row.status_state = StatusState::Ok;
    row.status_message = None;
    row.last_sync_at = Some(now);
    row.updated_at = now;
}

// ----- service --------------------------------------------------------------------------

impl App {
    /// All accounts, oldest first.
    pub async fn list_accounts(&self) -> AppResult<Vec<Account>> {
        self.with_reader(|conn| Ok(store::accounts(conn)?.iter().map(Account::from).collect()))
            .await
    }

    pub async fn account(&self, id: &str) -> AppResult<Account> {
        let id = id.to_owned();
        self.with_reader(move |conn| {
            store::account(conn, &id)?
                .as_ref()
                .map(Account::from)
                .ok_or_else(not_found_account)
        })
        .await
    }

    fn provider_base_url(&self, kind: AccountKind, url: Option<&str>) -> AppResult<String> {
        match (kind, url) {
            (AccountKind::Caldav, Some(url)) => validate_url(url),
            (AccountKind::Caldav, None) => Err(AppError::bad_request(
                "url fehlt (Adresse des CalDAV-Servers)",
            )),
            (_, Some(_)) => Err(AppError::bad_request(format!(
                "url gibt es nur für kind caldav, nicht für {}",
                kind.as_str()
            ))),
            (AccountKind::Fastmail, None) => Ok(self.config().fastmail_url.to_string()),
            (AccountKind::Icloud, None) => Ok(self.config().icloud_url.to_string()),
            (other, None) => Err(unsupported_kind(other)
                .unwrap_or_else(|| AppError::bad_request("Kontoart nicht unterstützt"))),
        }
    }

    /// Creates an account: checks the credentials with the provider,
    /// stores them encrypted and adds the account's event calendars.
    pub async fn create_account(&self, input: NewAccount) -> AppResult<Account> {
        let kind = AccountKind::parse(input.kind.trim()).ok_or_else(|| {
            AppError::bad_request(format!(
                "Unbekannte Kontoart „{}“ (möglich: fastmail, icloud, caldav, ics)",
                input.kind
            ))
        })?;
        if kind == AccountKind::Ics {
            return self.create_ics_account(input).await;
        }
        if let Some(e) = unsupported_kind(kind) {
            return Err(e);
        }
        let base_url = self.provider_base_url(kind, input.url.as_deref())?;
        let username = validate_username(input.username.as_deref())?;
        let password = validate_password(kind, input.password.as_ref())?;
        let name = match input.name.as_deref() {
            Some(n) => validate_name(n, "Der Name eines Kontos")?,
            None => match kind {
                AccountKind::Caldav => Url::parse(&base_url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or_else(|| "CalDAV".into()),
                AccountKind::Icloud => "iCloud".into(),
                _ => "Fastmail".into(),
            },
        };

        {
            let (base_url, username) = (base_url.clone(), username.clone());
            let existing = self
                .with_reader(move |conn| Ok(store::find_account(conn, kind, &base_url, &username)?))
                .await?;
            if existing.is_some() {
                return Err(duplicate_account());
            }
        }

        let remote = provider::fetch(kind, &base_url, &username, &password)
            .await
            .map_err(ProviderError::into_app_error)?;

        let id = new_id("acc");
        let now = now_ms();
        let credentials = StoredCredentials {
            password,
            ..Default::default()
        }
        .seal(self.cipher(), &id);
        let mut row = AccountRow {
            id,
            kind,
            name,
            username,
            base_url,
            principal_url: None,
            home_url: None,
            auto_schedule: false,
            user_addresses: Vec::new(),
            credentials: Some(credentials),
            status_state: StatusState::Ok,
            status_message: None,
            last_sync_at: None,
            created_at: now,
            updated_at: now,
            seq: 0,
        };
        apply_remote_to_account(&mut row, &remote, now);
        let account = self
            .with_change_tx(move |tx| {
                if store::find_account(tx, kind, &row.base_url, &row.username)?.is_some() {
                    return Err(duplicate_account());
                }
                store::insert_account(tx, &row)?;
                let report = reconcile(tx, &row, &remote.calendars, now)?;
                tracing::info!(
                    account = %row.id,
                    kind = kind.as_str(),
                    calendars = report.added,
                    "account connected"
                );
                Ok(Account::from(&row))
            })
            .await?;
        // The scheduler runs the first sync right away.
        self.wake_sync_scheduler();
        Ok(account)
    }

    /// Renames an account or replaces its credentials (checked with the
    /// provider before they are stored).
    pub async fn update_account(&self, id: &str, patch: AccountPatch) -> AppResult<Account> {
        let id = id.to_owned();
        let current = {
            let id = id.clone();
            self.with_reader(move |conn| store::account(conn, &id)?.ok_or_else(not_found_account))
                .await?
        };
        let name = patch
            .name
            .as_deref()
            .map(|n| validate_name(n, "Der Name eines Kontos"))
            .transpose()?;
        let reconnect = patch.username.is_some() || patch.password.is_some() || patch.url.is_some();
        if !reconnect {
            let Some(name) = name else {
                return self.account(&id).await;
            };
            return self
                .with_change_tx(move |tx| {
                    let mut row = store::account(tx, &id)?.ok_or_else(not_found_account)?;
                    if row.name != name {
                        row.name = name;
                        row.updated_at = now_ms();
                        store::update_account(tx, &row)?;
                    }
                    Ok(Account::from(&row))
                })
                .await;
        }

        let kind = current.kind;
        if kind == AccountKind::Ics {
            return self.reconnect_ics_account(current, name, patch).await;
        }
        let base_url = match patch.url.as_deref() {
            Some(url) => self.provider_base_url(kind, Some(url))?,
            None => current.base_url.clone(),
        };
        let username = match patch.username.as_deref() {
            Some(u) => validate_username(Some(u))?,
            None => current.username.clone(),
        };
        let password = match patch.password.as_ref() {
            Some(p) => validate_password(kind, Some(p))?,
            None => self.decrypt_credentials(&current).map(|c| c.password).ok_or_else(|| {
                AppError::bad_request(
                    "Die gespeicherten Zugangsdaten sind nicht mehr lesbar, bitte password mitschicken",
                )
            })?,
        };
        {
            let (base_url, username, id) = (base_url.clone(), username.clone(), id.clone());
            let other = self
                .with_reader(move |conn| Ok(store::find_account(conn, kind, &base_url, &username)?))
                .await?;
            if other.is_some_and(|other| other != id) {
                return Err(duplicate_account());
            }
        }

        let remote = provider::fetch(kind, &base_url, &username, &password)
            .await
            .map_err(ProviderError::into_app_error)?;
        let credentials = StoredCredentials {
            password,
            ..Default::default()
        }
        .seal(self.cipher(), &id);
        let account = self
            .with_change_tx(move |tx| {
                let mut row = store::account(tx, &id)?.ok_or_else(not_found_account)?;
                if let Some(other) = store::find_account(tx, kind, &base_url, &username)? {
                    if other != id {
                        return Err(duplicate_account());
                    }
                }
                let now = now_ms();
                if let Some(name) = name {
                    row.name = name;
                }
                row.username = username;
                row.base_url = base_url;
                row.credentials = Some(credentials);
                apply_remote_to_account(&mut row, &remote, now);
                store::update_account(tx, &row)?;
                reconcile(tx, &row, &remote.calendars, now)?;
                tracing::info!(account = %row.id, "account credentials updated");
                Ok(Account::from(&row))
            })
            .await?;
        // An account on `auth_failed` is synced again once it has new
        // credentials.
        self.wake_sync_scheduler();
        Ok(account)
    }

    /// Removes an account with its calendars from Lichtung (nothing changes
    /// at the provider).
    pub async fn delete_account(&self, id: &str) -> AppResult<()> {
        let id = id.to_owned();
        self.with_change_tx(move |tx| {
            if store::delete_account(tx, &id, now_ms())? {
                tracing::info!(account = %id, "account removed");
                Ok(())
            } else {
                Err(not_found_account())
            }
        })
        .await
    }

    pub(crate) fn decrypt_credentials(&self, row: &AccountRow) -> Option<StoredCredentials> {
        let blob = row.credentials.as_deref()?;
        match StoredCredentials::open(self.cipher(), &row.id, blob) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::error!(account = %row.id, error = %e, "cannot decrypt credentials");
                None
            }
        }
    }

    pub(crate) async fn set_status(
        &self,
        id: &str,
        state: StatusState,
        message: Option<String>,
    ) -> AppResult<Account> {
        let id = id.to_owned();
        self.with_change_tx(move |tx| {
            let mut row = store::account(tx, &id)?.ok_or_else(not_found_account)?;
            if row.status_state != state || row.status_message != message {
                row.status_state = state;
                row.status_message = message;
                row.updated_at = now_ms();
                store::update_account(tx, &row)?;
            }
            Ok(Account::from(&row))
        })
        .await
    }

    /// Aligns the stored calendars of an account with the provider and
    /// updates the account status. Provider failures are stored in the
    /// status (`auth_failed` or `error`) and returned as errors.
    pub async fn refresh_calendars(&self, id: &str) -> AppResult<RefreshReport> {
        let row = {
            let id = id.to_owned();
            self.with_reader(move |conn| store::account(conn, &id)?.ok_or_else(not_found_account))
                .await?
        };
        if row.kind == AccountKind::Ics {
            // One calendar, refreshed with its events by the feed sync.
            return Ok(self.sync_account(id).await?.calendars);
        }
        let Some(credentials) = self.decrypt_credentials(&row) else {
            let message = "Zugangsdaten fehlen, bitte das Konto neu verbinden".to_owned();
            self.set_status(id, StatusState::AuthFailed, Some(message.clone()))
                .await?;
            return Err(AppError::InvalidCredentials(message));
        };
        let remote = match provider::fetch(
            row.kind,
            &row.base_url,
            &row.username,
            &credentials.password,
        )
        .await
        {
            Ok(remote) => remote,
            Err(e) => {
                let (state, message) = e.status();
                tracing::warn!(account = %row.id, error = %e.error, "calendar refresh failed");
                self.set_status(id, state, Some(message)).await?;
                return Err(e.into_app_error());
            }
        };
        let id = id.to_owned();
        self.with_change_tx(move |tx| {
            let mut row = store::account(tx, &id)?.ok_or_else(not_found_account)?;
            let now = now_ms();
            let report = reconcile(tx, &row, &remote.calendars, now)?;
            apply_remote_to_account(&mut row, &remote, now);
            store::update_account(tx, &row)?;
            Ok(report)
        })
        .await
    }

    /// `POST /api/accounts/{id}/sync`: marks the account as syncing and runs
    /// [`App::sync_account`] (calendars and events) in the background, once
    /// per account at a time. Also runs for accounts on `auth_failed` (the
    /// scheduler skips those). Returns the account as it is now.
    pub async fn request_sync(&self, id: &str) -> AppResult<Account> {
        let account = self.account(id).await?;
        if !self.begin_sync(id) {
            return Ok(account);
        }
        let account = match self.set_status(id, StatusState::Syncing, None).await {
            Ok(a) => a,
            Err(e) => {
                self.end_sync(id);
                return Err(e);
            }
        };
        crate::sync::spawn_claimed(self.clone(), id.to_owned());
        Ok(account)
    }

    /// Whether a background sync of the account is still running.
    pub fn sync_running(&self, id: &str) -> bool {
        self.is_syncing(id)
    }

    // ----- calendars ---------------------------------------------------------------

    pub async fn list_calendars(&self) -> AppResult<Vec<Calendar>> {
        self.with_reader(|conn| {
            Ok(store::calendars(conn, None)?
                .iter()
                .map(Calendar::from)
                .collect())
        })
        .await
    }

    /// Changes the settings of a calendar.
    pub async fn update_calendar(&self, id: &str, patch: CalendarPatch) -> AppResult<Calendar> {
        let color = match patch.color {
            Some(Some(c)) => Some(Some(validate_color(&c)?)),
            other => other,
        };
        let position = patch.position.map(validate_position).transpose()?;
        let id = id.to_owned();
        self.with_change_tx(move |tx| {
            let mut row = store::calendar(tx, &id)?.ok_or_else(not_found_calendar)?;
            let before = row.clone();
            if let Some(color) = color {
                row.color = color;
            }
            if let Some(v) = patch.hidden {
                row.hidden = v;
            }
            if let Some(v) = patch.mcp_write {
                row.mcp_write = v;
            }
            if let Some(v) = patch.openings_busy {
                row.openings_busy = v;
            }
            if let Some(v) = patch.all_day_blocks {
                row.all_day_blocks = v;
            }
            if let Some(v) = position {
                row.position = v;
            }
            if row != before {
                row.updated_at = now_ms();
                store::update_calendar(tx, &row)?;
            }
            Ok(Calendar::from(&row))
        })
        .await
    }

    // ----- calendar sets -------------------------------------------------------------

    pub async fn list_calendar_sets(&self) -> AppResult<Vec<CalendarSet>> {
        self.with_reader(|conn| Ok(store::sets(conn)?.iter().map(CalendarSet::from).collect()))
            .await
    }

    pub async fn create_calendar_set(&self, input: NewCalendarSet) -> AppResult<CalendarSet> {
        let name = validate_name(&input.name, "Der Name eines Kalender-Sets")?;
        let position = input.position.map(validate_position).transpose()?;
        let calendar_ids = input.calendar_ids;
        self.with_change_tx(move |tx| {
            check_calendar_ids(tx, &calendar_ids)?;
            let now = now_ms();
            let row = SetRow {
                id: new_id("set"),
                name,
                calendar_ids,
                position: match position {
                    Some(p) => p,
                    None => store::next_set_position(tx)?,
                },
                created_at: now,
                updated_at: now,
                seq: 0,
            };
            store::save_set(tx, &row)?;
            Ok(CalendarSet::from(&row))
        })
        .await
    }

    pub async fn update_calendar_set(
        &self,
        id: &str,
        patch: CalendarSetPatch,
    ) -> AppResult<CalendarSet> {
        let name = patch
            .name
            .as_deref()
            .map(|n| validate_name(n, "Der Name eines Kalender-Sets"))
            .transpose()?;
        let position = patch.position.map(validate_position).transpose()?;
        let id = id.to_owned();
        self.with_change_tx(move |tx| {
            let mut row = store::set(tx, &id)?.ok_or_else(not_found_set)?;
            let before = row.clone();
            if let Some(name) = name {
                row.name = name;
            }
            if let Some(ids) = patch.calendar_ids {
                check_calendar_ids(tx, &ids)?;
                row.calendar_ids = ids;
            }
            if let Some(p) = position {
                row.position = p;
            }
            if row != before {
                row.updated_at = now_ms();
                store::save_set(tx, &row)?;
            }
            Ok(CalendarSet::from(&row))
        })
        .await
    }

    pub async fn delete_calendar_set(&self, id: &str) -> AppResult<()> {
        let id = id.to_owned();
        self.with_change_tx(move |tx| {
            if store::delete_set(tx, &id, now_ms())? {
                Ok(())
            } else {
                Err(not_found_set())
            }
        })
        .await
    }
}

fn check_calendar_ids(conn: &Connection, ids: &[String]) -> AppResult<()> {
    if ids.len() > MAX_SET_CALENDARS {
        return Err(AppError::bad_request(format!(
            "Ein Kalender-Set hat höchstens {MAX_SET_CALENDARS} Kalender"
        )));
    }
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id.as_str()) {
            return Err(AppError::bad_request(format!(
                "Kalender {id} steht doppelt in calendar_ids"
            )));
        }
        if store::calendar(conn, id)?.is_none() {
            return Err(AppError::bad_request(format!(
                "Kalender {id} gibt es nicht"
            )));
        }
    }
    Ok(())
}

/// Resets syncing states left over from a previous run (the background
/// sync ended with the process). Called at startup.
pub fn reset_stale_sync_states(conn: &Connection) -> anyhow::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let stale: Vec<AccountRow> = store::accounts(&tx)?
        .into_iter()
        .filter(|a| a.status_state == StatusState::Syncing)
        .collect();
    for mut row in stale {
        row.status_state = StatusState::Ok;
        row.updated_at = now_ms();
        store::update_account(&tx, &row)?;
    }
    tx.commit()?;
    Ok(())
}

/// `lichtung-server reset-secret-key`: after the old `LICHTUNG_SECRET_KEY`
/// was lost, forgets all stored credentials (accounts, calendars and
/// settings stay, status `auth_failed`) and stores a key check value for
/// the configured key. Returns the number of accounts to reconnect. Run it
/// while the server is stopped.
pub fn reset_secret_key(config: &Config) -> anyhow::Result<usize> {
    let mut conn = db::open(&config.db_path())
        .with_context(|| format!("Datenbank {} öffnen", config.db_path().display()))?;
    let tx = conn.transaction()?;
    let now = now_ms();
    let mut count = 0;
    for mut row in store::accounts(&tx)? {
        row.credentials = None;
        row.status_state = StatusState::AuthFailed;
        row.status_message =
            Some("Der Server-Schlüssel wurde ersetzt, bitte das Konto neu verbinden".to_owned());
        row.updated_at = now;
        store::update_account(&tx, &row)?;
        count += 1;
    }
    crypto::write_key_check(&tx, &Cipher::new(&config.secret_key))?;
    tx.commit()?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_account_kinds_start_locked_for_mcp() {
        for kind in [
            AccountKind::Fastmail,
            AccountKind::Icloud,
            AccountKind::Caldav,
        ] {
            assert!(default_mcp_write(kind, true), "{kind:?}");
            assert!(!default_mcp_write(kind, false), "{kind:?}");
        }
        for kind in [AccountKind::Ics, AccountKind::Microsoft] {
            assert!(!default_mcp_write(kind, true), "{kind:?}");
            assert!(!default_mcp_write(kind, false), "{kind:?}");
        }
    }

    #[test]
    fn reconcile_locks_new_ics_calendars_for_mcp() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&mut conn).unwrap();
        let account = |kind: AccountKind, id: &str| AccountRow {
            id: id.to_owned(),
            kind,
            name: "Beispiel".into(),
            username: "anna@example.org".into(),
            base_url: format!("https://{id}.example.org/"),
            principal_url: None,
            home_url: None,
            auto_schedule: false,
            user_addresses: Vec::new(),
            credentials: None,
            status_state: StatusState::Ok,
            status_message: None,
            last_sync_at: None,
            created_at: 1,
            updated_at: 1,
            seq: 0,
        };
        let remote = |href: &str| RemoteCalendar {
            href: href.to_owned(),
            name: "Kalender".into(),
            description: None,
            color: None,
            // Even a feed that claims write access starts locked.
            writable: true,
            shared: false,
            can_invite: false,
        };
        for (kind, id, expected) in [
            (AccountKind::Ics, "acc_ics", false),
            (AccountKind::Microsoft, "acc_ms", false),
            (AccountKind::Caldav, "acc_dav", true),
        ] {
            let a = account(kind, id);
            store::insert_account(&conn, &a).unwrap();
            reconcile(&conn, &a, &[remote(&format!("/{id}/"))], 1).unwrap();
            let cals = store::calendars(&conn, Some(id)).unwrap();
            assert_eq!(cals.len(), 1);
            assert_eq!(cals[0].mcp_write, expected, "{kind:?}");
        }
    }

    #[test]
    fn colors_are_validated_and_normalized() {
        assert_eq!(validate_color("#3b82f6").unwrap(), "#3B82F6");
        assert_eq!(validate_color("#3B82F6").unwrap(), "#3B82F6");
        for bad in [
            "3B82F6",
            "#3B82F",
            "#3B82F6FF",
            "#GGGGGG",
            "blau",
            "",
            "#abc",
        ] {
            assert!(validate_color(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn urls_are_validated() {
        assert_eq!(
            validate_url(" https://dav.example.org/dav ").unwrap(),
            "https://dav.example.org/dav"
        );
        for bad in [
            "ftp://dav.example.org/",
            "https://anna:geheim@dav.example.org/",
            "https://dav.example.org/#x",
            "kein url",
        ] {
            assert!(validate_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn ids_have_prefix_and_32_hex_characters() {
        let id = new_id("acc");
        assert!(id.starts_with("acc_"));
        assert_eq!(id.len(), 4 + 32);
        assert!(id[4..].bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn new_account_debug_hides_the_password() {
        let input: NewAccount = serde_json::from_value(serde_json::json!({
            "kind": "fastmail", "username": "anna@example.org", "password": "app-passwort-1234"
        }))
        .unwrap();
        let debug = format!("{input:?}");
        assert!(!debug.contains("app-passwort"), "{debug}");
    }

    #[test]
    fn calendar_patch_distinguishes_null_from_missing() {
        let p: CalendarPatch = serde_json::from_str(r#"{"color": null}"#).unwrap();
        assert_eq!(p.color, Some(None));
        let p: CalendarPatch = serde_json::from_str(r#"{"hidden": true}"#).unwrap();
        assert_eq!(p.color, None);
        let p: CalendarPatch = serde_json::from_str(r##"{"color": "#112233"}"##).unwrap();
        assert_eq!(p.color, Some(Some("#112233".into())));
        assert!(serde_json::from_str::<CalendarPatch>(r#"{"name": "x"}"#).is_err());
    }
}
