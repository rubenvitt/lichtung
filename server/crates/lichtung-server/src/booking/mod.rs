//! The public booking page (LIC-33) and its protection (LIC-34): the only
//! part of Lichtung reachable without login.
//!
//! | Route | Purpose |
//! |---|---|
//! | `GET /o/{token}` | free times of the link, seven days per page (`?from=YYYY-MM-DD`), grouped by day in the guest's zone (`?tz=`) |
//! | `GET /o/{token}?slot=<start>` | the booking form for one time |
//! | `POST /o/{token}` | book: checks, then the event in the target calendar with the guest as attendee |
//!
//! **Language:** `?lang=de|en`, else `Accept-Language`, else English.
//! **Zone:** `?tz=` (IANA); a small inline script sets it from the browser
//! when missing, else the link's zone; the page shows and offers to change
//! it.
//!
//! **What the page shows:** the link's title and length and free times.
//! Never anything from the calendar: titles, places and attendees of other
//! events do not leave the server, busy times only show as missing slots.
//!
//! **Booking** (`POST`): origin check, rate limits, form token (signed,
//! bound to link and slot, minimum fill time), honeypot, input checks; then
//! under [`App::opening_booking_guard`]: limits on bookings,
//! [`App::check_opening_slot`], the event through the write path
//! (`create_event`, origin `server`) with the guest as `ATTENDEE`
//! (`RSVP=TRUE`) and the account's own address as `ORGANIZER`, then
//! [`App::mark_opening_booked`] (tried twice), which also stores what the
//! guest entered; after releasing the guard
//! [`App::remove_booked_placeholders`]. The provider sends the invitation
//! from the own address (CalDAV scheduling, RFC 6638; targets need
//! `can_invite`).
//!
//! **Note private (LIC-33):** the invitation goes from the own address to
//! whatever address the guest typed, so it must carry nothing the guest
//! controls, or the booking page would send the guest's text (spam) from
//! the own address. Title: the link's title only. Description: only the
//! link's video link (set by the owner), else none. The guest's name stands
//! only as the attendee's `CN` (checked: no links, no control characters);
//! name, address and note are stored with the booking and shown only in
//! the management API ([`crate::openings::Booking`]).
//!
//! Unknown and deleted links look the same (`404`); a used one-off link
//! says so (`410`). Limits: [`guard`], [`crate::config::BookingLimits`].

pub mod guard;
pub mod html;

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::rejection::{FormRejection, QueryRejection};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State};
use axum::http::header::{ACCEPT_LANGUAGE, ORIGIN};
use axum::http::{Extensions, HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::{Form, Router};
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use lichtung_ical::Zone;
use serde::Deserialize;
use serde_json::json;

use crate::accounts::store as accounts_store;
use crate::app::App;
use crate::error::AppResult;
use crate::events::EventContext;
use crate::openings::{Guest, Opening, Slot};
use crate::writes::{Origin, WriteOutcome};
use guard::{Bucket, FormError, InputError, Limiter};
use html::{Ctx, Entered, Lang, Nav};

/// Body limit of the booking form.
pub const BODY_LIMIT: usize = 8 * 1024;
/// Days on one page of free times.
pub const DAYS_PER_PAGE: i64 = 7;
/// Without `?from=` an empty first page moves on by at most this many
/// pages to the first free time.
const AUTO_ADVANCE_PAGES: usize = 4;
/// `Retry-After` of the per-address page and form limits.
const RETRY_SHORT_SECS: u64 = 10 * 60;
/// `Retry-After` of the probing and booking limits.
const RETRY_LONG_SECS: u64 = 60 * 60;

/// State of the booking routes: the app and the rate limits (in memory).
#[derive(Clone)]
pub struct BookingState {
    app: App,
    limiter: Arc<Limiter>,
}

impl BookingState {
    pub fn new(app: App) -> Self {
        Self {
            app,
            limiter: Arc::default(),
        }
    }
}

pub fn routes<S>(state: BookingState) -> Router<S> {
    Router::new()
        .route("/o/{token}", get(show).post(book))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state)
}

