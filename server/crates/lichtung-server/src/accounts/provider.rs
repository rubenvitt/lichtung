//! Talking to the calendar provider when an account is created, changed or
//! refreshed: discovery plus calendar listing via [`lichtung_caldav`], and
//! the mapping of provider errors onto API errors and account status.

use std::time::Duration;

use lichtung_caldav::{CalendarInfo, Client, Credentials, Discovery, Sharing, Url};

use super::{AccountKind, StatusState};
use crate::crypto::Secret;
use crate::error::AppError;

/// Overall timeout per provider request while creating or refreshing an
/// account (the app waits for the answer of `POST /api/accounts`).
pub const PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);

/// What the provider reported about an account.
#[derive(Debug, Clone)]
pub struct Remote {
    pub principal: Url,
    pub home: Url,
    pub auto_schedule: bool,
    /// Event calendars in provider order (see [`event_calendars`]).
    pub calendars: Vec<RemoteCalendar>,
}

/// One event calendar as the provider reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCalendar {
    pub href: String,
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub writable: bool,
    pub shared: bool,
    pub can_invite: bool,
}

/// A failed provider call, with the provider for the message.
#[derive(Debug)]
pub struct ProviderError {
    pub kind: AccountKind,
    pub error: lichtung_caldav::Error,
}

impl ProviderError {
    /// The API error: rejected credentials → `422 invalid_credentials`,
    /// everything else → `502 provider_unavailable` with a German message.
    pub fn into_app_error(self) -> AppError {
        let provider = self.kind.provider_name();
        use lichtung_caldav::Error as E;
        match self.error {
            E::Unauthorized => AppError::InvalidCredentials(format!(
                "{provider} hat die Zugangsdaten abgelehnt. Benutzername und App-Passwort prüfen."
            )),
            E::Unavailable { .. } => AppError::ProviderUnavailable(format!(
                "{provider} ist gerade nicht erreichbar. Bitte später erneut versuchen."
            )),
            E::RateLimited { .. } => AppError::ProviderUnavailable(format!(
                "{provider} drosselt gerade die Anfragen. Bitte später erneut versuchen."
            )),
            E::InvalidInput(msg) => AppError::bad_request(msg),
            other => AppError::ProviderUnavailable(format!(
                "{provider} hat unerwartet geantwortet: {other}"
            )),
        }
    }

    /// Account status after a failed refresh.
    pub fn status(&self) -> (StatusState, String) {
        let state = if matches!(self.error, lichtung_caldav::Error::Unauthorized) {
            StatusState::AuthFailed
        } else {
            StatusState::Error
        };
        let message = match &self.error {
            lichtung_caldav::Error::Unauthorized => format!(
                "{} hat die Zugangsdaten abgelehnt. Bitte neu verbinden.",
                self.kind.provider_name()
            ),
            other => other.to_string(),
        };
        (state, message)
    }
}

/// Discovers the account at `base_url` and lists its calendars.
pub async fn fetch(
    kind: AccountKind,
    base_url: &str,
    username: &str,
    password: &Secret,
) -> Result<Remote, ProviderError> {
    let wrap = |error| ProviderError { kind, error };
    let client = Client::builder(base_url)
        .credentials(Credentials::basic(username, password.expose()))
        .timeout(PROVIDER_TIMEOUT)
        .build()
        .map_err(wrap)?;
    let discovery = client.discover().await.map_err(wrap)?;
    let home = discovery.calendar_home().cloned().ok_or_else(|| {
        wrap(lichtung_caldav::Error::Protocol(
            "Server meldet keine Kalender-Sammlung (calendar-home-set)".into(),
        ))
    })?;
    let infos = client
        .list_calendars(home.as_str(), Some(&discovery.principal))
        .await
        .map_err(wrap)?;
    Ok(Remote {
        auto_schedule: discovery.auto_schedule(),
        calendars: event_calendars(&discovery, infos),
        principal: discovery.principal,
        home,
    })
}

/// Only collections that can hold events become calendars; VTODO-only
/// collections (Reminders) are skipped, tasks are a non-goal (spec
/// section 2). Ordered by `calendar-order`, then as listed.
pub fn event_calendars(discovery: &Discovery, infos: Vec<CalendarInfo>) -> Vec<RemoteCalendar> {
    let mut infos: Vec<(usize, CalendarInfo)> = infos
        .into_iter()
        .filter(CalendarInfo::supports_events)
        .enumerate()
        .collect();
    infos.sort_by_key(|(index, info)| (info.order.is_none(), info.order, *index));
    infos
        .into_iter()
        .map(|(_, info)| RemoteCalendar {
            href: info.href.as_str().to_owned(),
            name: calendar_name(&info),
            description: info.description.clone().filter(|d| !d.trim().is_empty()),
            color: info.color.clone(),
            writable: info.writable,
            shared: info.sharing == Sharing::SharedWithMe,
            // Provisional (LIC-6 settles it): a calendar sends real
            // invitations if the server announced server-side scheduling
            // (RFC 6638 `calendar-auto-schedule`) and we may write into it.
            can_invite: discovery.auto_schedule() && info.writable,
        })
        .collect()
}

/// `DAV:displayname`, else the last path segment of the collection.
fn calendar_name(info: &CalendarInfo) -> String {
    if let Some(name) = info
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return name.to_owned();
    }
    let path = lichtung_caldav::href::decoded_path(&info.href);
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Kalender")
        .to_owned()
}
