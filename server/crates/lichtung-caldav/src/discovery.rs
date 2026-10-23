//! Account discovery (RFC 6764, RFC 4791 §6, RFC 6638 §2).

use reqwest::Method;
use url::Url;

use crate::client::{Client, Ctx};
use crate::error::{Error, Result};
use crate::href::normalize_href;
use crate::xml::{Element, NS_CALDAV, NS_DAV};

/// What discovery found out about the account.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Discovery {
    /// `DAV:current-user-principal`.
    pub principal: Url,
    /// `CALDAV:calendar-home-set` (usually exactly one).
    pub calendar_homes: Vec<Url>,
    /// `CALDAV:calendar-user-address-set`, verbatim (`mailto:…`, URNs, …).
    pub calendar_user_addresses: Vec<String>,
    /// `CALDAV:schedule-inbox-URL` (RFC 6638).
    pub schedule_inbox: Option<Url>,
    /// `CALDAV:schedule-outbox-URL` (RFC 6638).
    pub schedule_outbox: Option<Url>,
    /// `DAV:displayname` of the principal.
    pub display_name: Option<String>,
    /// Compliance classes from the `DAV` header (lower-case), e.g.
    /// `calendar-access`, `calendar-auto-schedule`.
    pub dav_capabilities: Vec<String>,
}

impl Discovery {
    /// The first calendar home, if any.
    pub fn calendar_home(&self) -> Option<&Url> {
        self.calendar_homes.first()
    }

    /// `true` if the server announced server-side scheduling (RFC 6638).
    pub fn auto_schedule(&self) -> bool {
        self.has_capability("calendar-auto-schedule")
    }

    /// `true` if the `DAV` header contained the given class (case-insensitive).
    pub fn has_capability(&self, class: &str) -> bool {
        self.dav_capabilities
            .iter()
            .any(|c| c.eq_ignore_ascii_case(class))
    }

    /// `mailto:` addresses of the user without the scheme.
    pub fn email_addresses(&self) -> Vec<String> {
        self.calendar_user_addresses
            .iter()
            .filter_map(|a| {
                let (scheme, rest) = a.split_once(':')?;
                scheme
                    .eq_ignore_ascii_case("mailto")
                    .then(|| rest.to_string())
            })
            .collect()
    }
}

impl Client {
    /// Full discovery: principal, calendar home, scheduling URLs and the
    /// server's DAV capabilities.
    ///
    /// Tries `/.well-known/caldav` first (following redirects, including to
    /// other hosts), then the base URL itself.
    pub async fn discover(&self) -> Result<Discovery> {
        let principal = self.find_principal().await?;
        let (final_url, ms) = self
            .propfind(
                principal.clone(),
                "0",
                "<c:calendar-home-set/><c:calendar-user-address-set/>\
                 <c:schedule-inbox-URL/><c:schedule-outbox-URL/><d:displayname/>",
                Ctx::Discovery,
            )
            .await?;
        let resp = ms
            .responses
            .first()
            .ok_or_else(|| Error::Protocol("Principal ohne Antwort".into()))?;
        let hrefs = |name: &str| -> Result<Vec<Url>> {
            resp.prop(NS_CALDAV, name)
                .map(|p| {
                    p.children_named(NS_DAV, "href")
                        .filter_map(Element::text_opt)
                        .map(|h| normalize_href(&final_url, &h))
                        .collect()
                })
                .unwrap_or_else(|| Ok(Vec::new()))
        };
        let calendar_homes = hrefs("calendar-home-set")?;
        let calendar_user_addresses = resp
            .prop(NS_CALDAV, "calendar-user-address-set")
            .map(|p| {
                p.children_named(NS_DAV, "href")
                    .filter_map(Element::text_opt)
                    .collect()
            })
            .unwrap_or_default();
        let schedule_inbox = hrefs("schedule-inbox-URL")?.into_iter().next();
        let schedule_outbox = hrefs("schedule-outbox-URL")?.into_iter().next();
        let display_name = resp.prop_text(NS_DAV, "displayname");

        let options_target = calendar_homes.first().unwrap_or(&principal).clone();
        let dav_capabilities = self.dav_capabilities(options_target).await;

        Ok(Discovery {
            principal: resp.href.clone(),
            calendar_homes,
            calendar_user_addresses,
            schedule_inbox,
            schedule_outbox,
            display_name,
            dav_capabilities,
        })
    }

    /// Finds `DAV:current-user-principal` via `/.well-known/caldav` or the
    /// base URL.
    pub async fn find_principal(&self) -> Result<Url> {
        let mut well_known = self.base_url().clone();
        well_known.set_path("/.well-known/caldav");
        well_known.set_query(None);
        let mut candidates = vec![well_known];
        if !candidates.contains(self.base_url()) {
            candidates.push(self.base_url().clone());
        }
        let mut last_err = None;
        for candidate in candidates {
            match self
                .propfind(
                    candidate,
                    "0",
                    "<d:current-user-principal/><d:resourcetype/>",
                    Ctx::Discovery,
                )
                .await
            {
                Ok((final_url, ms)) => {
                    let found = ms.responses.iter().find_map(|r| {
                        let p = r.prop(NS_DAV, "current-user-principal")?;
                        if p.has_child(NS_DAV, "unauthenticated") {
                            return Some(Err(Error::Unauthorized));
                        }
                        p.href_text().map(|h| normalize_href(&final_url, &h))
                    });
                    match found {
                        Some(result) => return result,
                        None => {
                            last_err = Some(Error::Protocol(
                                "Server meldet keinen current-user-principal".into(),
                            ))
                        }
                    }
                }
                Err(e @ (Error::Unauthorized | Error::RateLimited { .. })) => return Err(e),
                Err(e) if e.is_retryable() => return Err(e),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Protocol("kein Principal gefunden".into())))
    }

    /// Compliance classes from the `DAV` header of an OPTIONS request.
    /// Failures are ignored (the information is optional).
    pub async fn dav_capabilities(&self, url: Url) -> Vec<String> {
        match self
            .send(Method::OPTIONS, url, &[], None, Ctx::Normal)
            .await
        {
            Ok(raw) => raw
                .headers
                .get_all("dav")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .flat_map(|v| v.split(','))
                .map(|c| c.trim().to_ascii_lowercase())
                .filter(|c| !c.is_empty())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}