#[derive(Debug, Default, Deserialize)]
struct PageQuery {
    lang: Option<String>,
    tz: Option<String>,
    from: Option<String>,
    slot: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct BookingForm {
    slot: Option<String>,
    tz: Option<String>,
    lang: Option<String>,
    form: Option<String>,
    name: Option<String>,
    email: Option<String>,
    note: Option<String>,
    /// Honeypot: hidden from people, filled by naive bots.
    website: Option<String>,
}

/// One request: language, client key (and IPv6 /48), time.
struct Visit {
    lang: Lang,
    source: String,
    net: Option<String>,
    now: DateTime<Utc>,
}

impl Visit {
    fn new(extensions: &Extensions, headers: &HeaderMap, lang: Option<&str>) -> Self {
        let peer = extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
        let accept = headers.get(ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok());
        let ip = guard::source_of(peer, headers);
        Visit {
            lang: Lang::choose(lang, accept),
            source: guard::source_key(ip),
            net: guard::net_key(ip),
            now: Utc::now(),
        }
    }

    fn now_ms(&self) -> i64 {
        self.now.timestamp_millis()
    }
}

/// A public token as the server issues it: 43 characters of base64url.
fn plausible_token(token: &str) -> bool {
    token.len() == 43
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn parse_instant(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// The guest's zone: `?tz=` if valid, else the link's zone, else UTC.
fn guest_zone(raw: Option<&str>, opening: &Opening) -> Tz {
    raw.and_then(|z| Tz::from_str(z.trim()).ok())
        .or_else(|| Tz::from_str(&opening.time_zone).ok())
        .unwrap_or(Tz::UTC)
}

/// Local midnight of `day` in `tz` as UTC; where midnight falls into a
/// gap (a zone that starts summer time at 00:00), the first moment after
/// it, as in [`crate::free`].
fn day_start(day: NaiveDate, tz: Tz) -> DateTime<Utc> {
    Zone::Iana(tz).to_utc(day.and_time(NaiveTime::MIN))
}

/// Latest start the link can offer.
fn horizon_end(opening: &Opening, now: DateTime<Utc>) -> DateTime<Utc> {
    if opening.suggestions_only {
        opening
            .suggestions
            .iter()
            .filter_map(|s| parse_instant(&s.start))
            .max()
            .unwrap_or(now)
    } else {
        now + Duration::days(opening.horizon_days)
    }
}

/// First day worth showing: the first suggestion, else now plus minimum
/// notice.
fn first_day(opening: &Opening, now: DateTime<Utc>, tz: Tz) -> NaiveDate {
    let first = if opening.suggestions_only {
        opening
            .suggestions
            .iter()
            .filter_map(|s| parse_instant(&s.start))
            .min()
            .unwrap_or(now)
    } else {
        now + Duration::minutes(opening.min_notice_minutes)
    };
    first.max(now).with_timezone(&tz).date_naive()
}

/// Rejects a booking sent from another site. Browsers send `Origin: null`
/// under `Referrer-Policy: no-referrer`, so only a foreign origin and a
/// `Sec-Fetch-Site: cross-site` count.
fn foreign_origin(app: &App, headers: &HeaderMap) -> bool {
    let cross_site = headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("cross-site"));
    let foreign = headers
        .get(ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| o != "null" && o != app.config().public_origin());
    cross_site || foreign
}

impl BookingState {
    fn limits(&self) -> &crate::config::BookingLimits {
        &self.app.config().booking_limits
    }

    /// Per-address limits before anything else: an address that probed
    /// too many unknown links waits, then the request is counted.
    fn admit(&self, visit: &Visit, bucket: Bucket, limit: u32) -> Option<Response> {
        let now = visit.now_ms();
        let limits = self.limits();
        let probing =
            !self
                .limiter
                .allows(Bucket::Miss, &visit.source, limits.misses_per_source, now)
                || visit.net.as_deref().is_some_and(|net| {
                    !self
                        .limiter
                        .allows(Bucket::MissNet, net, limits.misses_per_net, now)
                });
        if probing {
            return Some(html::too_many(visit.lang, RETRY_LONG_SECS));
        }
        if !self.limiter.try_hit(bucket, &visit.source, limit, now) {
            tracing::info!(bucket = ?bucket, "booking page rate limited");
            return Some(html::too_many(visit.lang, RETRY_SHORT_SECS));
        }
        None
    }

    /// The link behind a token, or the page to show instead (unknown,
    /// deleted and malformed look the same; they count as a miss).
    async fn lookup(&self, token: &str, visit: &Visit) -> Result<Opening, Response> {
        let t = visit.lang.texts();
        let found = if plausible_token(token) {
            match self.app.opening_by_token(token).await {
                Ok(found) => found,
                Err(e) => {
                    tracing::error!(error = %e, "booking page lookup failed");
                    return Err(unavailable(visit.lang));
                }
            }
        } else {
            None
        };
        match found {
            None => {
                self.limiter
                    .hit(Bucket::Miss, &visit.source, visit.now_ms());
                if let Some(net) = &visit.net {
                    self.limiter.hit(Bucket::MissNet, net, visit.now_ms());
                }
                Err(html::message(
                    visit.lang,
                    StatusCode::NOT_FOUND,
                    t.not_found_title,
                    t.not_found_text,
                ))
            }
            Some(o) if o.booked_at.is_some() => Err(html::message(
                visit.lang,
                StatusCode::GONE,
                t.used_title,
                t.used_text,
            )),
            Some(o) => Ok(o),
        }
    }

    async fn slots_from(
        &self,
        opening: &Opening,
        from: NaiveDate,
        tz: Tz,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Slot>> {
        let start = day_start(from, tz);
        let end = day_start(from + Duration::days(DAYS_PER_PAGE), tz);
        self.app.opening_slots(&opening.id, start, end, now).await
    }

    /// The page of free times.
    async fn list(
        &self,
        ctx: &Ctx<'_>,
        opening: &Opening,
        from: Option<&str>,
        now: DateTime<Utc>,
        notice: Option<&str>,
        status: StatusCode,
    ) -> Response {
        let tz = ctx.tz;
        let today = now.with_timezone(&tz).date_naive();
        let last_day = horizon_end(opening, now)
            .with_timezone(&tz)
            .date_naive()
            .max(today);
        let explicit = from.and_then(|d| NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").ok());
        let mut day = match explicit {
            Some(d) => d.clamp(today, last_day),
            None => first_day(opening, now, tz).clamp(today, last_day),
        };
        let page = Duration::days(DAYS_PER_PAGE);
        let mut slots = match self.slots_from(opening, day, tz, now).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(opening = %opening.id, error = %e, "free times unavailable");
                return unavailable(ctx.lang);
            }
        };
        if explicit.is_none() {
            for _ in 0..AUTO_ADVANCE_PAGES {
                if !slots.is_empty() || day + page > last_day {
                    break;
                }
                day += page;
                slots = match self.slots_from(opening, day, tz, now).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(opening = %opening.id, error = %e, "free times unavailable");
                        return unavailable(ctx.lang);
                    }
                };
            }
        }
        let nav = Nav {
            from: day,
            earlier: (day > today).then(|| (day - page).max(today)),
            later: (day + page <= last_day).then_some(day + page),
        };
        html::slots_page(ctx, &slots, &nav, notice, status)
    }

