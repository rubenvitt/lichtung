//! Writing events to the provider (LIC-10): create, update, delete and
//! answer invitations, shared by the sync API (`POST /api/sync/push`) and
//! MCP (LIC-12). Every write names its origin ([`Origin`]).
//!
//! **Procedure.** The stored iCalendar original is the starting point;
//! [`lichtung_ical::edit_event`] / [`lichtung_ical::delete_event`] apply the
//! change with its scope, so properties Lichtung does not model survive.
//! The result goes to the provider with `If-Match: <stored ETag>` (new
//! objects with `If-None-Match: *` at `<calendar>/<uid>.ics`); afterwards
//! the server stores what the provider has: the sent data with the ETag of
//! the response, or (no ETag, or the server may have changed the data)
//! the object fetched again.
//!
//! **Conflicts: the provider wins.** A conflict exists when `base_version`
//! is not the stored `version` or the provider answers 412 (changed) or
//! 404 (gone). Then the object is fetched from the provider at once and
//! stored (new `version`), the change is *not* applied, and a
//! [`crate::conflicts`] entry keeps it visible until dismissed. Nothing
//! disappears silently.
//!
//! **Several objects.** "This and following" splits a series: the new
//! series is created first, then the original is shortened; if that fails,
//! the new series is deleted again. Moving to another calendar (scope
//! `all`) creates the event in the target and then deletes it in the
//! source; a failed deletion is reported as a conflict. Servers that refuse
//! the same UID twice in one calendar home (`no-uid-conflict`) get the
//! other order: delete in the source, create in the target, and restore
//! the source if that fails.
//!
//! **Rights.** Calendars that are not `writable` (shared read-only, ICS
//! subscriptions) refuse with `calendar_read_only`; for origin `mcp:` the
//! calendar setting `mcp_write` must also allow it.
//!
//! **Provider errors.** Unreachable or rate limited → `retry`
//! (`provider_unavailable`, `retry_after_seconds`); refused (4xx) →
//! `rejected` (`provider_rejected`); credentials refused → `retry` and the
//! account goes to `auth_failed`.
//!
//! Writes of one calendar never overlap with its sync round
//! ([`App::lock_calendars`]), and mutations run one at a time
//! ([`App::write_guard`]).

pub mod input;

use chrono::Utc;
use lichtung_caldav::{Client, Error as DavError, Precondition, PutResult};
use lichtung_ical::{
    delete_event, edit_event, CalTime, CalendarObject, EditContext, EditOutcome, EditScope,
    Error as IcalError, EventChanges,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::accounts::store::{self as accounts_store, AccountRow, CalendarRow};
use crate::accounts::{provider, AccountKind, StatusState};
use crate::app::{now_ms, App};
use crate::auth::{Principal, PrincipalKind, BOOTSTRAP_TOKEN_NAME};
use crate::conflicts::{self, Conflict, ConflictRow};
use crate::error::{AppError, AppResult};
use crate::events::store::{self as events_store, EventRow};
use crate::events::{default_zone, derive, event_json, normalize_ics, EventContext};
use input::{
    build_object, check_object, check_uid, describe, edit_changes, resolve, set_alarms,
    set_attendees, set_recurrence, DateInput, EventInput, Fields, InputError, Mutation,
    ResponseStatus, Scope,
};

/// Wait suggested to the client when the provider gave none.
pub const DEFAULT_RETRY_AFTER_SECS: u64 = 60;

/// Where a write comes from (`origin` of a conflict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A device of the app: `app:<device>`.
    App(String),
    /// A browser session: `web`.
    Web,
    /// An API token: `api:<token>`.
    Api(String),
    /// MCP (LIC-12): `mcp:<token or client>`; subject to `mcp_write`.
    Mcp(String),
    /// The server itself.
    Server,
}

impl Origin {
    pub fn label(&self) -> String {
        match self {
            Origin::App(name) => format!("app:{name}"),
            Origin::Web => "web".to_owned(),
            Origin::Api(name) => format!("api:{name}"),
            Origin::Mcp(name) => format!("mcp:{name}"),
            Origin::Server => "server".to_owned(),
        }
    }

    /// The origin of a request to `/api`.
    pub fn of(principal: &Principal) -> Origin {
        match &principal.kind {
            PrincipalKind::Session | PrincipalKind::DevAuto => Origin::Web,
            PrincipalKind::Device { name, .. } => Origin::App(name.clone()),
            PrincipalKind::Token { name, .. } => Origin::Api(name.clone()),
            PrincipalKind::Bootstrap => Origin::Api(BOOTSTRAP_TOKEN_NAME.to_owned()),
            PrincipalKind::OAuth { client_name, .. } => Origin::Mcp(client_name.clone()),
        }
    }

    fn is_mcp(&self) -> bool {
        matches!(self, Origin::Mcp(_))
    }
}

/// The result of one write (contract, section 6, "Antwort").
#[derive(Debug, Clone, PartialEq)]
pub enum WriteOutcome {
    /// Written to the provider. `events`: new state of the touched events,
    /// `deleted_event_ids`: removed ones.
    Applied {
        events: Vec<Value>,
        deleted_event_ids: Vec<String>,
    },
    /// The provider wins; `events` is the current state (empty if gone).
    Conflict {
        conflict: Conflict,
        events: Vec<Value>,
    },
    /// Permanently invalid.
    Rejected { code: &'static str, message: String },
    /// Temporarily impossible; send again later.
    Retry {
        code: &'static str,
        message: String,
        retry_after_seconds: Option<u64>,
    },
}

impl WriteOutcome {
    pub fn rejected(code: &'static str, message: impl Into<String>) -> Self {
        WriteOutcome::Rejected {
            code,
            message: message.into(),
        }
    }

    pub fn is_retry(&self) -> bool {
        matches!(self, WriteOutcome::Retry { .. })
    }

    pub fn status(&self) -> &'static str {
        match self {
            WriteOutcome::Applied { .. } => "applied",
            WriteOutcome::Conflict { .. } => "conflict",
            WriteOutcome::Rejected { .. } => "rejected",
            WriteOutcome::Retry { .. } => "retry",
        }
    }

