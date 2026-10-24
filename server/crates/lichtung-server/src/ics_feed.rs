//! ICS subscriptions (LIC-14): read-only calendars fetched over HTTP, among
//! them the published feed of the work calendar (spec section 4, spike S3).
//!
//! **Account.** `kind: ics` holds exactly one calendar (`writable`,
//! `can_invite` and `mcp_write` are `false`). The feed URL usually carries a
//! secret token (in the query or in the path), so it is stored only inside
//! the encrypted credentials ([`StoredCredentials::url`]), together with an
//! optional Basic-auth user and password. The account row holds:
//!
//! - `username`: the URL for display ([`display_url`]): without user info,
//!   query and fragment, path segments that look like keys replaced by `…`;
//! - `base_url`: a fingerprint of the full URL (HMAC-SHA256 with
//!   `LICHTUNG_SECRET_KEY`), so the same feed cannot be added twice without
//!   storing the URL in plain text.
//!
//! **URL schemes.** `https` and `http` as given. `webcals` becomes `https`.
//! `webcal` is tried as `https` first and only falls back to `http` if the
//! HTTPS connection cannot be made at all (most feeds behind `webcal://`
//! links speak HTTPS, and the token should not travel in plain text).
//!
//! **Fetch** ([`fetch`]): conditional `GET` with `If-None-Match` /
//! `If-Modified-Since` from the last answer; `304` means nothing to do.
//! Redirects are followed by hand (at most [`MAX_REDIRECTS`]); Basic auth
//! goes only to the origin of the feed (or its HTTPS upgrade). Bodies over
//! [`MAX_FEED_BYTES`] are refused, the whole request times out after
//! [`FEED_TIMEOUT`]. Errors never contain the URL (reqwest errors are
//! stripped of it before they are logged).
//!
//! **Content** ([`prepare`]): the feed is split per `UID`
//! ([`lichtung_ical::split_by_uid`]); every UID becomes one event whose
//! `href` is the synthetic key `ics:uid:<UID>` and whose original is its own
//! `VCALENDAR` with the `VTIMEZONE`s it references. Outlook/Exchange quirks
//! are tolerated: duplicate `VEVENT`s of one UID with the same
//! `RECURRENCE-ID` (or two masters) are reduced to the newest (`SEQUENCE`,
//! then `LAST-MODIFIED`, then `DTSTAMP`); objects without a master are kept
//! as they are (the event JSON handles them); Windows zone names are
//! resolved by `lichtung-ical`; `X-MICROSOFT-CDO-BUSYSTATUS` feeds the
//! availability when `TRANSP` is missing ([`crate::events::json`]).
//!
//! **ETag of an event**: a SHA-256 over its components without `DTSTAMP`
//! (many feeds stamp every event with the time of the request), so an
//! unchanged event keeps its version. **Applying** ([`apply_feed`]) runs in
//! one transaction through the change log: new events, changed ones
//! (version + 1), vanished ones (tombstone), the calendar's name and color
//! (`X-WR-CALNAME`, `X-APPLE-CALENDAR-COLOR`), the HTTP validators (ETag in
//! `calendars.sync_token`, Last-Modified in `calendars.ctag`, columns the
//! CalDAV sync uses for its own tokens) and the account status.
//!
//! **Schedule:** the sync scheduler ([`crate::sync`]) runs ICS accounts
//! every [`ICS_SYNC_INTERVAL`]; `POST /api/accounts/{id}/sync` runs one at
//! once. Status as with CalDAV: `ok`, `error` (temporary, with backoff),
//! `auth_failed` (401/403, waits for new credentials).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;