    /// Whether `start` is still a free time of the link.
    async fn bookable(
        &self,
        opening: &Opening,
        start: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<Option<Slot>> {
        let end = start + Duration::minutes(opening.duration_minutes);
        self.app
            .check_opening_slot(&opening.id, start, end, now)
            .await
    }

    fn form_token(&self, opening: &Opening, start: DateTime<Utc>, now_ms: i64) -> String {
        guard::sign_form(
            &self.app.config().secret_key,
            &opening.token,
            start.timestamp_millis(),
            now_ms,
        )
    }

    /// The account's own address for the target calendar: the organizer of
    /// the booking (`None`: the provider could not send an invitation).
    async fn organizer(&self, calendar_id: &str) -> AppResult<Option<String>> {
        let id = calendar_id.to_owned();
        self.app
            .with_reader(move |conn| {
                let Some(calendar) = accounts_store::calendar(conn, &id)? else {
                    return Ok(None);
                };
                let Some(account) = accounts_store::account(conn, &calendar.account_id)? else {
                    return Ok(None);
                };
                Ok(EventContext::for_account(&account)
                    .self_addresses
                    .into_iter()
                    .next())
            })
            .await
    }
}

fn unavailable(lang: Lang) -> Response {
    let t = lang.texts();
    html::message(
        lang,
        StatusCode::SERVICE_UNAVAILABLE,
        t.unavailable_title,
        t.unavailable_text,
    )
}

fn invalid(lang: Lang) -> Response {
    let t = lang.texts();
    html::message(
        lang,
        StatusCode::BAD_REQUEST,
        t.invalid_title,
        t.invalid_text,
    )
}

fn ctx_of(opening: &Opening, lang: Lang, tz: Tz) -> Ctx<'_> {
    Ctx {
        lang,
        tz,
        title: &opening.title,
        duration_minutes: opening.duration_minutes,
    }
}

