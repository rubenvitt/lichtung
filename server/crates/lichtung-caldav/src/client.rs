//! The [`Client`]: configuration, HTTP transport with manual redirect
//! handling and the mapping of HTTP statuses onto [`Error`].

use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, LOCATION, RETRY_AFTER};
use reqwest::Method;
use url::Url;

use crate::error::{summarize_body, Error, Result};
use crate::href::{canonicalize, normalize_href};

/// Maximum number of redirects followed per request.
const MAX_REDIRECTS: usize = 10;
/// Default overall request timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// Default connect timeout.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// `User-Agent` sent with every request.
pub const USER_AGENT: &str = concat!("lichtung/", env!("CARGO_PKG_VERSION"));

/// Basic-auth credentials (e.g. a Fastmail or iCloud app password).
#[derive(Clone)]
pub struct Credentials {
    username: String,
    password: String,
}

impl Credentials {
    /// Username and (app) password for HTTP Basic authentication.
    pub fn basic(username: impl Into<String>, password: impl Into<String>) -> Self {
        Credentials {
            username: username.into(),
            password: password.into(),
        }
    }

    /// The username.
    pub fn username(&self) -> &str {
        &self.username
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Builder for [`Client`].
#[derive(Debug)]
pub struct ClientBuilder {
    base_url: String,
    credentials: Option<Credentials>,
    http: Option<reqwest::Client>,
    timeout: Duration,
    connect_timeout: Duration,
    trusted_origins: Vec<String>,
}

impl ClientBuilder {
    /// Basic-auth credentials. Without them requests are sent anonymously.
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Uses the given `reqwest::Client` instead of building one.
    ///
    /// The client **must** be built with `redirect::Policy::none()`: this
    /// crate follows redirects itself so that it can decide when credentials
    /// may be forwarded to another host. Timeouts and `User-Agent` of an
    /// injected client are its own; see [`Client::default_http_client`].
    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    /// Overall timeout per request (default 60 s). Ignored with an injected client.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Connect timeout (default 15 s). Ignored with an injected client.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Additionally sends credentials to this origin, even over plain HTTP.
    /// Meant for test setups; production hosts are covered by the default
    /// rules (see [`Client`]).
    pub fn trust_origin(mut self, url: &str) -> Self {
        if let Ok(u) = Url::parse(url) {
            self.trusted_origins.push(origin_key(&u));
        }
        self
    }

    /// Builds the client.
    pub fn build(self) -> Result<Client> {
        let mut base = Url::parse(self.base_url.trim())
            .map_err(|e| Error::InvalidInput(format!("Basis-URL {:?}: {e}", self.base_url)))?;
        if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
            return Err(Error::InvalidInput(format!(
                "Basis-URL {:?} ist keine http(s)-URL",
                self.base_url
            )));
        }
        canonicalize(&mut base);
        let http = match self.http {
            Some(http) => http,
            None => Client::default_http_client(self.timeout, self.connect_timeout)?,
        };
        let mut trusted = self.trusted_origins;
        trusted.push(origin_key(&base));
        Ok(Client {
            base,
            credentials: self.credentials,
            http,
            trusted_origins: Mutex::new(trusted),
        })
    }
}

/// Asynchronous CalDAV client for one account.
///
/// All hrefs returned by the client are absolute, normalized [`Url`]s;
/// methods taking an href accept absolute URLs or paths relative to the base
/// URL.
///
/// Credentials are sent to the base URL's origin, to origins configured with
/// [`ClientBuilder::trust_origin`], to HTTPS origins reached through a
/// redirect from a trusted origin (iCloud partition hosts), and to HTTPS
/// hosts sharing the base host's parent domain (`p42-caldav.example.org` for
/// base `caldav.example.org`).
pub struct Client {
    base: Url,
    credentials: Option<Credentials>,
    http: reqwest::Client,
    trusted_origins: Mutex<Vec<String>>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base.as_str())
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
}

/// How an HTTP status maps onto [`Error`] depends a little on what we did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ctx {
    /// Discovery: 403 means the account is not allowed in at all.
    Discovery,
    /// Ordinary reads and writes.
    Normal,
    /// sync-collection: 403/409 with `DAV:valid-sync-token` → token invalid.
    Sync,
}

/// A response that passed the status check.
#[derive(Debug)]
pub(crate) struct RawResponse {
    /// URL after following redirects (base for relative hrefs).
    pub url: Url,
    pub status: u16,
    pub headers: HeaderMap,
    pub body: String,
}

impl RawResponse {
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }
}