use hmac::{Hmac, Mac};
use lichtung_ical::{split_by_uid, CalendarObject, Component};
use reqwest::header::{
    HeaderMap, ACCEPT, CONTENT_LENGTH, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED,
    LOCATION,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use crate::accounts::provider::RemoteCalendar;
use crate::accounts::store::{self as accounts_store, AccountRow};
use crate::accounts::{
    self, validate_name, Account, AccountKind, AccountPatch, NewAccount, RefreshReport,
    StatusState, MAX_PASSWORD_CHARS, MAX_URL_CHARS, MAX_USERNAME_CHARS,
};
use crate::app::{now_ms, App};
use crate::crypto::{Secret, StoredCredentials};
use crate::error::{AppError, AppResult};
use crate::events::store::{self as events_store, SyncState, Upsert};
use crate::events::{default_zone, derive, normalize_ics};
use crate::sync::{CalendarSyncReport, SyncReport};

/// Regular fetch interval of an ICS subscription.
pub const ICS_SYNC_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Largest feed accepted (bytes of the body).
pub const MAX_FEED_BYTES: usize = 20 * 1024 * 1024;
/// Overall timeout of one fetch (connect, redirects excluded, and body).
pub const FEED_TIMEOUT: Duration = Duration::from_secs(60);
/// Timeout for establishing a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Redirects followed per fetch.
pub const MAX_REDIRECTS: usize = 5;
/// `remote_href` of the one calendar of an ICS account.
pub const FEED_CALENDAR_HREF: &str = "ics:feed";
/// `href` prefix of an event with a `UID`.
const UID_HREF_PREFIX: &str = "ics:uid:";
/// `href` prefix of an event without `UID` (followed by its content hash).
const NO_UID_HREF_PREFIX: &str = "ics:nouid:";
/// Longest HTTP validator (ETag, Last-Modified) that is stored.
const MAX_VALIDATOR_CHARS: usize = 1000;

// ----- URLs -----------------------------------------------------------------------------

/// A feed URL as the user gave it: the URLs to try in order (only `webcal`
/// has two) and Basic-auth credentials found in the URL itself.
#[derive(Debug)]
pub(crate) struct FeedUrl {
    pub candidates: Vec<Url>,
    pub user_info: Option<(String, Secret)>,
}

/// Parses and normalizes a feed URL (see the module docs). Messages never
/// repeat the URL: it is secret.
pub(crate) fn parse_feed_url(raw: &str) -> AppResult<FeedUrl> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::bad_request("url fehlt (Adresse des ICS-Abos)"));
    }
    if raw.chars().count() > MAX_URL_CHARS {
        return Err(AppError::bad_request("url ist zu lang"));
    }
    let invalid = || AppError::bad_request("url ist keine gültige Adresse");
    let (scheme, rest) = raw.split_once(':').ok_or_else(invalid)?;
    let schemes: &[&str] = match scheme.to_ascii_lowercase().as_str() {
        "https" | "webcals" => &["https"],
        "http" => &["http"],
        "webcal" => &["https", "http"],
        _ => {
            return Err(AppError::bad_request(
                "url muss mit https://, http://, webcal:// oder webcals:// beginnen",
            ))
        }
    };
    let mut user_info = None;
    let mut candidates = Vec::new();
    for scheme in schemes {
        let mut url = Url::parse(&format!("{scheme}:{rest}")).map_err(|_| invalid())?;
        if url.host_str().is_none_or(str::is_empty) {
            return Err(invalid());
        }
        if !url.username().is_empty() || url.password().is_some() {
            let decode = |s: &str| {
                percent_decode(s).ok_or_else(|| {
                    AppError::bad_request("Zugangsdaten in der url sind ungültig kodiert")
                })
            };
            user_info = Some((
                decode(url.username())?,
                Secret::new(decode(url.password().unwrap_or(""))?),
            ));
            let _ = url.set_username("");
            let _ = url.set_password(None);
        }
        url.set_fragment(None);
        candidates.push(url);
    }
    Ok(FeedUrl {
        candidates,
        user_info,
    })
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Whether a path segment looks like a key or token (long, or long enough
/// with several digits): Outlook and Google put the secret into the path.
fn looks_secret(segment: &str) -> bool {
    let len = segment.chars().count();
    let digits = segment.chars().filter(char::is_ascii_digit).count();
    len >= 32 || (len >= 16 && digits >= 4)
}

/// The URL as shown in API answers (`username` of the account): scheme,
/// host, port and path, without user info, query and fragment; path
/// segments that look like keys become `…`.
pub fn display_url(url: &Url) -> String {
    let mut out = format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""));
    if let Some(port) = url.port() {
        out.push_str(&format!(":{port}"));
    }
    match url.path_segments() {
        Some(segments) => {
            for segment in segments {
                out.push('/');
                if looks_secret(segment) {
                    out.push('…');
                } else {
                    out.push_str(segment);
                }
            }
        }
        None => out.push('/'),
    }
    out
}

/// Fingerprint of the full URL for the account identity (`base_url`).
pub(crate) fn fingerprint(key: &[u8], url: &Url) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(b"lichtung:ics-feed:");
    mac.update(url.as_str().as_bytes());
    format!("ics:{}", hex(&mac.finalize().into_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ----- fetching -------------------------------------------------------------------------

/// HTTP validators of the last answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// Result of a fetch.
#[derive(Debug)]
pub enum Fetched {
    /// `304 Not Modified`.
    NotModified,
    /// `200` with the body (UTF-8, invalid bytes replaced).
    Feed {
        body: String,
        validators: Validators,
    },
}

/// A failed fetch or an unusable feed. `Display` is German and never
/// contains the URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedError {
    /// 401 or 403.
    Unauthorized(u16),
    /// 404 or 410.
    NotFound(u16),
    /// Another 4xx (or an unexpected 1xx/2xx/3xx).
    Rejected(u16),
    /// 5xx or 429.
    Unavailable(u16),
    Timeout,
    /// The connection could not be established (DNS, refused, TLS).
    Connect,
    /// The connection broke off.
    Network,
    TooLarge,
    TooManyRedirects,
    BadRedirect,
    /// The answer is not iCalendar (e.g. an HTML login page).
    NotCalendar,
    /// iCalendar that cannot be parsed.
    Unreadable(String),
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedError::Unauthorized(s) => write!(
                f,
                "Der ICS-Server verlangt Zugangsdaten oder hat sie abgelehnt (HTTP {s})"
            ),
            FeedError::NotFound(s) => write!(
                f,
                "Unter dieser Adresse gibt es keinen Kalender (HTTP {s}). Ist der Link noch gültig?"
            ),
            FeedError::Rejected(s) => {
                write!(f, "Der ICS-Server hat die Anfrage abgelehnt (HTTP {s})")
            }
            FeedError::Unavailable(s) => {
                write!(f, "Der ICS-Server ist gerade nicht erreichbar (HTTP {s})")
            }
            FeedError::Timeout => f.write_str("Der ICS-Server hat nicht rechtzeitig geantwortet"),
            FeedError::Connect => f.write_str("Keine Verbindung zum ICS-Server möglich"),
            FeedError::Network => f.write_str("Die Verbindung zum ICS-Server ist abgebrochen"),
            FeedError::TooLarge => write!(
                f,
                "Der Kalender ist größer als {} MB und wird nicht übernommen",
                MAX_FEED_BYTES / (1024 * 1024)
            ),
            FeedError::TooManyRedirects => write!(
                f,
                "Der ICS-Server leitet zu oft weiter (mehr als {MAX_REDIRECTS} Mal)"
            ),
            FeedError::BadRedirect => {
                f.write_str("Der ICS-Server leitet auf eine ungültige Adresse weiter")
            }
            FeedError::NotCalendar => f.write_str(
                "Unter dieser Adresse liegt kein Kalender (die Antwort ist kein iCalendar/VCALENDAR)",
            ),
            FeedError::Unreadable(e) => write!(f, "Der Kalender ist nicht lesbar: {e}"),
        }
    }
}