/// `GET /o/{token}`: free times, or with `?slot=` the form for one.
async fn show(
    State(state): State<BookingState>,
    Path(token): Path<String>,
    extensions: Extensions,
    headers: HeaderMap,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    let q = query.map(|Query(q)| q).unwrap_or_default();
    let visit = Visit::new(&extensions, &headers, q.lang.as_deref());
    if let Some(r) = state.admit(&visit, Bucket::View, state.limits().views_per_source) {
        return r;
    }
    let opening = match state.lookup(&token, &visit).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    let tz = guest_zone(q.tz.as_deref(), &opening);
    let ctx = ctx_of(&opening, visit.lang, tz);
    let t = visit.lang.texts();
    if let Some(start) = q.slot.as_deref().and_then(parse_instant) {
        match state.bookable(&opening, start, visit.now).await {
            Ok(Some(slot)) => {
                let form = state.form_token(&opening, slot.start, visit.now_ms());
                return html::form_page(
                    &ctx,
                    &slot,
                    &form,
                    &Entered::default(),
                    &[],
                    StatusCode::OK,
                );
            }
            Ok(None) => {
                let from = start.with_timezone(&tz).date_naive().format("%Y-%m-%d");
                return state
                    .list(
                        &ctx,
                        &opening,
                        Some(&from.to_string()),
                        visit.now,
                        Some(t.taken),
                        StatusCode::OK,
                    )
                    .await;
            }
            Err(e) => {
                tracing::warn!(opening = %opening.id, error = %e, "slot check failed");
                return unavailable(visit.lang);
            }
        }
    }
    state
        .list(
            &ctx,
            &opening,
            q.from.as_deref(),
            visit.now,
            None,
            StatusCode::OK,
        )
        .await
}