impl Client {
    /// Starts a builder for an account at `base_url`
    /// (e.g. `https://caldav.fastmail.com/dav/`).
    pub fn builder(base_url: impl Into<String>) -> ClientBuilder {
        ClientBuilder {
            base_url: base_url.into(),
            credentials: None,
            http: None,
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            trusted_origins: Vec::new(),
        }
    }

    /// Shortcut for a client with Basic-auth credentials and defaults.
    pub fn new(base_url: impl Into<String>, credentials: Credentials) -> Result<Self> {
        Client::builder(base_url).credentials(credentials).build()
    }

    /// The `reqwest::Client` this crate builds by default: rustls, no
    /// automatic redirects, `User-Agent: lichtung/<version>`, the given timeouts.
    pub fn default_http_client(
        timeout: Duration,
        connect_timeout: Duration,
    ) -> Result<reqwest::Client> {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .connect_timeout(connect_timeout)
            .build()
            .map_err(|e| Error::InvalidInput(format!("HTTP-Client: {e}")))
    }

    /// The (normalized) base URL.
    pub fn base_url(&self) -> &Url {
        &self.base
    }

    /// Resolves an href (absolute or relative to the base URL) into a
    /// normalized absolute URL.
    pub fn resolve(&self, href: &str) -> Result<Url> {
        normalize_href(&self.base, href)
            .map_err(|_| Error::InvalidInput(format!("href {href:?} ist ungültig")))
    }

    /// URL for a new resource `<calendar>/<encoded uid>.ics`.
    pub fn new_resource_href(&self, calendar_href: &str, uid: &str) -> Result<Url> {
        crate::href::new_resource_href(&self.resolve(calendar_href)?, uid)
    }

    fn credentials_allowed(&self, url: &Url) -> bool {
        let key = origin_key(url);
        if self
            .trusted_origins
            .lock()
            .map(|t| t.contains(&key))
            .unwrap_or(false)
        {
            return true;
        }
        url.scheme() == "https" && shares_parent_domain(&self.base, url)
    }

    fn trust(&self, url: &Url) {
        if let Ok(mut t) = self.trusted_origins.lock() {
            let key = origin_key(url);
            if !t.contains(&key) {
                t.push(key);
            }
        }
    }

    /// Sends a request, follows redirects and maps error statuses.
    pub(crate) async fn send(
        &self,
        method: Method,
        url: Url,
        headers: &[(&'static str, String)],
        body: Option<(&'static str, String)>,
        ctx: Ctx,
    ) -> Result<RawResponse> {
        let mut method = method;
        let mut url = url;
        let mut body = body;
        for _ in 0..=MAX_REDIRECTS {
            let mut req = self.http.request(method.clone(), url.clone());
            for (name, value) in headers {
                req = req.header(*name, value.as_str());
            }
            if let Some((content_type, content)) = &body {
                req = req
                    .header(CONTENT_TYPE, *content_type)
                    .body(content.clone());
            }
            if let Some(c) = &self.credentials {
                if self.credentials_allowed(&url) {
                    req = req.basic_auth(&c.username, Some(&c.password));
                }
            }
            let resp = req.send().await.map_err(Error::from_reqwest)?;
            let status = resp.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                let location = resp
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        Error::Protocol(format!("Weiterleitung {status} ohne Location"))
                    })?;
                let next = normalize_href(&url, location)?;
                if !matches!(next.scheme(), "http" | "https") {
                    return Err(Error::Protocol(format!(
                        "Weiterleitung auf {} nicht unterstützt",
                        next.scheme()
                    )));
                }
                // Forward credentials to another host only over HTTPS and
                // only when the redirecting host was trusted itself.
                if next.scheme() == "https" && self.credentials_allowed(&url) {
                    self.trust(&next);
                }
                if status == 303 {
                    method = Method::GET;
                    body = None;
                }
                url = next;
                continue;
            }
            let resp_headers = resp.headers().clone();
            let bytes = resp.bytes().await.map_err(Error::from_reqwest)?;
            let raw = RawResponse {
                url,
                status,
                headers: resp_headers,
                body: String::from_utf8_lossy(&bytes).into_owned(),
            };
            return check_status(raw, ctx);
        }
        Err(Error::Protocol("zu viele Weiterleitungen".into()))
    }

