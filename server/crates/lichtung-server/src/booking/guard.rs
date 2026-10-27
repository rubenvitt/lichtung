//! Protection of the public booking page (LIC-34): rate limits in memory,
//! the signed form token (minimum fill time, bound to link and slot) and
//! the checks of what a guest enters.
//!
//! - **Rate limits** ([`Limiter`]): sliding windows per client address
//!   ([`crate::oauth::routes::client_source`], an IPv6 address counts as
//!   its /64, unknown links and bookings also per /48), per link, per
//!   guest address and in total. One container, so memory is enough; a
//!   restart starts the counts over. The number of keys is capped: when
//!   full, the entries hit longest ago make room (never the total), so a
//!   flood of addresses cannot lock out new visitors.
//! - **Form token** ([`sign_form`], [`check_form`]): `<issued ms>.<HMAC>`
//!   over the link token, the slot start and the time the form was
//!   issued, keyed with a key derived from `LICHTUNG_SECRET_KEY`. A form
//!   sent back sooner than `min_fill` or later than `max_form_age`, or for
//!   another link or slot, is refused.
//! - **Input** ([`check_name`], [`check_email`], [`check_note`]): length
//!   limits, no control characters, no links or addresses in the name.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::auth::constant_time_eq;

/// Longest name of a guest.
pub const MAX_NAME_CHARS: usize = 100;
/// Longest e-mail address (RFC 5321 path limit).
pub const MAX_EMAIL_CHARS: usize = 254;
/// Longest note of a guest.
pub const MAX_NOTE_CHARS: usize = 1_000;
/// At most this many keys are tracked at once; beyond that the entries
/// hit longest ago are dropped (see [`Limiter`]).
const MAX_KEYS: usize = 20_000;
/// How many entries one eviction drops, so a flood does not pay for a
/// full scan on every new key.
const EVICT_BATCH: usize = MAX_KEYS / 10;

const MINUTE_MS: i64 = 60 * 1000;

/// What a limit counts, with its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bucket {
    /// Page views per address (10 minutes).
    View,
    /// Booking attempts per address (10 minutes).
    Post,
    /// Requests with an unknown link per address (1 hour).
    Miss,
    /// Requests with an unknown link per IPv6 /48 (1 hour).
    MissNet,
    /// Bookings per address (24 hours).
    BookingSource,
    /// Bookings per IPv6 /48 (24 hours).
    BookingNet,
    /// Bookings per link (24 hours).
    BookingLink,
    /// Bookings per guest e-mail address (24 hours).
    BookingEmail,
    /// All bookings (24 hours).
    BookingTotal,
}

impl Bucket {
    pub fn window_ms(self) -> i64 {
        match self {
            Bucket::View | Bucket::Post => 10 * MINUTE_MS,
            Bucket::Miss | Bucket::MissNet => 60 * MINUTE_MS,
            Bucket::BookingSource
            | Bucket::BookingNet
            | Bucket::BookingLink
            | Bucket::BookingEmail
            | Bucket::BookingTotal => 24 * 60 * MINUTE_MS,
        }
    }
}

/// The key of a client address: the address, an IPv6 address as its /64,
/// `unknown` without one. An IPv4-mapped IPv6 address counts as IPv4.
pub fn source_key(source: Option<IpAddr>) -> String {
    match source.map(|ip| ip.to_canonical()) {
        Some(IpAddr::V6(v6)) => v6_prefix(v6, 64),
        Some(v4) => v4.to_string(),
        None => "unknown".to_owned(),
    }
}

/// The coarser network of a client address: an IPv6 address as its /48
/// (many /64s are cheap to come by there); `None` for IPv4 and without
/// an address.
pub fn net_key(source: Option<IpAddr>) -> Option<String> {
    match source.map(|ip| ip.to_canonical()) {
        Some(IpAddr::V6(v6)) => Some(v6_prefix(v6, 48)),
        _ => None,
    }
}