/// `POST /o/{token}`: book a time (module docs).
async fn book(
    State(state): State<BookingState>,
    Path(token): Path<String>,
    extensions: Extensions,
    headers: HeaderMap,
    form: Result<Form<BookingForm>, FormRejection>,
) -> Response {
    let form = form.map(|Form(f)| f).ok();
    let lang = form.as_ref().and_then(|f| f.lang.as_deref());
    let visit = Visit::new(&extensions, &headers, lang);
    let lang = visit.lang;
    let t = lang.texts();
    if let Some(r) = state.admit(&visit, Bucket::Post, state.limits().posts_per_source) {
        return r;
    }
    if foreign_origin(&state.app, &headers) {
        tracing::info!("booking from a foreign origin refused");
        return html::message(lang, StatusCode::FORBIDDEN, t.invalid_title, t.invalid_text);
    }
    let opening = match state.lookup(&token, &visit).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    let Some(form) = form else {
        return invalid(lang);
    };
    let tz = guest_zone(form.tz.as_deref(), &opening);
    let ctx = ctx_of(&opening, lang, tz);
    if form
        .website
        .as_deref()
        .is_some_and(|w| !w.trim().is_empty())
    {
        tracing::info!(opening = %opening.id, "booking refused: honeypot filled");
        return invalid(lang);
    }
    let Some(start) = form.slot.as_deref().and_then(parse_instant) else {
        return invalid(lang);
    };
    let end = start + Duration::minutes(opening.duration_minutes);
    let slot = Slot {
        start,
        end,
        suggestion_id: None,
    };
    let entered = Entered {
        name: form.name.clone().unwrap_or_default(),
        email: form.email.clone().unwrap_or_default(),
        note: form.note.clone().unwrap_or_default(),
    };
    let now_ms = visit.now_ms();
    let again = |errors: &[&str]| {
        let token = state.form_token(&opening, start, now_ms);
        html::form_page(
            &ctx,
            &slot,
            &token,
            &entered,
            errors,
            StatusCode::BAD_REQUEST,
        )
    };
    match guard::check_form(
        &state.app.config().secret_key,
        form.form.as_deref().unwrap_or(""),
        &opening.token,
        start.timestamp_millis(),
        now_ms,
        state.limits(),
    ) {
        Ok(()) => {}
        Err(FormError::Invalid) => {
            tracing::info!(opening = %opening.id, "booking refused: invalid form token");
            return invalid(lang);
        }
        Err(FormError::TooFast) => {
            tracing::info!(opening = %opening.id, "booking refused: form sent too fast");
            return again(&[t.resend]);
        }
        Err(FormError::Expired) => return again(&[t.expired]),
    }
    let name = guard::check_name(&entered.name);
    let email = guard::check_email(&entered.email);
    let note = guard::check_note(&entered.note);
    let (name, email, note) = match (name, email, note) {
        (Ok(n), Ok(e), Ok(o)) => (n, e, o),
        (n, e, o) => {
            let errors: Vec<&str> = [n.err(), e.err(), o.err()]
                .into_iter()
                .flatten()
                .map(|err| match err {
                    InputError::Name => t.name_invalid,
                    InputError::Email => t.email_invalid,
                    InputError::Note => t.note_invalid,
                })
                .collect();
            return again(&errors);
        }
    };

    // From here on one booking at a time: limits, the last check and the
    // write see every earlier booking.
    let guard = state.app.opening_booking_guard().await;
    let limits = state.limits();
    let email_key = email.to_lowercase();
    let mut counted = vec![
        (
            Bucket::BookingSource,
            visit.source.as_str(),
            limits.bookings_per_source,
        ),
        (
            Bucket::BookingLink,
            opening.id.as_str(),
            limits.bookings_per_link,
        ),
        (
            Bucket::BookingEmail,
            email_key.as_str(),
            limits.bookings_per_email,
        ),
        (Bucket::BookingTotal, "all", limits.bookings_total),
    ];
    if let Some(net) = &visit.net {
        counted.push((Bucket::BookingNet, net.as_str(), limits.bookings_per_net));
    }
    if let Some((bucket, _, _)) = counted
        .iter()
        .find(|(b, key, limit)| !state.limiter.allows(*b, key, *limit, now_ms))
    {
        tracing::warn!(opening = %opening.id, bucket = ?bucket, "booking limit reached");
        return html::too_many(lang, RETRY_LONG_SECS);
    }
    let slot = match state.bookable(&opening, start, visit.now).await {
        Ok(Some(slot)) => slot,
        Ok(None) => {
            let from = start.with_timezone(&tz).date_naive().format("%Y-%m-%d");
            return state
                .list(
                    &ctx,
                    &opening,
                    Some(&from.to_string()),
                    visit.now,
                    Some(t.taken),
                    StatusCode::CONFLICT,
                )
                .await;
        }
        Err(e) => {
            tracing::warn!(opening = %opening.id, error = %e, "slot check failed");
            return unavailable(lang);
        }
    };
    match state.organizer(&opening.calendar_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            tracing::error!(opening = %opening.id, calendar = %opening.calendar_id,
                "booking refused: the target account has no own address to invite from");
            return unavailable(lang);
        }
        Err(e) => {
            tracing::error!(opening = %opening.id, error = %e, "organizer lookup failed");
            return unavailable(lang);
        }
    }
    let mutation = booking_mutation(&opening, &slot, &name, &email);
    let event_id = match state.app.apply_mutation(&Origin::Server, &mutation).await {
        Ok(WriteOutcome::Applied { events, .. }) => {
            match events.first().and_then(|e| e["id"].as_str()) {
                Some(id) => id.to_owned(),
                None => {
                    tracing::error!(opening = %opening.id, "booking written without event");
                    return unavailable(lang);
                }
            }
        }
        Ok(other) => {
            tracing::warn!(opening = %opening.id, outcome = %other.status(),
                "booking not written");
            return unavailable(lang);
        }
        Err(e) => {
            tracing::warn!(opening = %opening.id, error = %e, "booking not written");
            return unavailable(lang);
        }
    };
    let guest = Guest {
        name,
        email: email.clone(),
        note,
    };
    let queued = mark_booked(&state.app, &opening.id, &event_id, &slot, &guest, visit.now).await;
    for (bucket, key, _) in counted {
        state.limiter.hit(bucket, key, now_ms);
    }
    // The placeholders go at the provider without holding up other
    // bookings.
    drop(guard);
    tracing::info!(opening = %opening.id, event = %event_id, "booking created");
    if queued {
        state.app.remove_booked_placeholders().await;
    }
    html::success_page(&ctx, slot.start, slot.end, &email)
}