    /// The result object of a mutation with this `id`.
    pub fn to_json(&self, id: &str) -> Value {
        match self {
            WriteOutcome::Applied {
                events,
                deleted_event_ids,
            } => json!({"id": id, "status": "applied", "events": events,
                        "deleted_event_ids": deleted_event_ids}),
            WriteOutcome::Conflict { conflict, events } => {
                json!({"id": id, "status": "conflict", "conflict": conflict, "events": events})
            }
            WriteOutcome::Rejected { code, message } => {
                json!({"id": id, "status": "rejected", "error": {"code": code, "message": message}})
            }
            WriteOutcome::Retry {
                code,
                message,
                retry_after_seconds,
            } => {
                let mut v = json!({"id": id, "status": "retry",
                                   "error": {"code": code, "message": message}});
                if let Some(s) = retry_after_seconds {
                    v["retry_after_seconds"] = json!(s);
                }
                v
            }
        }
    }
}

impl From<InputError> for WriteOutcome {
    fn from(e: InputError) -> Self {
        WriteOutcome::Rejected {
            code: e.code,
            message: e.message,
        }
    }
}

/// Why a write stopped early: an outcome for the client, or an internal
/// error (database).
enum Stop {
    Outcome(Box<WriteOutcome>),
    Internal(AppError),
}

impl From<WriteOutcome> for Stop {
    fn from(o: WriteOutcome) -> Self {
        Stop::Outcome(Box::new(o))
    }
}

impl From<InputError> for Stop {
    fn from(e: InputError) -> Self {
        Stop::Outcome(Box::new(e.into()))
    }
}

impl From<AppError> for Stop {
    fn from(e: AppError) -> Self {
        Stop::Internal(e)
    }
}

impl From<rusqlite::Error> for Stop {
    fn from(e: rusqlite::Error) -> Self {
        Stop::Internal(e.into())
    }
}

type Step<T> = Result<T, Stop>;

fn finish(result: Step<WriteOutcome>) -> AppResult<WriteOutcome> {
    match result {
        Ok(o) => Ok(o),
        Err(Stop::Outcome(o)) => Ok(*o),
        Err(Stop::Internal(e)) => Err(e),
    }
}

/// What a write is about: origin and the mutation as sent (kept in a
/// conflict).
#[derive(Debug, Clone)]
pub struct WriteContext {
    pub origin: Origin,
    pub mutation: Value,
}

/// What a successful PUT left at the provider.
struct Written {
    /// Normalized URL of the resource.
    href: String,
    etag: Option<String>,
    /// The ETag belongs to exactly the sent data (it came with the PUT
    /// response); otherwise the object is fetched again before storing.
    exact: bool,
    /// The data sent (or found).
    data: String,
}

impl Written {
    fn from_put(put: PutResult, data: String) -> Self {
        Written {
            href: put.href.to_string(),
            exact: put.etag_from_response && put.etag.is_some(),
            etag: put.etag,
            data,
        }
    }
}

/// A calendar with its account.
struct Target {
    calendar: CalendarRow,
    account: AccountRow,
}

/// "bei Fastmail", "auf dem CalDAV-Server" …
fn at_provider(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Fastmail => "bei Fastmail",
        AccountKind::Icloud => "bei iCloud",
        AccountKind::Caldav => "auf dem CalDAV-Server",
        AccountKind::Ics => "im ICS-Abo",
        AccountKind::Microsoft => "bei Microsoft",
    }
}

fn provider_name(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Caldav => "Der CalDAV-Server",
        other => other.provider_name(),
    }
}

/// The title of a stored original (for messages).
fn title_of(ics: &str) -> String {
    CalendarObject::parse(ics)
        .ok()
        .and_then(|o| {
            o.master()
                .or_else(|| o.overrides().into_iter().next())
                .and_then(|e| e.summary())
        })
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "ohne Titel".to_owned())
}

fn ical_error(e: IcalError) -> InputError {
    match e {
        IcalError::UnknownOccurrence(_) => InputError {
            code: "occurrence_not_found",
            message: format!("Das Vorkommen gibt es in dieser Serie nicht ({e})"),
        },
        IcalError::InvalidEdit(_) => InputError::scope(e.to_string()),
        other => InputError::event(other.to_string()),
    }
}

fn occurrence_of(occurrence: &Option<DateInput>) -> Step<Option<CalTime>> {
    Ok(occurrence
        .as_ref()
        .map(|d| d.to_caltime("occurrence"))
        .transpose()?)
}

/// What the provider said on a failed write, as a short German phrase.
fn short_reason(e: &DavError) -> String {
    match e {
        DavError::Rejected { status, body } if body.is_empty() => format!("HTTP {status}"),
        DavError::Rejected { status, body } => format!("HTTP {status}: {body}"),
        DavError::Forbidden { detail } if detail.is_empty() => "keine Berechtigung".to_owned(),
        DavError::Forbidden { detail } => format!("keine Berechtigung: {detail}"),
        other => other.to_string(),
    }
}

/// The provider refuses the same UID twice in one calendar home.
fn is_uid_conflict(e: &DavError) -> bool {
    let text = match e {
        DavError::Rejected { body, .. } => body,
        DavError::Forbidden { detail } => detail,
        _ => return false,
    };
    text.to_ascii_lowercase().contains("uid-conflict")
}

impl App {
    // ----- public entry points -------------------------------------------------------

    /// Applies one mutation of the sync API (`id` aside), one at a time.
    pub async fn apply_mutation(
        &self,
        origin: &Origin,
        mutation: &Value,
    ) -> AppResult<WriteOutcome> {
        let _guard = self.write_guard().await;
        self.apply_mutation_locked(origin, mutation).await
    }

    /// [`App::apply_mutation`] for a caller that holds [`App::write_guard`].
    pub(crate) async fn apply_mutation_locked(
        &self,
        origin: &Origin,
        mutation: &Value,
    ) -> AppResult<WriteOutcome> {
        let parsed = match Mutation::parse(mutation) {
            Ok(m) => m,
            Err(e) => return Ok(e.into()),
        };
        let ctx = WriteContext {
            origin: origin.clone(),
            mutation: mutation.clone(),
        };
        let result = match &parsed {
            Mutation::CreateEvent { calendar_id, event } => {
                self.do_create(&ctx, calendar_id, event).await
            }
            Mutation::UpdateEvent {
                event_id,
                base_version,
                scope,
                occurrence,
                changes,
            } => {
                self.do_update(&ctx, event_id, *base_version, *scope, occurrence, changes)
                    .await
            }
            Mutation::DeleteEvent {
                event_id,
                base_version,
                scope,
                occurrence,
            } => {
                self.do_delete(&ctx, event_id, *base_version, *scope, occurrence)
                    .await
            }
            Mutation::Respond {
                event_id,
                occurrence,
                status,
                comment,
            } => {
                self.do_respond(&ctx, event_id, occurrence, *status, comment.as_deref())
                    .await
            }
        };
        finish(result)
    }