fn v6_prefix(v6: std::net::Ipv6Addr, len: u32) -> String {
    let bits = u128::from(v6) & !((1u128 << (128 - len)) - 1);
    format!("{}/{len}", std::net::Ipv6Addr::from(bits))
}

/// Sliding-window counters (timestamps in Unix milliseconds).
///
/// At most [`MAX_KEYS`] keys: a new key in a full map first drops the
/// expired entries, then the [`EVICT_BATCH`] entries hit longest ago
/// (never [`Bucket::BookingTotal`]). A new key is never refused for lack
/// of room; forgetting an old count is the lesser harm than locking out
/// every new visitor, and the total booking limit stays.
#[derive(Debug, Default)]
pub struct Limiter {
    hits: Mutex<HashMap<(Bucket, String), VecDeque<i64>>>,
}

fn make_room(map: &mut HashMap<(Bucket, String), VecDeque<i64>>, now: i64) {
    map.retain(|(b, _), times| times.back().is_some_and(|t| *t > now - b.window_ms()));
    if map.len() < MAX_KEYS {
        return;
    }
    let mut by_age: Vec<(i64, (Bucket, String))> = map
        .iter()
        .filter(|((b, _), _)| *b != Bucket::BookingTotal)
        .map(|(k, times)| (times.back().copied().unwrap_or(i64::MIN), k.clone()))
        .collect();
    let n = EVICT_BATCH.min(by_age.len());
    if n == 0 {
        return;
    }
    by_age.select_nth_unstable_by_key(n - 1, |(last, _)| *last);
    for (_, key) in by_age.into_iter().take(n) {
        map.remove(&key);
    }
    tracing::warn!(
        evicted = n,
        "booking page rate limiter full, oldest entries dropped"
    );
}

impl Limiter {
    /// Runs `f` on the hits of `key` within the window. Without an entry
    /// `f` gets `None`, unless `insert`: then the entry is created, making
    /// room if needed.
    fn with_entry<T>(
        &self,
        bucket: Bucket,
        key: &str,
        now: i64,
        insert: bool,
        f: impl FnOnce(Option<&mut VecDeque<i64>>) -> T,
    ) -> T {
        let mut map = self.hits.lock().unwrap_or_else(PoisonError::into_inner);
        let entry_key = (bucket, key.to_owned());
        if !map.contains_key(&entry_key) {
            if !insert {
                return f(None);
            }
            if map.len() >= MAX_KEYS {
                make_room(&mut map, now);
            }
        }
        let times = map.entry(entry_key).or_default();
        let since = now - bucket.window_ms();
        while times.front().is_some_and(|t| *t <= since) {
            times.pop_front();
        }
        f(Some(times))
    }

    /// Hits of `key` within the window.
    pub fn count(&self, bucket: Bucket, key: &str, now: i64) -> usize {
        self.with_entry(bucket, key, now, false, |times| {
            times.map_or(0, |t| t.len())
        })
    }

    /// Whether one more hit stays within `limit` (nothing is counted).
    pub fn allows(&self, bucket: Bucket, key: &str, limit: u32, now: i64) -> bool {
        self.count(bucket, key, now) < limit as usize
    }

    /// Counts a hit.
    pub fn hit(&self, bucket: Bucket, key: &str, now: i64) {
        self.with_entry(bucket, key, now, true, |times| {
            if let Some(t) = times {
                t.push_back(now);
            }
        });
    }