/// Marks the link booked after its event was written ([`App::mark_opening_booked`]),
/// trying once more if that fails: the event exists and the invitation is
/// out, so a one-off link left unmarked would stay bookable. `true` if
/// placeholders wait for removal.
async fn mark_booked(
    app: &App,
    opening_id: &str,
    event_id: &str,
    slot: &Slot,
    guest: &Guest,
    now: DateTime<Utc>,
) -> bool {
    let booking = Some((slot, guest));
    let mut result = app
        .mark_opening_booked(opening_id, event_id, booking, now)
        .await;
    if let Err(e) = &result {
        if !matches!(e, crate::error::AppError::Conflict(_)) {
            tracing::warn!(opening = %opening_id, event = %event_id, error = %e,
                "marking the link booked failed, trying again");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            result = app
                .mark_opening_booked(opening_id, event_id, booking, now)
                .await;
        }
    }
    match result {
        Ok(queued) => queued,
        // Booked already (cannot happen under the booking guard): it is
        // marked, only not with this event.
        Err(e @ crate::error::AppError::Conflict(_)) => {
            tracing::error!(opening = %opening_id, event = %event_id, error = %e,
                "booking written for a link that was booked already");
            false
        }
        Err(e) => {
            tracing::error!(opening = %opening_id, event = %event_id, error = %e,
                "BOOKING WRITTEN, BUT THE LINK WAS NOT MARKED BOOKED: a one-off link stays \
                 bookable until it is deleted");
            false
        }
    }
}

/// The `create_event` mutation of a booking: in the link's zone, the
/// link's title, the guest as attendee (the write path adds `ORGANIZER`
/// with the own address, `PARTSTAT=NEEDS-ACTION`, `RSVP=TRUE`), the video
/// link as place, URL and description. Nothing else the guest entered: the
/// provider sends this event to the guest's address (module docs).
fn booking_mutation(opening: &Opening, slot: &Slot, name: &str, email: &str) -> serde_json::Value {
    let tz = Tz::from_str(&opening.time_zone).unwrap_or(Tz::UTC);
    let local = |t: DateTime<Utc>| {
        json!({
            "date_time": t.with_timezone(&tz).format("%Y-%m-%dT%H:%M:%S").to_string(),
            "time_zone": tz.name(),
        })
    };
    let mut event = json!({
        "title": opening.title,
        "start": local(slot.start),
        "end": local(slot.end),
        "availability": "busy",
        "attendees": [{"email": email, "name": name}],
    });
    if let Some(v) = &opening.video_url {
        event["location"] = json!(v);
        event["url"] = json!(v);
        event["notes"] = json!(format!("Video: {v}"));
    }
    json!({
        "op": "create_event",
        "calendar_id": opening.calendar_id,
        "event": event,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_checked_for_shape() {
        assert!(plausible_token(&crate::auth::random_secret()));
        assert!(!plausible_token("kurz"));
        assert!(!plausible_token(&"a".repeat(42)));
        assert!(!plausible_token(&format!("{}!", "a".repeat(42))));
    }

    #[test]
    fn day_starts_follow_the_zone() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        assert_eq!(
            day_start(d, berlin).to_rfc3339(),
            "2026-11-02T23:00:00+00:00"
        );
        assert_eq!(
            day_start(d, Tz::UTC).to_rfc3339(),
            "2026-11-03T00:00:00+00:00"
        );
    }

    #[test]
    fn a_day_without_midnight_starts_after_the_gap() {
        use chrono::TimeZone;
        // Chile starts summer time at 00:00 (2026-09-06: 00:00 → 01:00).
        let santiago: Tz = "America/Santiago".parse().unwrap();
        let d = NaiveDate::from_ymd_opt(2026, 9, 6).unwrap();
        let midnight = d.and_time(NaiveTime::MIN);
        assert!(
            santiago.from_local_datetime(&midnight).earliest().is_none(),
            "midnight does not exist that day"
        );
        let start = day_start(d, santiago);
        assert_eq!(start.to_rfc3339(), "2026-09-06T04:00:00+00:00");
        assert_eq!(
            start.with_timezone(&santiago).to_rfc3339(),
            "2026-09-06T01:00:00-03:00"
        );
        assert_eq!(
            day_start(d.pred_opt().unwrap(), santiago).to_rfc3339(),
            "2026-09-05T04:00:00+00:00"
        );
    }
}