    /// Creates an event (`create_event`).
    pub async fn create_event(
        &self,
        ctx: &WriteContext,
        calendar_id: &str,
        event: &EventInput,
    ) -> AppResult<WriteOutcome> {
        let _guard = self.write_guard().await;
        finish(self.do_create(ctx, calendar_id, event).await)
    }

    /// Changes an event (`update_event`).
    pub async fn update_event(
        &self,
        ctx: &WriteContext,
        event_id: &str,
        base_version: i64,
        scope: Scope,
        occurrence: &Option<DateInput>,
        changes: &EventInput,
    ) -> AppResult<WriteOutcome> {
        let _guard = self.write_guard().await;
        finish(
            self.do_update(ctx, event_id, base_version, scope, occurrence, changes)
                .await,
        )
    }

    /// Deletes an event or occurrences of it (`delete_event`).
    pub async fn delete_event(
        &self,
        ctx: &WriteContext,
        event_id: &str,
        base_version: i64,
        scope: Scope,
        occurrence: &Option<DateInput>,
    ) -> AppResult<WriteOutcome> {
        let _guard = self.write_guard().await;
        finish(
            self.do_delete(ctx, event_id, base_version, scope, occurrence)
                .await,
        )
    }

    /// Answers an invitation (`respond`).
    pub async fn respond(
        &self,
        ctx: &WriteContext,
        event_id: &str,
        occurrence: &Option<DateInput>,
        status: ResponseStatus,
        comment: Option<&str>,
    ) -> AppResult<WriteOutcome> {
        let _guard = self.write_guard().await;
        finish(
            self.do_respond(ctx, event_id, occurrence, status, comment)
                .await,
        )
    }

    // ----- building blocks -----------------------------------------------------------

    async fn target(&self, calendar_id: &str) -> Step<Target> {
        let id = calendar_id.to_owned();
        let found = self
            .with_reader(move |conn| {
                let Some(calendar) = accounts_store::calendar(conn, &id)? else {
                    return Ok(None);
                };
                let account = accounts_store::account(conn, &calendar.account_id)?;
                Ok(account.map(|account| Target { calendar, account }))
            })
            .await?;
        found.ok_or_else(|| {
            Stop::from(WriteOutcome::rejected(
                "invalid_request",
                format!("Den Kalender {calendar_id} gibt es nicht"),
            ))
        })
    }

    fn check_writable(origin: &Origin, t: &Target) -> Step<()> {
        let name = &t.calendar.name;
        let read_only = || {
            Stop::from(WriteOutcome::rejected(
                "calendar_read_only",
                format!("Der Kalender „{name}“ ist nur lesbar."),
            ))
        };
        match t.account.kind {
            AccountKind::Ics => return Err(read_only()),
            AccountKind::Microsoft => {
                return Err(Stop::from(WriteOutcome::rejected(
                    "calendar_read_only",
                    format!(
                        "Der Kalender „{name}“ ist noch nicht beschreibbar \
                         (Microsoft-Anbindung folgt)."
                    ),
                )))
            }
            _ => {}
        }
        if !t.calendar.writable {
            return Err(read_only());
        }
        if origin.is_mcp() && !t.calendar.mcp_write {
            return Err(Stop::from(WriteOutcome::rejected(
                "calendar_read_only",
                format!(
                    "Claude darf in den Kalender „{name}“ nicht schreiben \
                     (Einstellung „mcp_write“ in der App)."
                ),
            )));
        }
        Ok(())
    }

    fn client_for(&self, account: &AccountRow) -> Step<Client> {
        let Some(credentials) = self.decrypt_credentials(account) else {
            return Err(Stop::from(WriteOutcome::Retry {
                code: "provider_unavailable",
                message: "Die Zugangsdaten des Kontos fehlen, bitte das Konto neu verbinden."
                    .into(),
                retry_after_seconds: None,
            }));
        };
        provider::client(
            account.kind,
            &account.base_url,
            &account.username,
            &credentials.password,
        )
        .map_err(|e| {
            Stop::from(WriteOutcome::Retry {
                code: "provider_unavailable",
                message: e.error.to_string(),
                retry_after_seconds: Some(DEFAULT_RETRY_AFTER_SECS),
            })
        })
    }

    /// Maps a provider error that is no conflict onto `retry`/`rejected`;
    /// refused credentials also set the account to `auth_failed`.
    async fn provider_failure(&self, account: &AccountRow, e: DavError) -> Stop {
        let provider = provider_name(account.kind);
        tracing::warn!(account = %account.id, error = %e, "write to provider failed");
        let outcome = match e {
            DavError::Unavailable { .. } => WriteOutcome::Retry {
                code: "provider_unavailable",
                message: format!("{provider} ist gerade nicht erreichbar."),
                retry_after_seconds: Some(DEFAULT_RETRY_AFTER_SECS),
            },
            DavError::RateLimited { retry_after } => WriteOutcome::Retry {
                code: "provider_unavailable",
                message: format!("{provider} drosselt gerade die Anfragen."),
                retry_after_seconds: Some(
                    retry_after.map_or(DEFAULT_RETRY_AFTER_SECS, |d| d.as_secs().max(1)),
                ),
            },
            DavError::Unauthorized => {
                let message = format!(
                    "{provider} hat die Zugangsdaten abgelehnt. Bitte das Konto neu verbinden."
                );
                if let Err(err) = self
                    .set_status(&account.id, StatusState::AuthFailed, Some(message.clone()))
                    .await
                {
                    return Stop::Internal(err);
                }
                WriteOutcome::Retry {
                    code: "provider_unavailable",
                    message,
                    retry_after_seconds: None,
                }
            }
            DavError::Rejected { .. } | DavError::Forbidden { .. } | DavError::InvalidInput(_) => {
                WriteOutcome::rejected(
                    "provider_rejected",
                    format!(
                        "{provider} hat die Änderung abgelehnt ({}).",
                        short_reason(&e)
                    ),
                )
            }
            other => WriteOutcome::Retry {
                code: "provider_unavailable",
                message: format!("{provider} hat unerwartet geantwortet: {other}"),
                retry_after_seconds: Some(DEFAULT_RETRY_AFTER_SECS),
            },
        };
        Stop::from(outcome)
    }