    /// Counts a hit if it stays within `limit`; `false` = refused.
    pub fn try_hit(&self, bucket: Bucket, key: &str, limit: u32, now: i64) -> bool {
        self.with_entry(bucket, key, now, true, |times| match times {
            Some(t) if t.len() < limit as usize => {
                t.push_back(now);
                true
            }
            _ => false,
        })
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.hits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

// ----- form token ----------------------------------------------------------------

fn form_key(secret_key: &[u8]) -> Hmac<Sha256> {
    let mut derive = Hmac::<Sha256>::new_from_slice(secret_key).expect("any key length");
    derive.update(b"lichtung booking form v1");
    let key = derive.finalize().into_bytes();
    Hmac::<Sha256>::new_from_slice(&key).expect("any key length")
}

fn form_mac(secret_key: &[u8], link_token: &str, slot_ms: i64, issued_ms: i64) -> Vec<u8> {
    let mut mac = form_key(secret_key);
    mac.update(link_token.as_bytes());
    mac.update(b"\0");
    mac.update(slot_ms.to_string().as_bytes());
    mac.update(b"\0");
    mac.update(issued_ms.to_string().as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// The form token for a booking form of `link_token` and the slot
/// starting at `slot_ms`, issued at `issued_ms`.
pub fn sign_form(secret_key: &[u8], link_token: &str, slot_ms: i64, issued_ms: i64) -> String {
    let mac = form_mac(secret_key, link_token, slot_ms, issued_ms);
    format!("{issued_ms}.{}", URL_SAFE_NO_PAD.encode(mac))
}

/// Why a form token was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormError {
    /// Missing, forged or for another link or slot.
    Invalid,
    /// Sent back sooner than the minimum fill time.
    TooFast,
    /// Older than the maximum age.
    Expired,
}

/// Checks a form token against link, slot and the time limits.
pub fn check_form(
    secret_key: &[u8],
    form: &str,
    link_token: &str,
    slot_ms: i64,
    now_ms: i64,
    limits: &crate::config::BookingLimits,
) -> Result<(), FormError> {
    let (issued, mac) = form.split_once('.').ok_or(FormError::Invalid)?;
    let issued: i64 = issued.parse().map_err(|_| FormError::Invalid)?;
    let mac = URL_SAFE_NO_PAD
        .decode(mac)
        .map_err(|_| FormError::Invalid)?;
    let expected = form_mac(secret_key, link_token, slot_ms, issued);
    if !constant_time_eq(&mac, &expected) {
        return Err(FormError::Invalid);
    }
    let age = now_ms - issued;
    let min = i64::try_from(limits.min_fill.as_millis()).unwrap_or(i64::MAX);
    let max = i64::try_from(limits.max_form_age.as_millis()).unwrap_or(i64::MAX);
    if age < min {
        return Err(FormError::TooFast);
    }
    if age > max {
        return Err(FormError::Expired);
    }
    Ok(())
}

// ----- input --------------------------------------------------------------------

/// Why an input was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
    Name,
    Email,
    Note,
}

fn looks_like_link(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("://")
        || lower.contains("www.")
        || lower.contains('@')
        || lower.contains('<')
        || lower.contains('>')
        || lower
            .split(|c: char| c.is_whitespace())
            .any(|word| word.contains('.') && word.contains('/'))
}

/// The guest's name: 1 to [`MAX_NAME_CHARS`] characters, no control
/// characters, no links, addresses or markup.
pub fn check_name(raw: &str) -> Result<String, InputError> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let len = name.chars().count();
    if len == 0 || len > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(InputError::Name);
    }
    if looks_like_link(&name) {
        return Err(InputError::Name);
    }
    Ok(name)
}