    /// PROPFIND with the given depth and `<d:prop>` content; returns the
    /// parsed multistatus and the final URL.
    pub(crate) async fn propfind(
        &self,
        url: Url,
        depth: &'static str,
        props: &str,
        ctx: Ctx,
    ) -> Result<(Url, crate::xml::Multistatus)> {
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><d:propfind {}><d:prop>{props}</d:prop></d:propfind>"#,
            crate::xml::NS_DECL
        );
        self.dav_request("PROPFIND", url, depth, body, ctx).await
    }

    /// REPORT with an XML body; returns the parsed multistatus.
    pub(crate) async fn report(
        &self,
        url: Url,
        depth: &'static str,
        body: String,
        ctx: Ctx,
    ) -> Result<(Url, crate::xml::Multistatus)> {
        self.dav_request("REPORT", url, depth, body, ctx).await
    }

    async fn dav_request(
        &self,
        method: &'static str,
        url: Url,
        depth: &'static str,
        body: String,
        ctx: Ctx,
    ) -> Result<(Url, crate::xml::Multistatus)> {
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|e| Error::Protocol(format!("Methode {method}: {e}")))?;
        let raw = self
            .send(
                method,
                url,
                &[("Depth", depth.to_string())],
                Some(("application/xml; charset=utf-8", body)),
                ctx,
            )
            .await?;
        if raw.status != 207 && raw.body.trim().is_empty() {
            return Err(Error::Protocol(format!(
                "Multistatus erwartet, HTTP {} ohne Inhalt erhalten",
                raw.status
            )));
        }
        let ms = crate::xml::parse_multistatus(&raw.body, &raw.url)?;
        Ok((raw.url, ms))
    }
}

/// Maps non-success statuses onto typed errors.
pub(crate) fn check_status(raw: RawResponse, ctx: Ctx) -> Result<RawResponse> {
    let status = raw.status;
    if (200..300).contains(&status) {
        return Ok(raw);
    }
    let mentions_sync_token = raw.body.contains("valid-sync-token");
    Err(match status {
        401 => Error::Unauthorized,
        403 | 409 if ctx == Ctx::Sync && mentions_sync_token => Error::SyncTokenInvalid,
        403 if ctx == Ctx::Discovery => Error::Unauthorized,
        403 => Error::Forbidden {
            detail: summarize_body(&raw.body),
        },
        404 | 410 => Error::NotFound,
        412 => Error::PreconditionFailed,
        408 => Error::Unavailable {
            reason: "HTTP 408 (Zeitüberschreitung beim Anbieter)".into(),
        },
        429 => Error::RateLimited {
            retry_after: retry_after(&raw.headers),
        },
        503 if raw.headers.contains_key(RETRY_AFTER) => Error::RateLimited {
            retry_after: retry_after(&raw.headers),
        },
        // Not implemented / insufficient storage will not heal by retrying.
        501 | 505 | 507 => Error::Rejected {
            status,
            body: summarize_body(&raw.body),
        },
        500..=599 => Error::Unavailable {
            reason: format!("HTTP {status}"),
        },
        300..=399 => Error::Protocol(format!("Weiterleitung {status} nicht verfolgbar")),
        _ => Error::Rejected {
            status,
            body: summarize_body(&raw.body),
        },
    })
}

/// `Retry-After` as delay-seconds or HTTP-date (RFC 9110 §10.2.3); a date
/// in the past yields zero.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let raw = headers
        .get(RETRY_AFTER)
        .and_then(|v: &HeaderValue| v.to_str().ok())?
        .trim();
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = httpdate::parse_http_date(raw).ok()?;
    Some(
        at.duration_since(std::time::SystemTime::now())
            .unwrap_or(Duration::ZERO),
    )
}

fn origin_key(url: &Url) -> String {
    format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap_or_default().to_ascii_lowercase(),
        url.port_or_known_default().unwrap_or_default()
    )
}