    async fn event_row(&self, id: &str) -> AppResult<Option<EventRow>> {
        let id = id.to_owned();
        self.with_reader(move |conn| Ok(events_store::event(conn, &id)?))
            .await
    }

    /// The JSON of stored events (missing ones are skipped).
    async fn events_json(&self, ids: Vec<String>) -> AppResult<Vec<Value>> {
        self.with_reader(move |conn| {
            let mut out = Vec::new();
            for id in &ids {
                if let Some(row) = events_store::event(conn, id)? {
                    let ctx = events_store::context_for_calendar(conn, &row.calendar_id)?;
                    out.push(event_json(&row, &ctx));
                }
            }
            Ok(out)
        })
        .await
    }

    /// Stores an object as the provider has it.
    async fn store_object(
        &self,
        calendar_id: &str,
        href: &str,
        etag: Option<String>,
        ics: &str,
    ) -> AppResult<String> {
        let ics = normalize_ics(ics);
        let derived = derive(&ics, default_zone());
        let (calendar_id, href) = (calendar_id.to_owned(), href.to_owned());
        self.with_change_tx(move |tx| {
            let (_, id) = events_store::upsert(
                tx,
                &calendar_id,
                &href,
                etag.as_deref(),
                &ics,
                &derived,
                now_ms(),
            )?;
            Ok(id)
        })
        .await
    }

    /// Stores what a successful PUT left at the provider: the sent data
    /// with the response's ETag, else the object fetched again (if that
    /// fails: the sent data without ETag, so the next sync fetches it).
    async fn store_written(
        &self,
        client: &Client,
        calendar_id: &str,
        w: &Written,
    ) -> AppResult<String> {
        if w.exact {
            return self
                .store_object(calendar_id, &w.href, w.etag.clone(), &w.data)
                .await;
        }
        match client.get(&w.href).await {
            Ok(obj) => {
                self.store_object(calendar_id, &w.href, obj.etag, &obj.data)
                    .await
            }
            Err(e) => {
                tracing::warn!(href = %w.href, error = %e, "refetch after write failed");
                self.store_object(calendar_id, &w.href, None, &w.data).await
            }
        }
    }

    /// Fetches an object from the provider at once and stores it (or
    /// deletes it if gone). Returns the stored event ID, `None` if gone.
    /// A failed fetch keeps what is stored.
    async fn refresh_object(
        &self,
        client: &Client,
        calendar_id: &str,
        href: &str,
    ) -> AppResult<Option<String>> {
        match client.get(href).await {
            Ok(obj) => Ok(Some(
                self.store_object(calendar_id, href, obj.etag, &obj.data)
                    .await?,
            )),
            Err(DavError::NotFound) => {
                let (calendar_id, href) = (calendar_id.to_owned(), href.to_owned());
                self.with_change_tx(move |tx| {
                    events_store::delete_by_href(tx, &calendar_id, &href, now_ms())?;
                    Ok(())
                })
                .await?;
                Ok(None)
            }
            Err(e) => {
                tracing::warn!(href = %href, error = %e, "refetch for a conflict failed");
                let (calendar_id, href) = (calendar_id.to_owned(), href.to_owned());
                self.with_reader(move |conn| {
                    Ok(events_store::event_by_href(conn, &calendar_id, &href)?.map(|r| r.id))
                })
                .await
            }
        }
    }

    /// Records a conflict and answers with the current state of the event.
    async fn record_conflict(
        &self,
        ctx: &WriteContext,
        event_id: Option<String>,
        calendar_id: &str,
        message: String,
    ) -> Step<WriteOutcome> {
        let row = ConflictRow {
            id: conflicts::new_conflict_id(),
            event_id: event_id.clone(),
            calendar_id: Some(calendar_id.to_owned()),
            origin: ctx.origin.label(),
            message,
            mutation: ctx.mutation.clone(),
            created_at: now_ms(),
            seq: 0,
        };
        tracing::info!(conflict = %row.id, origin = %row.origin, "write conflict, provider wins");
        let stored = row.clone();
        self.with_change_tx(move |tx| {
            conflicts::insert(tx, &stored)?;
            Ok(())
        })
        .await?;
        let events = self.events_json(event_id.into_iter().collect()).await?;
        Ok(WriteOutcome::Conflict {
            conflict: Conflict::from(&row),
            events,
        })
    }

    /// The provider (or the stored version) is ahead: fetch the object,
    /// store it and record the conflict. `what` describes the dropped
    /// change ("Deine Änderung (Beginn 10:00)").
    async fn provider_wins(
        &self,
        ctx: &WriteContext,
        client: &Client,
        target: &Target,
        row: &EventRow,
        what: &str,
    ) -> Step<WriteOutcome> {
        let current = self
            .refresh_object(client, &row.calendar_id, &row.href)
            .await?;
        let title = title_of(&row.ics);
        let at = at_provider(target.account.kind);
        let message = match &current {
            Some(_) => format!(
                "Der Termin „{title}“ wurde inzwischen {at} geändert. {what} wurde nicht übernommen."
            ),
            None => format!(
                "Der Termin „{title}“ wurde inzwischen {at} gelöscht. {what} wurde nicht übernommen."
            ),
        };
        self.record_conflict(ctx, current, &row.calendar_id, message)
            .await
    }