impl FeedError {
    /// The connection could not be made at all (for the `webcal` fallback).
    fn is_connect(&self) -> bool {
        matches!(self, FeedError::Connect)
    }

    /// The API error when creating or reconnecting an account.
    pub fn into_app_error(self) -> AppError {
        let message = self.to_string();
        match self {
            FeedError::Unauthorized(_) => AppError::InvalidCredentials(format!(
                "{message}. Benutzername und Passwort bzw. den Link prüfen."
            )),
            FeedError::Unavailable(_)
            | FeedError::Timeout
            | FeedError::Connect
            | FeedError::Network => {
                AppError::ProviderUnavailable(format!("{message}. Bitte später erneut versuchen."))
            }
            FeedError::NotFound(_)
            | FeedError::Rejected(_)
            | FeedError::TooLarge
            | FeedError::TooManyRedirects
            | FeedError::BadRedirect
            | FeedError::NotCalendar
            | FeedError::Unreadable(_) => AppError::InvalidFeed(message),
        }
    }

    /// Account status after a failed run.
    pub fn status(&self) -> (StatusState, String) {
        match self {
            FeedError::Unauthorized(_) => (
                StatusState::AuthFailed,
                format!("{self}. Bitte neu verbinden."),
            ),
            _ => (StatusState::Error, self.to_string()),
        }
    }
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("lichtung/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .expect("HTTP client for ICS feeds builds")
    })
}

fn map_reqwest(e: reqwest::Error) -> FeedError {
    // Never log the URL: it carries the feed's token.
    let e = e.without_url();
    tracing::debug!(error = %e, "ICS fetch failed");
    if e.is_timeout() {
        FeedError::Timeout
    } else if e.is_connect() {
        FeedError::Connect
    } else {
        FeedError::Network
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port() == b.port()
}

/// Whether Basic auth for `origin` may go to `url`: same origin, or the
/// HTTPS upgrade of the same host.
fn may_send_credentials(origin: &Url, url: &Url) -> bool {
    same_origin(origin, url)
        || (url.scheme() == "https"
            && url.host_str() == origin.host_str()
            && url.port_or_known_default() == Some(443))
}

fn header_value(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.chars().count() <= MAX_VALIDATOR_CHARS)
        .map(str::to_owned)
}