/// `caldav.example.org` and `p42-caldav.example.org` share `example.org`.
/// Requires at least three labels on the base host so that a bare
/// registrable domain never widens to its whole TLD.
fn shares_parent_domain(base: &Url, other: &Url) -> bool {
    let (Some(url::Host::Domain(b)), Some(url::Host::Domain(o))) = (base.host(), other.host())
    else {
        return false;
    };
    let b = b.to_ascii_lowercase();
    let o = o.to_ascii_lowercase();
    let labels: Vec<&str> = b.split('.').collect();
    if labels.len() < 3 {
        return o == b;
    }
    let parent = labels[1..].join(".");
    o == parent || o.ends_with(&format!(".{parent}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(base: &str) -> Client {
        Client::new(base, Credentials::basic("anna", "geheim")).unwrap()
    }

    #[test]
    fn credentials_follow_the_rules() {
        let c = client("https://caldav.example.org/");
        let ok = |u: &str| c.credentials_allowed(&Url::parse(u).unwrap());
        assert!(ok("https://caldav.example.org/dav/"));
        assert!(ok("https://p42-caldav.example.org/123/calendars/"));
        assert!(!ok("http://p42-caldav.example.org/"));
        assert!(!ok("https://caldav.example.net/"));
        assert!(!ok("https://example.org.evil.test/"));
        c.trust(&Url::parse("https://dav.example.net/").unwrap());
        assert!(ok("https://dav.example.net/x"));
    }

    #[test]
    fn plain_http_base_only_trusts_itself() {
        let c = client("http://127.0.0.1:5232/");
        assert!(c.credentials_allowed(&Url::parse("http://127.0.0.1:5232/anna/").unwrap()));
        assert!(!c.credentials_allowed(&Url::parse("http://127.0.0.1:5233/").unwrap()));
    }

    #[test]
    fn two_label_base_does_not_widen() {
        let c = client("https://example.org/");
        assert!(!c.credentials_allowed(&Url::parse("https://other.org/").unwrap()));
        assert!(!c.credentials_allowed(&Url::parse("https://a.org/").unwrap()));
    }

    #[test]
    fn rejects_bad_base() {
        assert!(Client::builder("ftp://example.org/").build().is_err());
        assert!(Client::builder("kein url").build().is_err());
    }

    #[test]
    fn debug_redacts_password() {
        let c = client("https://caldav.example.org/");
        let s = format!("{c:?}");
        assert!(!s.contains("geheim"));
        assert!(s.contains("redacted"));
    }

    fn raw(status: u16, body: &str) -> RawResponse {
        RawResponse {
            url: Url::parse("https://dav.example.org/").unwrap(),
            status,
            headers: HeaderMap::new(),
            body: body.into(),
        }
    }

    #[test]
    fn retry_after_accepts_seconds_and_dates() {
        let mut h = HeaderMap::new();
        h.insert(RETRY_AFTER, HeaderValue::from_static("120"));
        assert_eq!(retry_after(&h), Some(Duration::from_secs(120)));
        let later = std::time::SystemTime::now() + Duration::from_secs(600);
        h.insert(
            RETRY_AFTER,
            HeaderValue::from_str(&httpdate::fmt_http_date(later)).unwrap(),
        );
        let d = retry_after(&h).unwrap();
        assert!(
            d > Duration::from_secs(590) && d <= Duration::from_secs(600),
            "{d:?}"
        );
        h.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        assert_eq!(retry_after(&h), Some(Duration::ZERO));
        h.insert(RETRY_AFTER, HeaderValue::from_static("bald"));
        assert_eq!(retry_after(&h), None);
    }

    #[test]
    fn icloud_partition_hosts_get_credentials() {
        // Explicit :443 as iCloud writes it in calendar-home-set hrefs.
        let c = client("https://caldav.icloud.com/");
        let home = crate::href::normalize_href(
            c.base_url(),
            "https://p42-caldav.icloud.com:443/1234567890/calendars/",
        )
        .unwrap();
        assert_eq!(
            home.as_str(),
            "https://p42-caldav.icloud.com/1234567890/calendars/"
        );
        assert!(c.credentials_allowed(&home));
        let explicit = Url::parse("https://p07-caldav.icloud.com:443/x/").unwrap();
        assert!(c.credentials_allowed(&explicit));
        assert!(!c.credentials_allowed(&Url::parse("http://p42-caldav.icloud.com/").unwrap()));
        assert!(!c.credentials_allowed(&Url::parse("https://icloud.com.example.org/").unwrap()));
    }

    #[test]
    fn status_mapping() {
        let e = |s, b, ctx| check_status(raw(s, b), ctx).unwrap_err();
        assert!(matches!(e(401, "", Ctx::Normal), Error::Unauthorized));
        assert!(matches!(e(403, "", Ctx::Discovery), Error::Unauthorized));
        assert!(matches!(e(403, "", Ctx::Normal), Error::Forbidden { .. }));
        let tok = r#"<d:error xmlns:d="DAV:"><d:valid-sync-token/></d:error>"#;
        assert!(matches!(e(403, tok, Ctx::Sync), Error::SyncTokenInvalid));
        assert!(matches!(e(409, tok, Ctx::Sync), Error::SyncTokenInvalid));
        assert!(matches!(
            e(409, tok, Ctx::Normal),
            Error::Rejected { status: 409, .. }
        ));
        assert!(matches!(e(412, "", Ctx::Normal), Error::PreconditionFailed));
        assert!(matches!(e(404, "", Ctx::Normal), Error::NotFound));
        assert!(matches!(e(429, "", Ctx::Normal), Error::RateLimited { .. }));
        assert!(matches!(e(502, "", Ctx::Normal), Error::Unavailable { .. }));
        assert!(matches!(
            e(507, "", Ctx::Normal),
            Error::Rejected { status: 507, .. }
        ));
        assert!(matches!(
            e(415, "", Ctx::Normal),
            Error::Rejected { status: 415, .. }
        ));
    }
}