    /// Loads the event of an update, delete or answer. Unknown but deleted
    /// earlier (tombstone) → conflict; never known → `event_not_found`.
    async fn load_event(&self, ctx: &WriteContext, event_id: &str, what: &str) -> Step<EventRow> {
        if let Some(row) = self.event_row(event_id).await? {
            return Ok(row);
        }
        let id = event_id.to_owned();
        let deleted = self
            .with_reader(move |conn| {
                Ok(conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM tombstones WHERE type = 'event' AND id = ?1)",
                    [&id],
                    |r| r.get::<_, bool>(0),
                )?)
            })
            .await?;
        if !deleted {
            return Err(WriteOutcome::rejected(
                "event_not_found",
                format!("Den Termin {event_id} gibt es nicht."),
            )
            .into());
        }
        let calendar_id = ctx
            .mutation
            .get("calendar_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let row = ConflictRow {
            id: conflicts::new_conflict_id(),
            event_id: None,
            calendar_id: (!calendar_id.is_empty()).then_some(calendar_id),
            origin: ctx.origin.label(),
            message: format!(
                "Der Termin wurde inzwischen beim Anbieter gelöscht. {what} wurde nicht übernommen."
            ),
            mutation: ctx.mutation.clone(),
            created_at: now_ms(),
            seq: 0,
        };
        let stored = row.clone();
        self.with_change_tx(move |tx| {
            conflicts::insert(tx, &stored)?;
            Ok(())
        })
        .await?;
        Err(WriteOutcome::Conflict {
            conflict: Conflict::from(&row),
            events: Vec::new(),
        }
        .into())
    }

    /// The ETag to write against. Without a stored one the object is
    /// fetched: unchanged data → its ETag; otherwise the provider is ahead.
    async fn etag_for_write(
        &self,
        ctx: &WriteContext,
        client: &Client,
        target: &Target,
        row: &EventRow,
        what: &str,
    ) -> Step<String> {
        if let Some(etag) = &row.etag {
            return Ok(etag.clone());
        }
        match client.get(&row.href).await {
            Ok(obj) if normalize_ics(&obj.data) == row.ics && obj.etag.is_some() => {
                Ok(obj.etag.expect("checked"))
            }
            Ok(_) | Err(DavError::NotFound) => Err(self
                .provider_wins(ctx, client, target, row, what)
                .await?
                .into()),
            Err(e) => Err(self.provider_failure(&target.account, e).await),
        }
    }

    /// PUTs an edited object over the stored one; 412/404 → provider wins.
    async fn put_existing(
        &self,
        ctx: &WriteContext,
        client: &Client,
        target: &Target,
        row: &EventRow,
        object: &CalendarObject,
        what: &str,
    ) -> Step<Written> {
        let etag = self.etag_for_write(ctx, client, target, row, what).await?;
        let ics = object.to_ics();
        match client
            .put(&row.href, &ics, Precondition::IfMatch(etag))
            .await
        {
            Ok(put) => Ok(Written::from_put(put, ics)),
            Err(DavError::PreconditionFailed | DavError::NotFound) => Err(self
                .provider_wins(ctx, client, target, row, what)
                .await?
                .into()),
            Err(e) => Err(self.provider_failure(&target.account, e).await),
        }
    }

    /// Creates a new object in a calendar (`If-None-Match: *`). A 412 for
    /// an object with the same UID counts as written (an earlier attempt
    /// whose answer got lost).
    async fn put_new(
        &self,
        client: &Client,
        target: &Target,
        object: &CalendarObject,
    ) -> Result<Written, DavError> {
        let uid = object.uid().unwrap_or_default();
        let href = client.new_resource_href(&target.calendar.remote_href, &uid)?;
        let ics = object.to_ics();
        match client.put(href.as_str(), &ics, Precondition::Create).await {
            Ok(put) => Ok(Written::from_put(put, ics)),
            Err(DavError::PreconditionFailed) => {
                let existing = client.get(href.as_str()).await?;
                let same = CalendarObject::parse(&existing.data)
                    .ok()
                    .and_then(|o| o.uid())
                    .is_some_and(|u| u == uid);
                if same && normalize_ics(&existing.data) == normalize_ics(&ics) {
                    Ok(Written {
                        href: href.to_string(),
                        etag: existing.etag,
                        exact: true,
                        data: existing.data,
                    })
                } else {
                    Err(DavError::Rejected {
                        status: 412,
                        body: format!("Es gibt dort bereits einen Termin mit der UID {uid}"),
                    })
                }
            }
            Err(e) => Err(e),
        }
    }

    // ----- create --------------------------------------------------------------------

    async fn do_create(
        &self,
        ctx: &WriteContext,
        calendar_id: &str,
        event: &EventInput,
    ) -> Step<WriteOutcome> {
        let fields = resolve(event)?;
        if fields.calendar_id.is_some() {
            return Err(InputError::request(
                "calendar_id gehört beim Anlegen in die Mutation, nicht in event",
            )
            .into());
        }
        match &fields.title {
            Some(Some(t)) if !t.trim().is_empty() => {}
            _ => return Err(InputError::event("title fehlt (Pflicht beim Anlegen)").into()),
        }
        let (Some(_), Some(_)) = (&fields.start, &fields.end) else {
            return Err(InputError::event("start und end sind beim Anlegen Pflicht").into());
        };
        let uid = match &event.uid {
            Some(u) => check_uid(u)?,
            None => format!("{}@lichtung", Uuid::now_v7()),
        };
        let target = self.target(calendar_id).await?;
        Self::check_writable(&ctx.origin, &target)?;
        {
            let (calendar_id, uid_owned) = (calendar_id.to_owned(), uid.clone());
            let taken = self
                .with_reader(move |conn| {
                    Ok(conn.query_row(
                        "SELECT EXISTS (SELECT 1 FROM events WHERE calendar_id = ?1 AND uid = ?2)",
                        [&calendar_id, &uid_owned],
                        |r| r.get::<_, bool>(0),
                    )?)
                })
                .await?;
            if taken {
                return Err(InputError::event(format!(
                    "In diesem Kalender gibt es bereits einen Termin mit der UID {uid}"
                ))
                .into());
            }
        }
        let organizer = EventContext::for_account(&target.account)
            .self_addresses
            .into_iter()
            .next();
        let object = build_object(&uid, &fields, Utc::now(), organizer.as_deref());
        let client = self.client_for(&target.account)?;
        let _guard = self.lock_calendars(&[calendar_id]).await;
        let written = match self.put_new(&client, &target, &object).await {
            Ok(r) => r,
            Err(e) => return Err(self.provider_failure(&target.account, e).await),
        };
        let id = self.store_written(&client, calendar_id, &written).await?;
        tracing::info!(event = %id, origin = %ctx.origin.label(), "event created");
        Ok(WriteOutcome::Applied {
            events: self.events_json(vec![id]).await?,
            deleted_event_ids: Vec::new(),
        })
    }

    // ----- update --------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn do_update(
        &self,
        ctx: &WriteContext,
        event_id: &str,
        base_version: i64,
        scope: Scope,
        occurrence: &Option<DateInput>,
        changes: &EventInput,
    ) -> Step<WriteOutcome> {
        let fields = resolve(changes)?;
        if changes.uid.is_some() {
            return Err(InputError::request("uid lässt sich nicht ändern").into());
        }
        let occ = occurrence_of(occurrence)?;
        if scope != Scope::All && occ.is_none() {
            return Err(InputError::request(
                "occurrence fehlt (Pflicht bei scope this und this_and_following)",
            )
            .into());
        }
        if scope == Scope::This && fields.recurrence.is_some() {
            return Err(InputError::scope(
                "Die Wiederholung lässt sich nur für alle oder folgende Termine ändern",
            )
            .into());
        }
        let row = self.load_event(ctx, event_id, "Deine Änderung").await?;
        let moving_to = fields.calendar_id.clone().filter(|c| *c != row.calendar_id);
        if moving_to.is_some() && scope != Scope::All {
            return Err(InputError::scope(
                "In einen anderen Kalender lässt sich ein Termin nur ganz verschieben (scope all)",
            )
            .into());
        }
        let source = self.target(&row.calendar_id).await?;
        Self::check_writable(&ctx.origin, &source)?;
        let dest = match &moving_to {
            Some(id) => {
                let t = self.target(id).await?;
                Self::check_writable(&ctx.origin, &t)?;
                Some(t)
            }
            None => None,
        };
        let what = format!(
            "Deine Änderung ({})",
            describe(&fields, dest.as_ref().map(|d| d.calendar.name.as_str()))
        );

        let mut locked = vec![row.calendar_id.as_str()];
        if let Some(d) = &moving_to {
            locked.push(d.as_str());
        }
        let _guard = self.lock_calendars(&locked).await;
        // Re-read under the lock: a sync round may have stored a newer one.
        let Some(row) = self.event_row(&row.id).await? else {
            return self
                .load_event(ctx, event_id, &what)
                .await
                .map(|_| unreachable_outcome());
        };
        let client = self.client_for(&source.account)?;
        if row.version != base_version {
            return self.provider_wins(ctx, &client, &source, &row, &what).await;
        }
        if row.parse_error.is_some() {
            return Err(InputError::event(
                "Der Termin ist nicht lesbar und lässt sich deshalb nicht bearbeiten",
            )
            .into());
        }
        let object =
            CalendarObject::parse(&row.ics).map_err(|e| InputError::event(e.to_string()))?;

        let new_uid = format!("{}@lichtung", Uuid::now_v7());
        let ectx = EditContext::new(Utc::now(), default_zone()).with_new_uid(new_uid);
        let edit = edit_event(
            &object,
            occ.as_ref(),
            scope.edit_scope(),
            &edit_changes(&fields),
            &ectx,
        )
        .map_err(ical_error)?;
        let organizer = EventContext::for_account(&source.account)
            .self_addresses
            .into_iter()
            .next();
        let edit = apply_extras(
            edit,
            &object,
            occ.as_ref(),
            scope,
            &fields,
            organizer.as_deref(),
        )?;
        for o in edit.updated.iter().chain(edit.created.iter()) {
            check_object(o)?;
        }

        if let Some(dest) = dest {
            let updated = edit
                .updated
                .expect("an edit with scope all keeps the object");
            return self
                .move_event(ctx, &client, &source, &dest, &row, &updated, &what)
                .await;
        }

        // The new series first: if it cannot be created, nothing changed.
        let mut created: Option<Written> = None;
        if let Some(new) = &edit.created {
            match self.put_new(&client, &source, new).await {
                Ok(r) => created = Some(r),
                Err(e) => return Err(self.provider_failure(&source.account, e).await),
            }
        }
        let updated = edit.updated.as_ref().expect("an edit keeps the object");
        let written = self
            .put_existing(ctx, &client, &source, &row, updated, &what)
            .await;
        let written = match written {
            Ok(r) => r,
            Err(stop) => {
                if let Some(new) = &created {
                    // Undo the new series; the original stays as it was.
                    if let Err(e) = client.delete(&new.href, new.etag.as_deref()).await {
                        tracing::warn!(href = %new.href, error = %e, "cannot undo the split");
                    }
                }
                return Err(stop);
            }
        };
        let mut ids = vec![
            self.store_written(&client, &row.calendar_id, &written)
                .await?,
        ];
        if let Some(new) = &created {
            ids.push(self.store_written(&client, &row.calendar_id, new).await?);
        }
        tracing::info!(event = %row.id, origin = %ctx.origin.label(), "event updated");
        Ok(WriteOutcome::Applied {
            events: self.events_json(ids).await?,
            deleted_event_ids: Vec::new(),
        })
    }

    /// Moves an edited event into another calendar (see the module docs).
    #[allow(clippy::too_many_arguments)]
    async fn move_event(
        &self,
        ctx: &WriteContext,
        client: &Client,
        source: &Target,
        dest: &Target,
        row: &EventRow,
        object: &CalendarObject,
        what: &str,
    ) -> Step<WriteOutcome> {
        let etag = self.etag_for_write(ctx, client, source, row, what).await?;
        let dest_client = self.client_for(&dest.account)?;
        let written = match self.put_new(&dest_client, dest, object).await {
            Ok(r) => r,
            Err(e) if is_uid_conflict(&e) => {
                return self
                    .move_event_delete_first(
                        ctx,
                        client,
                        &dest_client,
                        source,
                        dest,
                        row,
                        object,
                        &etag,
                        what,
                    )
                    .await;
            }
            Err(e) => return Err(self.provider_failure(&dest.account, e).await),
        };
        let new_id = self
            .store_written(&dest_client, &dest.calendar.id, &written)
            .await?;
        match client.delete(&row.href, Some(&etag)).await {
            Ok(()) => {
                let (calendar_id, href) = (row.calendar_id.clone(), row.href.clone());
                self.with_change_tx(move |tx| {
                    events_store::delete_by_href(tx, &calendar_id, &href, now_ms())?;
                    Ok(())
                })
                .await?;
                tracing::info!(from = %row.id, to = %new_id, "event moved");
                Ok(WriteOutcome::Applied {
                    events: self.events_json(vec![new_id]).await?,
                    deleted_event_ids: vec![row.id.clone()],
                })
            }
            Err(e) => {
                if matches!(e, DavError::Unauthorized) {
                    if let Stop::Internal(err) = self.provider_failure(&source.account, e).await {
                        return Err(Stop::Internal(err));
                    }
                } else {
                    tracing::warn!(event = %row.id, error = %e, "move: deleting the source failed");
                }
                let current = self
                    .refresh_object(client, &row.calendar_id, &row.href)
                    .await?;
                let title = title_of(&row.ics);
                let message = format!(
                    "Der Termin „{title}“ wurde in den Kalender „{}“ übernommen, im Kalender „{}“ \
                     aber nicht gelöscht, weil er dort inzwischen geändert wurde oder nicht \
                     erreichbar war. Bitte prüfen, welche Fassung bleiben soll.",
                    dest.calendar.name, source.calendar.name
                );
                let conflict = match self
                    .record_conflict(ctx, current.clone(), &row.calendar_id, message)
                    .await?
                {
                    WriteOutcome::Conflict { conflict, .. } => conflict,
                    _ => unreachable!("conflict() answers with a conflict"),
                };
                let mut ids = vec![new_id];
                ids.extend(current);
                Ok(WriteOutcome::Conflict {
                    conflict,
                    events: self.events_json(ids).await?,
                })
            }
        }
    }

    /// Moving where the provider refuses the same UID twice in a calendar
    /// home: delete in the source first, then create in the target; if
    /// that fails, put the original back.
    #[allow(clippy::too_many_arguments)]
    async fn move_event_delete_first(
        &self,
        ctx: &WriteContext,
        client: &Client,
        dest_client: &Client,
        source: &Target,
        dest: &Target,
        row: &EventRow,
        object: &CalendarObject,
        etag: &str,
        what: &str,
    ) -> Step<WriteOutcome> {
        match client.delete(&row.href, Some(etag)).await {
            Ok(()) => {}
            Err(DavError::PreconditionFailed | DavError::NotFound) => {
                return self.provider_wins(ctx, client, source, row, what).await;
            }
            Err(e) => return Err(self.provider_failure(&source.account, e).await),
        }
        match self.put_new(dest_client, dest, object).await {
            Ok(written) => {
                let new_id = self
                    .store_written(dest_client, &dest.calendar.id, &written)
                    .await?;
                let (calendar_id, href) = (row.calendar_id.clone(), row.href.clone());
                self.with_change_tx(move |tx| {
                    events_store::delete_by_href(tx, &calendar_id, &href, now_ms())?;
                    Ok(())
                })
                .await?;
                Ok(WriteOutcome::Applied {
                    events: self.events_json(vec![new_id]).await?,
                    deleted_event_ids: vec![row.id.clone()],
                })
            }
            Err(e) => {
                tracing::warn!(event = %row.id, error = %e, "move: creating in the target failed, restoring");
                let restored = client.put(&row.href, &row.ics, Precondition::Create).await;
                match restored {
                    Ok(put) => {
                        let written = Written::from_put(put, row.ics.clone());
                        self.store_written(client, &row.calendar_id, &written)
                            .await?;
                        Err(self.provider_failure(&dest.account, e).await)
                    }
                    Err(restore_error) => {
                        tracing::error!(event = %row.id, error = %restore_error, "move: restoring the source failed");
                        let message = format!(
                            "Der Termin „{}“ ließ sich nicht in den Kalender „{}“ verschieben und \
                             wurde im Kalender „{}“ gelöscht. Die verworfene Änderung enthält ihn; \
                             bitte neu anlegen.",
                            title_of(&row.ics),
                            dest.calendar.name,
                            source.calendar.name
                        );
                        self.record_conflict(ctx, None, &row.calendar_id, message)
                            .await
                    }
                }
            }
        }
    }

    // ----- delete --------------------------------------------------------------------

    async fn do_delete(
        &self,
        ctx: &WriteContext,
        event_id: &str,
        base_version: i64,
        scope: Scope,
        occurrence: &Option<DateInput>,
    ) -> Step<WriteOutcome> {
        let occ = occurrence_of(occurrence)?;
        if scope != Scope::All && occ.is_none() {
            return Err(InputError::request(
                "occurrence fehlt (Pflicht bei scope this und this_and_following)",
            )
            .into());
        }
        let what = match scope {
            Scope::All => "Deine Löschung",
            Scope::This => "Deine Löschung dieses Vorkommens",
            Scope::ThisAndFollowing => "Deine Löschung dieses und der folgenden Vorkommen",
        };
        let row = self.load_event(ctx, event_id, what).await?;
        let target = self.target(&row.calendar_id).await?;
        Self::check_writable(&ctx.origin, &target)?;
        let _guard = self.lock_calendars(&[row.calendar_id.as_str()]).await;
        let Some(row) = self.event_row(&row.id).await? else {
            return self
                .load_event(ctx, event_id, what)
                .await
                .map(|_| unreachable_outcome());
        };
        let client = self.client_for(&target.account)?;
        if row.version != base_version {
            return self.provider_wins(ctx, &client, &target, &row, what).await;
        }
        let whole = scope == Scope::All;
        let edit = if whole {
            EditOutcome {
                updated: None,
                created: None,
            }
        } else {
            if row.parse_error.is_some() {
                return Err(InputError::event(
                    "Der Termin ist nicht lesbar; löschen lässt er sich nur ganz (scope all)",
                )
                .into());
            }
            let object =
                CalendarObject::parse(&row.ics).map_err(|e| InputError::event(e.to_string()))?;
            let ectx = EditContext::new(Utc::now(), default_zone());
            delete_event(&object, occ.as_ref(), scope.edit_scope(), &ectx).map_err(ical_error)?
        };
        match edit.updated {
            Some(updated) => {
                let written = self
                    .put_existing(ctx, &client, &target, &row, &updated, what)
                    .await?;
                let id = self
                    .store_written(&client, &row.calendar_id, &written)
                    .await?;
                Ok(WriteOutcome::Applied {
                    events: self.events_json(vec![id]).await?,
                    deleted_event_ids: Vec::new(),
                })
            }
            None => {
                let etag = self
                    .etag_for_write(ctx, &client, &target, &row, what)
                    .await?;
                match client.delete(&row.href, Some(&etag)).await {
                    // Already gone at the provider: what was asked for.
                    Ok(()) | Err(DavError::NotFound) => {}
                    Err(DavError::PreconditionFailed) => {
                        return self.provider_wins(ctx, &client, &target, &row, what).await;
                    }
                    Err(e) => return Err(self.provider_failure(&target.account, e).await),
                }
                let (calendar_id, href) = (row.calendar_id.clone(), row.href.clone());
                self.with_change_tx(move |tx| {
                    events_store::delete_by_href(tx, &calendar_id, &href, now_ms())?;
                    Ok(())
                })
                .await?;
                tracing::info!(event = %row.id, origin = %ctx.origin.label(), "event deleted");
                Ok(WriteOutcome::Applied {
                    events: Vec::new(),
                    deleted_event_ids: vec![row.id.clone()],
                })
            }
        }
    }

    // ----- respond -------------------------------------------------------------------

    async fn do_respond(
        &self,
        ctx: &WriteContext,
        event_id: &str,
        occurrence: &Option<DateInput>,
        status: ResponseStatus,
        comment: Option<&str>,
    ) -> Step<WriteOutcome> {
        let occ = occurrence_of(occurrence)?;
        if comment.is_some_and(|c| c.chars().count() > input::MAX_COMMENT_CHARS) {
            return Err(InputError::request(format!(
                "comment ist zu lang (höchstens {} Zeichen)",
                input::MAX_COMMENT_CHARS
            ))
            .into());
        }
        let what = format!("Deine Antwort ({})", status.label());
        let row = self.load_event(ctx, event_id, &what).await?;
        let target = self.target(&row.calendar_id).await?;
        Self::check_writable(&ctx.origin, &target)?;
        let _guard = self.lock_calendars(&[row.calendar_id.as_str()]).await;
        let Some(row) = self.event_row(&row.id).await? else {
            return self
                .load_event(ctx, event_id, &what)
                .await
                .map(|_| unreachable_outcome());
        };
        if row.parse_error.is_some() {
            return Err(InputError::event("Der Termin ist nicht lesbar").into());
        }
        let object =
            CalendarObject::parse(&row.ics).map_err(|e| InputError::event(e.to_string()))?;
        let me = EventContext::for_account(&target.account).self_addresses;
        let object = answer(&object, occ.as_ref(), &me, status, comment)?;
        let client = self.client_for(&target.account)?;
        let written = self
            .put_existing(ctx, &client, &target, &row, &object, &what)
            .await?;
        let id = self
            .store_written(&client, &row.calendar_id, &written)
            .await?;
        tracing::info!(event = %id, status = status.label(), "invitation answered");
        Ok(WriteOutcome::Applied {
            events: self.events_json(vec![id]).await?,
            deleted_event_ids: Vec::new(),
        })
    }
}