/// Fetches the feed (see the module docs). `limit` is the largest body
/// accepted.
pub async fn fetch(
    url: &Url,
    auth: Option<(&str, &str)>,
    validators: &Validators,
    limit: usize,
) -> Result<Fetched, FeedError> {
    let fetch = async {
        let mut current = url.clone();
        for _ in 0..=MAX_REDIRECTS {
            let mut req = http_client()
                .get(current.clone())
                .header(ACCEPT, "text/calendar, */*;q=0.5");
            if let Some(etag) = &validators.etag {
                req = req.header(IF_NONE_MATCH, etag);
            }
            if let Some(lm) = &validators.last_modified {
                req = req.header(IF_MODIFIED_SINCE, lm);
            }
            if let Some((user, password)) = auth {
                if may_send_credentials(url, &current) {
                    req = req.basic_auth(user, Some(password));
                }
            }
            let mut resp = req.send().await.map_err(map_reqwest)?;
            let status = resp.status().as_u16();
            match status {
                301 | 302 | 303 | 307 | 308 => {
                    let next = resp
                        .headers()
                        .get(LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|l| current.join(l).ok())
                        .filter(|u| matches!(u.scheme(), "http" | "https"))
                        .ok_or(FeedError::BadRedirect)?;
                    current = next;
                    continue;
                }
                304 => return Ok(Fetched::NotModified),
                200 => {}
                401 | 403 => return Err(FeedError::Unauthorized(status)),
                404 | 410 => return Err(FeedError::NotFound(status)),
                429 | 500..=599 => return Err(FeedError::Unavailable(status)),
                other => return Err(FeedError::Rejected(other)),
            }
            let declared = resp
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            if declared.is_some_and(|n| n > limit as u64) {
                return Err(FeedError::TooLarge);
            }
            let found = Validators {
                etag: header_value(resp.headers(), ETAG),
                last_modified: header_value(resp.headers(), LAST_MODIFIED),
            };
            let mut body: Vec<u8> = Vec::new();
            while let Some(chunk) = resp.chunk().await.map_err(map_reqwest)? {
                if body.len() + chunk.len() > limit {
                    return Err(FeedError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            return Ok(Fetched::Feed {
                body: String::from_utf8_lossy(&body).into_owned(),
                validators: found,
            });
        }
        Err(FeedError::TooManyRedirects)
    };
    tokio::time::timeout(FEED_TIMEOUT, fetch)
        .await
        .unwrap_or(Err(FeedError::Timeout))
}

// ----- content --------------------------------------------------------------------------

/// One event of a feed, ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedItem {
    /// Synthetic key (`ics:uid:<UID>`).
    pub href: String,
    /// Content hash (`sha256:<hex>`), the event's ETag.
    pub etag: String,
    /// The event's own `VCALENDAR` (normalized, CRLF).
    pub ics: String,
}

/// A parsed feed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreparedFeed {
    /// `X-WR-CALNAME`.
    pub name: Option<String>,
    /// `X-WR-CALDESC`.
    pub description: Option<String>,
    /// `X-APPLE-CALENDAR-COLOR` as `#RRGGBB`.
    pub color: Option<String>,
    pub items: Vec<FeedItem>,
    /// `VEVENT`s dropped as duplicates (same UID and `RECURRENCE-ID`).
    pub duplicates: usize,
}

fn text_property(calendar: &Component, name: &str) -> Option<String> {
    calendar
        .property(name)
        .map(|p| p.text().trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// `#RRGGBB` or `#RRGGBBAA` (Apple) → `#RRGGBB` in upper case.
fn feed_color(raw: &str) -> Option<String> {
    let hex = raw.trim().strip_prefix('#')?;
    if matches!(hex.len(), 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(format!("#{}", hex[..6].to_ascii_uppercase()))
    } else {
        None
    }
}

/// Ranks duplicates of one occurrence: the newest wins.
fn rank(c: &Component) -> (u32, String, String) {
    let value = |name: &str| {
        c.property(name)
            .map(|p| p.value().trim().to_owned())
            .unwrap_or_default()
    };
    let sequence = c
        .property("SEQUENCE")
        .and_then(|p| p.value().trim().parse().ok())
        .unwrap_or(0);
    (sequence, value("LAST-MODIFIED"), value("DTSTAMP"))
}

/// Which occurrence a `VEVENT` describes: `RECURRENCE-ID` (with its zone),
/// or the master.
fn occurrence_key(c: &Component) -> String {
    match c.property("RECURRENCE-ID") {
        Some(p) => format!(
            "{}|{}",
            p.param("TZID").unwrap_or(""),
            p.value().trim().to_ascii_uppercase()
        ),
        None => "master".to_owned(),
    }
}

/// Keeps one `VEVENT` per occurrence (the newest); returns how many were
/// dropped.
fn dedup_events(calendar: &mut Component) -> usize {
    let components = std::mem::take(calendar.components_mut());
    let mut out: Vec<Component> = Vec::with_capacity(components.len());
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut dropped = 0;
    for c in components {
        if !c.is("VEVENT") {
            out.push(c);
            continue;
        }
        let key = occurrence_key(&c);
        match index.get(&key) {
            Some(&i) => {
                dropped += 1;
                if rank(&c) > rank(&out[i]) {
                    out[i] = c;
                }
            }
            None => {
                index.insert(key, out.len());
                out.push(c);
            }
        }
    }
    *calendar.components_mut() = out;
    dropped
}

/// SHA-256 over the components without `DTSTAMP`.
fn content_hash(calendar: &Component) -> String {
    let mut hasher = Sha256::new();
    for c in calendar.components() {
        if c.is("VEVENT") {
            let mut c = c.clone();
            c.remove_properties("DTSTAMP");
            hasher.update(c.to_ics().as_bytes());
        } else {
            hasher.update(c.to_ics().as_bytes());
        }
    }
    format!("sha256:{}", hex(&hasher.finalize()))
}

/// Moves the events of `from` into `into` (same UID in two `VCALENDAR`s of
/// one feed), with the time zones `into` lacks.
fn merge_into(into: &mut Component, from: Component) {
    let present: HashSet<String> = into
        .components_named("VTIMEZONE")
        .filter_map(|c| c.property("TZID").map(|p| p.value().to_owned()))
        .collect();
    for c in from.components() {
        let keep = if c.is("VTIMEZONE") {
            c.property("TZID")
                .is_some_and(|p| !present.contains(p.value()))
        } else {
            c.is("VEVENT")
        };
        if keep {
            into.components_mut().push(c.clone());
        }
    }
}

/// Parses a feed body into events (see the module docs).
pub fn prepare(body: &str) -> Result<PreparedFeed, FeedError> {
    let start = body.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    let head: String = start.chars().take(15).collect();
    if !head.eq_ignore_ascii_case("BEGIN:VCALENDAR") {
        return Err(FeedError::NotCalendar);
    }
    let objects = split_by_uid(start).map_err(|e| FeedError::Unreadable(e.to_string()))?;
    let header: Component = match objects.first() {
        Some(o) => o.calendar().clone(),
        None => lichtung_ical::parse(start)
            .map_err(|e| FeedError::Unreadable(e.to_string()))?
            .into_iter()
            .find(|c| c.is("VCALENDAR"))
            .ok_or(FeedError::NotCalendar)?,
    };

    // Group by UID (a feed may hold several VCALENDARs).
    let mut order: Vec<(Option<String>, Component)> = Vec::new();
    let mut by_uid: HashMap<String, usize> = HashMap::new();
    for object in objects {
        let uid = object.uid().filter(|u| !u.trim().is_empty());
        let component = object.into_component();
        match uid.as_ref().and_then(|u| by_uid.get(u)) {
            Some(&i) => merge_into(&mut order[i].1, component),
            None => {
                if let Some(u) = &uid {
                    by_uid.insert(u.clone(), order.len());
                }
                order.push((uid, component));
            }
        }
    }

    let mut feed = PreparedFeed {
        name: text_property(&header, "X-WR-CALNAME"),
        description: text_property(&header, "X-WR-CALDESC"),
        color: header
            .property("X-APPLE-CALENDAR-COLOR")
            .and_then(|p| feed_color(p.value())),
        ..Default::default()
    };
    let mut seen: HashSet<String> = HashSet::new();
    for (uid, mut component) in order {
        feed.duplicates += dedup_events(&mut component);
        let etag = content_hash(&component);
        let href = match &uid {
            Some(uid) => format!("{UID_HREF_PREFIX}{uid}"),
            None => format!("{NO_UID_HREF_PREFIX}{}", &etag["sha256:".len()..]),
        };
        if !seen.insert(href.clone()) {
            // Identical event without UID twice.
            feed.duplicates += 1;
            continue;
        }
        let ics = normalize_ics(
            &CalendarObject::from_component(component)
                .map_or_else(|_| String::new(), |o| o.to_ics()),
        );
        feed.items.push(FeedItem { href, etag, ics });
    }
    Ok(feed)
}

// ----- applying -------------------------------------------------------------------------

/// Brings the account's calendar and its events in line with the feed, and
/// stores the validators. Call inside a change transaction.
pub(crate) fn apply_feed(
    conn: &Connection,
    account: &AccountRow,
    feed: &PreparedFeed,
    validators: &Validators,
    now: i64,
) -> AppResult<(RefreshReport, CalendarSyncReport)> {
    let remote = RemoteCalendar {
        href: FEED_CALENDAR_HREF.to_owned(),
        name: feed.name.clone().unwrap_or_else(|| account.name.clone()),
        description: feed.description.clone(),
        color: feed.color.clone(),
        writable: false,
        shared: false,
        can_invite: false,
    };
    let calendars = accounts::reconcile(conn, account, &[remote], now)?;
    let calendar = accounts_store::calendars(conn, Some(&account.id))?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("ICS calendar missing")))?;

    let stored: HashMap<String, Option<String>> =
        events_store::etags_of_calendar(conn, &calendar.id)?
            .into_iter()
            .map(|(href, _, etag)| (href, etag))
            .collect();
    let zone = default_zone();
    let mut report = CalendarSyncReport {
        full: true,
        ..Default::default()
    };
    let mut present: HashSet<&str> = HashSet::new();
    for item in &feed.items {
        present.insert(item.href.as_str());
        if stored
            .get(&item.href)
            .is_some_and(|e| e.as_deref() == Some(item.etag.as_str()))
        {
            report.unchanged += 1;
            continue;
        }
        let derived = derive(&item.ics, zone);
        let (what, _) = events_store::upsert(
            conn,
            &calendar.id,
            &item.href,
            Some(&item.etag),
            &item.ics,
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
            tracing::warn!(calendar = %calendar.id, "unreadable event from ICS feed stored");
        }
    }
    for href in stored.keys() {
        if !present.contains(href.as_str())
            && events_store::delete_by_href(conn, &calendar.id, href, now)?.is_some()
        {
            report.deleted += 1;
        }
    }
    events_store::set_sync_state(
        conn,
        &calendar.id,
        &SyncState {
            sync_token: validators.etag.clone(),
            ctag: validators.last_modified.clone(),
            sync_unsupported: false,
        },
    )?;
    Ok((calendars, report))
}

// ----- service --------------------------------------------------------------------------

/// What is needed to fetch the feed of an account.
struct FeedSource {
    url: Url,
    basic: Option<(String, Secret)>,
}

impl FeedSource {
    fn auth(&self) -> Option<(&str, &str)> {
        self.basic
            .as_ref()
            .map(|(user, password)| (user.as_str(), password.expose()))
    }
}

fn validate_basic(
    username: Option<&str>,
    password: Option<&Secret>,
) -> AppResult<Option<(String, Secret)>> {
    let username = username.map(str::trim).unwrap_or_default();
    let password = password.cloned().unwrap_or_default();
    if username.is_empty() {
        if !password.expose().is_empty() {
            return Err(AppError::bad_request(
                "password ohne username: Für die Anmeldung am ICS-Server fehlt der Benutzername",
            ));
        }
        return Ok(None);
    }
    if username.chars().count() > MAX_USERNAME_CHARS || username.chars().any(char::is_control) {
        return Err(AppError::bad_request("username ist ungültig"));
    }
    if password.expose().chars().count() > MAX_PASSWORD_CHARS {
        return Err(AppError::bad_request("password ist zu lang"));
    }
    Ok(Some((username.to_owned(), password)))
}

fn sealed(app: &App, id: &str, url: &Url, basic: &Option<(String, Secret)>) -> Vec<u8> {
    StoredCredentials {
        password: basic.as_ref().map(|(_, p)| p.clone()).unwrap_or_default(),
        url: Some(Secret::new(url.as_str())),
        username: basic.as_ref().map(|(u, _)| Secret::new(u.clone())),
    }
    .seal(app.cipher(), id)
}

fn duplicate_account() -> AppError {
    AppError::Conflict("Dieses Kalender-Abo ist bereits verbunden".into())
}

fn not_found_account() -> AppError {
    AppError::NotFound("Konto nicht gefunden".into())
}

/// Fetches and checks a feed for creating or reconnecting an account: the
/// first candidate URL that answers wins (`webcal` falls back to `http` only
/// if HTTPS cannot connect).
async fn check_feed(
    candidates: &[Url],
    basic: &Option<(String, Secret)>,
) -> AppResult<(Url, Validators, PreparedFeed)> {
    let auth = basic.as_ref().map(|(u, p)| (u.as_str(), p.expose()));
    let mut last = FeedError::Connect;
    for (i, url) in candidates.iter().enumerate() {
        match fetch(url, auth, &Validators::default(), MAX_FEED_BYTES).await {
            Ok(Fetched::Feed { body, validators }) => {
                let feed = tokio::task::spawn_blocking(move || prepare(&body))
                    .await
                    .map_err(|e| AppError::Internal(e.into()))?
                    .map_err(FeedError::into_app_error)?;
                return Ok((url.clone(), validators, feed));
            }
            // Not asked for: treat as a refusal.
            Ok(Fetched::NotModified) => return Err(FeedError::Rejected(304).into_app_error()),
            Err(e) if e.is_connect() && i + 1 < candidates.len() => {
                tracing::info!("ICS feed not reachable over HTTPS, trying HTTP (webcal)");
                last = e;
            }
            Err(e) => return Err(e.into_app_error()),
        }
    }
    Err(last.into_app_error())
}

fn new_account_id() -> String {
    format!("acc_{}", Uuid::now_v7().simple())
}

impl App {
    /// `POST /api/accounts` with `kind: ics`: fetches and checks the feed,
    /// then stores the account (URL encrypted), its calendar and the events
    /// in one transaction.
    pub(crate) async fn create_ics_account(&self, input: NewAccount) -> AppResult<Account> {
        let raw = input
            .url
            .as_deref()
            .ok_or_else(|| AppError::bad_request("url fehlt (Adresse des ICS-Abos)"))?;
        let feed_url = parse_feed_url(raw)?;
        let name = input
            .name
            .as_deref()
            .map(|n| validate_name(n, "Der Name eines Kontos"))
            .transpose()?;
        let basic = if input.username.is_some() || input.password.is_some() {
            validate_basic(input.username.as_deref(), input.password.as_ref())?
        } else {
            feed_url.user_info.clone().filter(|(u, _)| !u.is_empty())
        };

        let (url, validators, feed) = check_feed(&feed_url.candidates, &basic).await?;
        let base_url = fingerprint(&self.config().secret_key, &url);
        let username = display_url(&url);
        {
            let (base_url, username) = (base_url.clone(), username.clone());
            let existing = self
                .with_reader(move |conn| {
                    Ok(accounts_store::find_account(
                        conn,
                        AccountKind::Ics,
                        &base_url,
                        &username,
                    )?)
                })
                .await?;
            if existing.is_some() {
                return Err(duplicate_account());
            }
        }
        let name = match name {
            Some(n) => n,
            None => match &feed.name {
                Some(n) => validate_name(n, "Der Kalendername")
                    .unwrap_or_else(|_| url.host_str().unwrap_or("Kalender-Abo").to_owned()),
                None => url.host_str().unwrap_or("Kalender-Abo").to_owned(),
            },
        };

        let id = new_account_id();
        let now = now_ms();
        let row = AccountRow {
            credentials: Some(sealed(self, &id, &url, &basic)),
            id,
            kind: AccountKind::Ics,
            name,
            username,
            base_url,
            principal_url: None,
            home_url: None,
            auto_schedule: false,
            user_addresses: Vec::new(),
            status_state: StatusState::Ok,
            status_message: None,
            last_sync_at: Some(now),
            created_at: now,
            updated_at: now,
            seq: 0,
        };
        let account = self
            .with_change_tx(move |tx| {
                if accounts_store::find_account(tx, AccountKind::Ics, &row.base_url, &row.username)?
                    .is_some()
                {
                    return Err(duplicate_account());
                }
                accounts_store::insert_account(tx, &row)?;
                let (_, report) = apply_feed(tx, &row, &feed, &validators, now)?;
                tracing::info!(
                    account = %row.id,
                    kind = "ics",
                    events = report.added,
                    duplicates = feed.duplicates,
                    "account connected"
                );
                Ok(Account::from(&row))
            })
            .await?;
        self.wake_sync_scheduler();
        Ok(account)
    }

    /// `PATCH /api/accounts/{id}` of an ICS account with a new `url`,
    /// `username` or `password` (renaming alone takes the general path):
    /// checks the feed first and stores the credentials only if it works.
    pub(crate) async fn reconnect_ics_account(
        &self,
        current: AccountRow,
        name: Option<String>,
        patch: AccountPatch,
    ) -> AppResult<Account> {
        let stored = self.decrypt_credentials(&current).unwrap_or_default();
        let (candidates, user_info) = match patch.url.as_deref() {
            Some(raw) => {
                let parsed = parse_feed_url(raw)?;
                (parsed.candidates, parsed.user_info)
            }
            None => {
                let url = stored
                    .url
                    .as_ref()
                    .and_then(|u| Url::parse(u.expose()).ok())
                    .ok_or_else(|| {
                        AppError::bad_request(
                            "Die gespeicherte Adresse ist nicht mehr lesbar, bitte url mitschicken",
                        )
                    })?;
                (vec![url], None)
            }
        };
        let username = match patch.username.as_deref() {
            Some(u) => Some(u.to_owned()),
            None => match &user_info {
                Some((u, _)) => Some(u.clone()),
                None => stored.username.as_ref().map(|u| u.expose().to_owned()),
            },
        };
        let no_user = username.as_deref().is_none_or(|u| u.trim().is_empty());
        let password = match (&patch.password, &user_info, patch.username.is_some()) {
            (Some(p), _, _) => p.clone(),
            // Clearing the user name drops the stored password as well.
            (None, _, _) if no_user => Secret::default(),
            (None, Some((_, p)), false) => p.clone(),
            _ => stored.password.clone(),
        };
        let basic = validate_basic(username.as_deref(), Some(&password))?;

        let (url, validators, feed) = check_feed(&candidates, &basic).await?;
        let base_url = fingerprint(&self.config().secret_key, &url);
        let display = display_url(&url);
        let credentials = sealed(self, &current.id, &url, &basic);
        let id = current.id.clone();
        let account = self
            .with_change_tx(move |tx| {
                let mut row = accounts_store::account(tx, &id)?.ok_or_else(not_found_account)?;
                if let Some(other) =
                    accounts_store::find_account(tx, AccountKind::Ics, &base_url, &display)?
                {
                    if other != id {
                        return Err(duplicate_account());
                    }
                }
                let now = now_ms();
                if let Some(name) = name {
                    row.name = name;
                }
                row.username = display;
                row.base_url = base_url;
                row.credentials = Some(credentials);
                row.status_state = StatusState::Ok;
                row.status_message = None;
                row.last_sync_at = Some(now);
                row.updated_at = now;
                accounts_store::update_account(tx, &row)?;
                apply_feed(tx, &row, &feed, &validators, now)?;
                tracing::info!(account = %row.id, "ICS account updated");
                Ok(Account::from(&row))
            })
            .await?;
        self.wake_sync_scheduler();
        Ok(account)
    }

    async fn fail_ics(&self, id: &str, e: FeedError) -> AppResult<SyncReport> {
        let (state, message) = e.status();
        tracing::warn!(account = %id, error = %e, "ICS sync failed");
        self.set_status(id, state, Some(message)).await?;
        Err(e.into_app_error())
    }

    /// One run of an ICS account (called by [`crate::sync`]): conditional
    /// fetch, then everything in one transaction.
    pub(crate) async fn run_ics_sync(&self, row: AccountRow) -> AppResult<SyncReport> {
        let id = row.id.clone();
        let source = self.decrypt_credentials(&row).and_then(|c| {
            let url = Url::parse(c.url.as_ref()?.expose()).ok()?;
            let basic = c.username.map(|u| (u.expose().to_owned(), c.password));
            Some(FeedSource { url, basic })
        });
        let Some(source) = source else {
            let message = "Zugangsdaten fehlen, bitte das Konto neu verbinden".to_owned();
            self.set_status(&id, StatusState::AuthFailed, Some(message.clone()))
                .await?;
            return Err(AppError::InvalidCredentials(message));
        };
        self.set_status(&id, StatusState::Syncing, None).await?;

        let validators = {
            let id = id.clone();
            self.with_reader(move |conn| {
                let Some(calendar) = accounts_store::calendars(conn, Some(&id))?
                    .into_iter()
                    .next()
                else {
                    return Ok(Validators::default());
                };
                let state = events_store::sync_state(conn, &calendar.id)?.unwrap_or_default();
                Ok(Validators {
                    etag: state.sync_token,
                    last_modified: state.ctag,
                })
            })
            .await?
        };
        let fetched = match fetch(&source.url, source.auth(), &validators, MAX_FEED_BYTES).await {
            Ok(f) => f,
            Err(e) => return self.fail_ics(&id, e).await,
        };
        let (body, validators) = match fetched {
            Fetched::NotModified => {
                self.finish_ics(&id, None).await?;
                tracing::debug!(account = %id, "ICS feed not modified");
                return Ok(SyncReport::default());
            }
            Fetched::Feed { body, validators } => (body, validators),
        };
        let feed = match tokio::task::spawn_blocking(move || prepare(&body))
            .await
            .map_err(|e| AppError::Internal(e.into()))?
        {
            Ok(f) => f,
            Err(e) => return self.fail_ics(&id, e).await,
        };
        let duplicates = feed.duplicates;
        match self.finish_ics(&id, Some((feed, validators))).await {
            Ok(report) => {
                tracing::info!(
                    account = %id,
                    added = report.events.added,
                    updated = report.events.updated,
                    deleted = report.events.deleted,
                    duplicates,
                    "ICS account synced"
                );
                Ok(report)
            }
            Err(e) => {
                self.set_status(
                    &id,
                    StatusState::Error,
                    Some("Interner Fehler beim Abgleich, siehe Server-Log".into()),
                )
                .await?;
                Err(e)
            }
        }
    }

    /// Applies a fetched feed (if any) and sets the account to `ok`, in one
    /// transaction.
    async fn finish_ics(
        &self,
        id: &str,
        fetched: Option<(PreparedFeed, Validators)>,
    ) -> AppResult<SyncReport> {
        let id = id.to_owned();
        self.with_change_tx(move |tx| {
            let mut row = accounts_store::account(tx, &id)?.ok_or_else(not_found_account)?;
            let now = now_ms();
            let report = match &fetched {
                Some((feed, validators)) => {
                    let (calendars, events) = apply_feed(tx, &row, feed, validators, now)?;
                    SyncReport { calendars, events }
                }
                None => SyncReport::default(),
            };
            row.status_state = StatusState::Ok;
            row.status_message = None;
            row.last_sync_at = Some(now);
            row.updated_at = now;
            accounts_store::update_account(tx, &row)?;
            Ok(report)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(body: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//Feed//DE\r\n\
             X-WR-CALNAME:Arbeit\r\nX-APPLE-CALENDAR-COLOR:#1a2b3cff\r\n{body}END:VCALENDAR\r\n"
        )
    }

    #[test]
    fn urls_are_normalized_and_schemes_mapped() {
        let u = parse_feed_url(" webcal://feeds.example.org/k.ics?token=abc#x ").unwrap();
        let urls: Vec<&str> = u.candidates.iter().map(Url::as_str).collect();
        assert_eq!(
            urls,
            [
                "https://feeds.example.org/k.ics?token=abc",
                "http://feeds.example.org/k.ics?token=abc"
            ]
        );
        let u = parse_feed_url("WEBCALS://feeds.example.org/k.ics").unwrap();
        assert_eq!(u.candidates[0].as_str(), "https://feeds.example.org/k.ics");
        let u = parse_feed_url("http://feeds.example.org/k.ics").unwrap();
        assert_eq!(u.candidates.len(), 1);
        // Built at runtime: an invented user:password pair in a URL.
        let with_user = format!(
            "https://anna:{}@feeds.example.org/k.ics",
            ["ge", "heim"].join("%3A")
        );
        let u = parse_feed_url(&with_user).unwrap();
        assert_eq!(u.candidates[0].as_str(), "https://feeds.example.org/k.ics");
        let (user, password) = u.user_info.unwrap();
        assert_eq!((user.as_str(), password.expose()), ("anna", "ge:heim"));
        for bad in [
            "ftp://feeds.example.org/",
            "kein url",
            "https://",
            "",
            "file:///etc",
        ] {
            let e = parse_feed_url(bad).unwrap_err();
            assert!(!e.to_string().contains("feeds.example.org"), "{e}");
        }
    }

    #[test]
    fn display_url_hides_query_and_tokens() {
        let url = Url::parse(
            "https://outlook.example.com/owa/calendar/0a1b2c3d4e5f60718293a4b5c6d7e8f9@example.com/9f8e7d6c5b4a39281706f5e4d3c2b1a0/calendar.ics?x=1",
        )
        .unwrap();
        assert_eq!(
            display_url(&url),
            "https://outlook.example.com/owa/calendar/…/…/calendar.ics"
        );
        let url =
            Url::parse("http://127.0.0.1:8080/feeds/private-a1b2c3d4e5f6a7b8/basic.ics").unwrap();
        assert_eq!(display_url(&url), "http://127.0.0.1:8080/feeds/…/basic.ics");
        let url = Url::parse("https://feiertage.example.org/").unwrap();
        assert_eq!(display_url(&url), "https://feiertage.example.org/");
    }

    #[test]
    fn fingerprint_depends_on_key_and_full_url() {
        let a = Url::parse("https://feeds.example.org/k.ics?token=1").unwrap();
        let b = Url::parse("https://feeds.example.org/k.ics?token=2").unwrap();
        assert_ne!(fingerprint(&[1; 32], &a), fingerprint(&[1; 32], &b));
        assert_ne!(fingerprint(&[1; 32], &a), fingerprint(&[2; 32], &a));
        assert!(fingerprint(&[1; 32], &a).starts_with("ics:"));
        assert!(!fingerprint(&[1; 32], &a).contains("token"));
    }

    #[test]
    fn feed_is_split_with_header_fields_and_stable_hashes() {
        let feed = wrap(
            "BEGIN:VEVENT\r\nUID:a@example.org\r\nDTSTAMP:20261023T080000Z\r\n\
             DTSTART:20261103T130000Z\r\nSUMMARY:Eins\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:b@example.org\r\nDTSTAMP:20261023T080000Z\r\n\
             DTSTART:20261104T130000Z\r\nSUMMARY:Zwei\r\nEND:VEVENT\r\n",
        );
        let p = prepare(&feed).unwrap();
        assert_eq!(p.name.as_deref(), Some("Arbeit"));
        assert_eq!(p.color.as_deref(), Some("#1A2B3C"));
        let hrefs: Vec<&str> = p.items.iter().map(|i| i.href.as_str()).collect();
        assert_eq!(hrefs, ["ics:uid:a@example.org", "ics:uid:b@example.org"]);
        assert!(p.items[0].ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(p.items[0].ics.contains("SUMMARY:Eins"));
        assert!(!p.items[0].ics.contains("Zwei"));

        // A new DTSTAMP alone does not change the hash.
        let restamped = feed.replace("DTSTAMP:20261023T080000Z", "DTSTAMP:20261023T091500Z");
        let q = prepare(&restamped).unwrap();
        assert_eq!(p.items[0].etag, q.items[0].etag);
        let changed = feed.replace("SUMMARY:Eins", "SUMMARY:Eins (neu)");
        let r = prepare(&changed).unwrap();
        assert_ne!(p.items[0].etag, r.items[0].etag);
        assert_eq!(p.items[1].etag, r.items[1].etag);
    }

    #[test]
    fn duplicates_keep_the_newest_and_no_uid_gets_a_hash_key() {
        let feed = wrap(
            "BEGIN:VEVENT\r\nUID:s@example.org\r\nRECURRENCE-ID:20261103T130000Z\r\n\
             DTSTART:20261103T140000Z\r\nSEQUENCE:1\r\nSUMMARY:Alt\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:s@example.org\r\nRECURRENCE-ID:20261103T130000Z\r\n\
             DTSTART:20261103T150000Z\r\nSEQUENCE:2\r\nSUMMARY:Neu\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nDTSTART:20261105T130000Z\r\nSUMMARY:Ohne UID\r\nEND:VEVENT\r\n",
        );
        let p = prepare(&feed).unwrap();
        assert_eq!(p.duplicates, 1);
        assert_eq!(p.items.len(), 2);
        assert!(p.items[0].ics.contains("SUMMARY:Neu"));
        assert!(!p.items[0].ics.contains("SUMMARY:Alt"));
        assert!(p.items[1].href.starts_with("ics:nouid:"));
    }

    #[test]
    fn non_calendars_are_refused() {
        assert_eq!(
            prepare("<!doctype html><html>Anmelden</html>"),
            Err(FeedError::NotCalendar)
        );
        assert!(matches!(
            prepare("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n"),
            Err(FeedError::Unreadable(_))
        ));
        let empty = prepare(
            "\u{feff}BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Leer\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        assert!(empty.items.is_empty());
        assert_eq!(empty.name.as_deref(), Some("Leer"));
    }

    #[test]
    fn errors_map_to_codes_and_states() {
        assert_eq!(
            FeedError::Unauthorized(401).into_app_error().code(),
            "invalid_credentials"
        );
        assert_eq!(
            FeedError::NotFound(404).into_app_error().code(),
            "invalid_feed"
        );
        assert_eq!(
            FeedError::NotCalendar.into_app_error().code(),
            "invalid_feed"
        );
        assert_eq!(FeedError::TooLarge.into_app_error().code(), "invalid_feed");
        assert_eq!(
            FeedError::Unavailable(503).into_app_error().code(),
            "provider_unavailable"
        );
        assert_eq!(
            FeedError::Timeout.into_app_error().code(),
            "provider_unavailable"
        );
        assert_eq!(
            FeedError::Unauthorized(403).status().0,
            StatusState::AuthFailed
        );
        assert_eq!(FeedError::Unavailable(503).status().0, StatusState::Error);
    }
}