fn valid_label(label: &str) -> bool {
    (1..=63).contains(&label.len())
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

/// The guest's e-mail address: `local@domain` in ASCII, at most
/// [`MAX_EMAIL_CHARS`], a domain with at least two labels and a top-level
/// domain of letters (or `xn--`).
pub fn check_email(raw: &str) -> Result<String, InputError> {
    let email = raw.trim();
    if email.is_empty() || email.len() > MAX_EMAIL_CHARS || !email.is_ascii() {
        return Err(InputError::Email);
    }
    let (local, domain) = email.rsplit_once('@').ok_or(InputError::Email)?;
    let local_ok = (1..=64).contains(&local.len())
        && local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+/=?^_`{|}~.-".contains(&b))
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..");
    let labels: Vec<&str> = domain.split('.').collect();
    let tld = labels.last().copied().unwrap_or("");
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|l| valid_label(l))
        && (tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic())
            || tld.starts_with("xn--"));
    if local_ok && domain_ok {
        Ok(email.to_owned())
    } else {
        Err(InputError::Email)
    }
}

/// The optional note: at most [`MAX_NOTE_CHARS`], line breaks and tabs
/// allowed, no other control characters. Empty = none.
pub fn check_note(raw: &str) -> Result<Option<String>, InputError> {
    let note = raw.replace("\r\n", "\n").replace('\r', "\n");
    let note = note.trim();
    if note.chars().count() > MAX_NOTE_CHARS
        || note
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(InputError::Note);
    }
    Ok((!note.is_empty()).then(|| note.to_owned()))
}

/// The address of a request as [`source_key`] (proxy-aware).
pub fn source_of(
    peer: Option<std::net::SocketAddr>,
    headers: &axum::http::HeaderMap,
) -> Option<IpAddr> {
    crate::oauth::routes::client_source(peer, headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BookingLimits;
    use std::time::Duration;

    #[test]
    fn limiter_counts_in_windows() {
        let l = Limiter::default();
        let w = Bucket::View.window_ms();
        assert!(l.try_hit(Bucket::View, "a", 2, 0));
        assert!(l.try_hit(Bucket::View, "a", 2, 1));
        assert!(!l.try_hit(Bucket::View, "a", 2, 2));
        assert!(l.try_hit(Bucket::View, "b", 2, 2), "per key");
        assert!(l.try_hit(Bucket::Post, "a", 2, 2), "per bucket");
        assert!(l.try_hit(Bucket::View, "a", 2, w + 1), "window slides");
        assert_eq!(l.count(Bucket::View, "a", w + 1), 1);
        assert!(l.allows(Bucket::Miss, "x", 1, 0));
        l.hit(Bucket::Miss, "x", 0);
        assert!(!l.allows(Bucket::Miss, "x", 1, 0));
    }

    #[test]
    fn ipv6_counts_per_64() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:bbbb::2".parse().unwrap();
        assert_eq!(source_key(Some(a)), source_key(Some(b)));
        assert_eq!(source_key(Some("192.0.2.7".parse().unwrap())), "192.0.2.7");
        assert_eq!(source_key(None), "unknown");
        assert_eq!(source_key(Some(a)), "2001:db8:1:2::/64");
        let c: IpAddr = "2001:db8:1:ffff::3".parse().unwrap();
        assert_eq!(net_key(Some(a)), net_key(Some(c)), "one /48");
        assert_eq!(net_key(Some(a)).unwrap(), "2001:db8:1::/48");
        assert_eq!(net_key(Some("192.0.2.7".parse().unwrap())), None);
    }

    #[test]
    fn ipv4_mapped_counts_as_ipv4() {
        let mapped: IpAddr = "::ffff:192.0.2.7".parse().unwrap();
        let other: IpAddr = "::ffff:198.51.100.9".parse().unwrap();
        assert_eq!(source_key(Some(mapped)), "192.0.2.7");
        assert_ne!(source_key(Some(mapped)), source_key(Some(other)));
        assert_eq!(net_key(Some(mapped)), None);

        // A listener on [::] sees Traefik as ::ffff:172.x: still a proxy.
        let proxy: std::net::SocketAddr = "[::ffff:172.18.0.2]:40000".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(
            source_of(Some(proxy), &headers),
            Some("172.18.0.2".parse().unwrap())
        );
        headers.insert("x-forwarded-for", "203.0.113.5".parse().unwrap());
        assert_eq!(
            source_of(Some(proxy), &headers),
            Some("203.0.113.5".parse().unwrap())
        );
        headers.insert("x-forwarded-for", "::ffff:198.51.100.9".parse().unwrap());
        assert_eq!(
            source_of(Some(proxy), &headers),
            Some("198.51.100.9".parse().unwrap())
        );
        let direct: std::net::SocketAddr = "[::ffff:198.51.100.77]:40000".parse().unwrap();
        assert_eq!(
            source_of(Some(direct), &headers),
            Some("198.51.100.77".parse().unwrap()),
            "a public peer's header is ignored"
        );
    }

    #[test]
    fn a_full_limiter_evicts_instead_of_refusing() {
        let l = Limiter::default();
        l.hit(Bucket::BookingTotal, "all", 0);
        l.hit(Bucket::Miss, "old", 1);
        for i in 0..MAX_KEYS {
            l.hit(Bucket::View, &format!("flood-{i}"), 10 + i as i64);
        }
        assert!(l.len() <= MAX_KEYS);
        let now = 10 + MAX_KEYS as i64;
        assert!(
            l.try_hit(Bucket::View, "new visitor", 1, now),
            "a new key gets room"
        );
        assert!(
            l.allows(Bucket::Miss, "another visitor", 1, now),
            "an unknown key is not probing"
        );
        assert_eq!(l.count(Bucket::Miss, "old", now), 0, "the oldest went");
        assert_eq!(
            l.count(Bucket::BookingTotal, "all", now),
            1,
            "the total stays"
        );
        assert_eq!(l.count(Bucket::View, "new visitor", now), 1);
    }

    #[test]
    fn form_tokens() {
        let key = [7u8; 32];
        let limits = BookingLimits {
            min_fill: Duration::from_secs(3),
            max_form_age: Duration::from_secs(60),
            ..BookingLimits::default()
        };
        let f = sign_form(&key, "tok", 1_000, 10_000);
        assert_eq!(check_form(&key, &f, "tok", 1_000, 13_000, &limits), Ok(()));
        assert_eq!(
            check_form(&key, &f, "tok", 1_000, 11_000, &limits),
            Err(FormError::TooFast)
        );
        assert_eq!(
            check_form(&key, &f, "tok", 1_000, 80_000, &limits),
            Err(FormError::Expired)
        );
        for (form, token, slot) in [
            (f.as_str(), "other", 1_000),
            (f.as_str(), "tok", 2_000),
            ("", "tok", 1_000),
            ("9000.abc", "tok", 1_000),
        ] {
            assert_eq!(
                check_form(&key, form, token, slot, 13_000, &limits),
                Err(FormError::Invalid)
            );
        }
        assert_eq!(
            check_form(&[8u8; 32], &f, "tok", 1_000, 13_000, &limits),
            Err(FormError::Invalid),
            "other key"
        );
    }

    #[test]
    fn inputs() {
        assert_eq!(check_name("  Bert   Beispiel ").unwrap(), "Bert Beispiel");
        assert_eq!(
            check_name("Zoë O'Neill-Šimić").unwrap(),
            "Zoë O'Neill-Šimić"
        );
        for bad in [
            "",
            "   ",
            "Buy now https://spam.example.org",
            "www.example.org",
            "bert@example.org",
            "example.org/x",
            "<b>Bert</b>",
            "Bert\u{0007}",
        ] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_name(&"x".repeat(MAX_NAME_CHARS + 1)).is_err());

        for ok in [
            "bert@example.org",
            "b.e+r_t@mail.example.org",
            "x@xn--bcher-kva.example",
        ] {
            assert!(check_email(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "bert",
            "bert@",
            "@example.org",
            "bert@example",
            "bert@@example.org",
            "be rt@example.org",
            ".bert@example.org",
            "bert..x@example.org",
            "bert@-example.org",
            "bert@example.o",
            "bert@example.123",
            "bört@example.org",
            "bert@exa mple.org",
        ] {
            assert!(check_email(bad).is_err(), "{bad:?}");
        }

        assert_eq!(check_note("  ").unwrap(), None);
        assert_eq!(
            check_note("Zeile 1\r\nZeile 2").unwrap().unwrap(),
            "Zeile 1\nZeile 2"
        );
        assert!(check_note(&"x".repeat(MAX_NOTE_CHARS + 1)).is_err());
        assert!(check_note("a\u{0000}b").is_err());
    }
}