/// `load_event` ends with an outcome when the row is gone; this is never
/// built.
fn unreachable_outcome() -> WriteOutcome {
    WriteOutcome::rejected("event_not_found", "Den Termin gibt es nicht mehr.")
}

/// Applies what `edit_event` does not know (attendees, alarms, RDATE and
/// EXDATE) to the components the scope chose.
fn apply_extras(
    mut edit: EditOutcome,
    original: &CalendarObject,
    occurrence: Option<&CalTime>,
    scope: Scope,
    f: &Fields,
    organizer: Option<&str>,
) -> Result<EditOutcome, InputError> {
    if f.attendees.is_none() && f.alarms.is_none() && !matches!(f.recurrence, Some(Some(_))) {
        return Ok(edit);
    }
    let recurring = original.master().is_some_and(|m| m.is_recurring());
    let resolver = original.resolver(default_zone());
    // The object holding the changed events, and whether only one
    // occurrence (override) of it is meant.
    let (object, single): (&mut CalendarObject, Option<CalTime>) = match (scope, recurring) {
        (Scope::This, true) => (edit.updated.as_mut().expect("kept"), occurrence.cloned()),
        (Scope::ThisAndFollowing, true) if edit.created.is_some() => {
            (edit.created.as_mut().expect("checked"), None)
        }
        _ => (edit.updated.as_mut().expect("kept"), None),
    };
    let indices: Vec<usize> = match &single {
        Some(rid) => object.find_override(rid, &resolver).into_iter().collect(),
        None => object
            .calendar()
            .components()
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is("VEVENT"))
            .map(|(i, _)| i)
            .collect(),
    };
    for i in indices {
        let mut ev = object.event_at_mut(i).expect("VEVENT index");
        if let Some(list) = &f.attendees {
            set_attendees(&mut ev, list, organizer);
        }
        if let Some(list) = &f.alarms {
            set_alarms(&mut ev, list);
        }
    }
    if let (Some(Some(r)), None) = (&f.recurrence, &single) {
        if let Some(mut master) = object.master_mut() {
            set_recurrence(&mut master, r);
        }
    }
    Ok(edit)
}

/// Sets the own `PARTSTAT` (and `COMMENT`) in the whole object or in one
/// occurrence. An attendee does not change `SEQUENCE` (RFC 5546).
fn answer(
    object: &CalendarObject,
    occurrence: Option<&CalTime>,
    me: &[String],
    status: ResponseStatus,
    comment: Option<&str>,
) -> Result<CalendarObject, InputError> {
    let resolver = object.resolver(default_zone());
    let recurring = object.master().is_some_and(|m| m.is_recurring()) || object.master().is_none();
    let mut out = object.clone();
    let indices: Vec<usize> = match occurrence.filter(|_| recurring) {
        Some(rid) => {
            let existing = object.find_override(rid, &resolver);
            let sequence = existing
                .and_then(|i| object.event_at(i))
                .or_else(|| object.master())
                .map(|e| e.sequence())
                .unwrap_or(0);
            if object.master().is_some() {
                let ectx = EditContext::new(Utc::now(), default_zone());
                out = edit_event(
                    object,
                    Some(rid),
                    EditScope::This,
                    &EventChanges::default(),
                    &ectx,
                )
                .map_err(ical_error)?
                .updated
                .expect("kept");
            }
            let i = out
                .find_override(rid, &resolver)
                .ok_or_else(|| InputError {
                    code: "occurrence_not_found",
                    message: "Das Vorkommen gibt es in dieser Serie nicht".into(),
                })?;
            if let Some(mut ev) = out.event_at_mut(i) {
                ev.set_sequence(sequence);
            }
            vec![i]
        }
        None => out
            .calendar()
            .components()
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is("VEVENT"))
            .map(|(i, _)| i)
            .collect(),
    };
    let partstat = status.partstat();
    let mut found = false;
    for i in indices {
        let mut ev = out.event_at_mut(i).expect("VEVENT index");
        let organizer_is_me = ev.organizer().is_some_and(|o| {
            let email = o.address.trim().to_ascii_lowercase();
            me.iter()
                .any(|m| email.trim_start_matches("mailto:") == m.as_str())
        });
        if organizer_is_me {
            continue;
        }
        let mut here = false;
        for address in me {
            here |= ev.set_partstat(address, &partstat);
        }
        if here {
            found = true;
            if let Some(text) = comment.map(str::trim) {
                let c = ev.component_mut();
                c.remove_properties("COMMENT");
                if !text.is_empty() {
                    c.insert_property(lichtung_ical::Property::new_text("COMMENT", text));
                }
            }
            ev.touch(Utc::now());
        }
    }
    if !found {
        return Err(InputError::event(
            "Du bist bei diesem Termin nicht als Teilnehmer eingetragen.",
        ));
    }
    Ok(out)
}
