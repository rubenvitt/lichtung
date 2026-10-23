//! Fetching and writing calendar object resources.

use reqwest::Method;
use url::Url;

use crate::client::{Client, Ctx};
use crate::error::{Error, Result};
use crate::href::same_resource;
use crate::xml::{escape, NS_CALDAV, NS_DAV};

/// Number of hrefs per `calendar-multiget` request.
pub const MULTIGET_CHUNK: usize = 100;

/// A calendar object resource with its iCalendar text, untouched.
///
/// Note: data fetched via REPORT travels inside XML, whose parser normalizes
/// CRLF line breaks to LF; [`Client::get`] returns the bytes as served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarObject {
    /// Normalized absolute URL.
    pub href: Url,
    /// ETag exactly as the server sent it.
    pub etag: Option<String>,
    /// iCalendar data.
    pub data: String,
}

/// Result of [`Client::multiget`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MultigetResult {
    /// Resources the server returned with calendar data.
    pub found: Vec<CalendarObject>,
    /// Requested resources the server reported as missing (404) or left out.
    pub missing: Vec<Url>,
}

/// Write precondition for [`Client::put`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Precondition {
    /// Create only: `If-None-Match: *`. Fails with
    /// [`Error::PreconditionFailed`] if the resource exists.
    Create,
    /// Update only the known version: `If-Match: <etag>`. Fails with
    /// [`Error::PreconditionFailed`] if the provider has a newer one.
    IfMatch(String),
}

/// Result of a successful [`Client::put`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PutResult {
    /// Normalized URL of the written resource.
    pub href: Url,
    /// New ETag. Taken from the response header; if the server sent none
    /// (it may have changed the data, RFC 4791 §5.3.4) it was fetched
    /// afterwards. `None` if the server reports none at all.
    pub etag: Option<String>,
    /// `true` if [`PutResult::etag`] came from the PUT response itself, i.e.
    /// the stored data is byte-identical to what was sent. If `false`, the
    /// caller should refetch the data.
    pub etag_from_response: bool,
    /// `Schedule-Tag` (RFC 6638), if the server sent one.
    pub schedule_tag: Option<String>,
}

/// UTC time range in iCalendar form (`20261023T000000Z`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeRange {
    start: String,
    end: String,
}

impl TimeRange {
    /// Builds a range from two UTC date-times like `20261023T000000Z`.
    pub fn new(start: &str, end: &str) -> Result<Self> {
        for v in [start, end] {
            let ok = v.len() == 16
                && v.as_bytes()[8] == b'T'
                && v.ends_with('Z')
                && v[..8].bytes().all(|b| b.is_ascii_digit())
                && v[9..15].bytes().all(|b| b.is_ascii_digit());
            if !ok {
                return Err(Error::InvalidInput(format!(
                    "Zeitpunkt {v:?} ist nicht im Format JJJJMMTTThhmmssZ"
                )));
            }
        }
        if start >= end {
            return Err(Error::InvalidInput(
                "Zeitraum endet vor seinem Beginn".into(),
            ));
        }
        Ok(TimeRange {
            start: start.into(),
            end: end.into(),
        })
    }
}

impl Client {
    /// Fetches resources with `calendar-multiget`, in chunks of
    /// [`MULTIGET_CHUNK`].
    pub async fn multiget<S: AsRef<str>>(
        &self,
        calendar_href: &str,
        hrefs: &[S],
    ) -> Result<MultigetResult> {
        let calendar = self.resolve(calendar_href)?;
        let wanted: Vec<Url> = hrefs
            .iter()
            .map(|h| self.resolve(h.as_ref()))
            .collect::<Result<_>>()?;
        let mut result = MultigetResult::default();
        for chunk in wanted.chunks(MULTIGET_CHUNK) {
            let mut body = format!(
                r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-multiget {}><d:prop><d:getetag/><c:calendar-data/></d:prop>"#,
                crate::xml::NS_DECL
            );
            for href in chunk {
                body.push_str("<d:href>");
                body.push_str(&escape(&href_for_body(&calendar, href)));
                body.push_str("</d:href>");
            }
            body.push_str("</c:calendar-multiget>");
            let (_, ms) = self
                .report(calendar.clone(), "1", body, Ctx::Normal)
                .await?;
            let mut seen: Vec<Url> = Vec::new();
            for r in ms.responses {
                seen.push(r.href.clone());
                let data = r.prop(NS_CALDAV, "calendar-data").map(|d| d.text.clone());
                match (r.status, data) {
                    (Some(s), _) if !(200..300).contains(&s) => result.missing.push(r.href),
                    (_, Some(data)) if !data.trim().is_empty() => {
                        result.found.push(CalendarObject {
                            etag: r.prop_text(NS_DAV, "getetag"),
                            href: r.href,
                            data,
                        })
                    }
                    _ => result.missing.push(r.href),
                }
            }
            for href in chunk {
                if !seen.iter().any(|s| same_resource(s, href)) {
                    result.missing.push(href.clone());
                }
            }
        }
        Ok(result)
    }

    /// Fetches one resource with GET; ETag from the response header.
    pub async fn get(&self, href: &str) -> Result<CalendarObject> {
        let url = self.resolve(href)?;
        let raw = self
            .send(
                Method::GET,
                url.clone(),
                &[("Accept", "text/calendar".into())],
                None,
                Ctx::Normal,
            )
            .await?;
        Ok(CalendarObject {
            etag: raw.header("etag"),
            href: url,
            data: raw.body,
        })
    }

    /// `calendar-query` for events overlapping a time range (recurring
    /// events are matched by the server, data is returned unexpanded).
    pub async fn calendar_query(
        &self,
        calendar_href: &str,
        range: &TimeRange,
    ) -> Result<Vec<CalendarObject>> {
        let calendar = self.resolve(calendar_href)?;
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query {}><d:prop><d:getetag/><c:calendar-data/></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{}" end="{}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#,
            crate::xml::NS_DECL,
            range.start,
            range.end
        );
        let (_, ms) = self
            .report(calendar.clone(), "1", body, Ctx::Normal)
            .await?;
        Ok(ms
            .responses
            .into_iter()
            .filter(|r| (200..300).contains(&r.effective_status()))
            .filter(|r| !same_resource(&r.href, &calendar))
            .filter_map(|r| {
                let data = r.prop(NS_CALDAV, "calendar-data")?.text.clone();
                Some(CalendarObject {
                    etag: r.prop_text(NS_DAV, "getetag"),
                    href: r.href,
                    data,
                })
            })
            .collect())
    }

    /// Writes iCalendar data with a precondition. On 412 the provider's
    /// version wins: [`Error::PreconditionFailed`].
    pub async fn put(
        &self,
        href: &str,
        ics: &str,
        precondition: Precondition,
    ) -> Result<PutResult> {
        let url = self.resolve(href)?;
        let condition = match &precondition {
            Precondition::Create => ("If-None-Match", "*".to_string()),
            Precondition::IfMatch(etag) => ("If-Match", etag.clone()),
        };
        let raw = self
            .send(
                Method::PUT,
                url.clone(),
                &[condition],
                Some(("text/calendar; charset=utf-8", ics.to_string())),
                Ctx::Normal,
            )
            .await?;
        let schedule_tag = raw.header("schedule-tag");
        if let Some(etag) = raw.header("etag") {
            return Ok(PutResult {
                href: url,
                etag: Some(etag),
                etag_from_response: true,
                schedule_tag,
            });
        }
        let (_, ms) = self
            .propfind(
                url.clone(),
                "0",
                "<d:getetag/><c:schedule-tag/>",
                Ctx::Normal,
            )
            .await?;
        let r = ms.responses.first();
        Ok(PutResult {
            etag: r.and_then(|r| r.prop_text(NS_DAV, "getetag")),
            etag_from_response: false,
            schedule_tag: schedule_tag
                .or_else(|| r.and_then(|r| r.prop_text(NS_CALDAV, "schedule-tag"))),
            href: url,
        })
    }

    /// Deletes a resource, optionally only if its ETag still matches.
    pub async fn delete(&self, href: &str, if_match: Option<&str>) -> Result<()> {
        let url = self.resolve(href)?;
        let headers: Vec<(&'static str, String)> = if_match
            .map(|e| vec![("If-Match", e.to_string())])
            .unwrap_or_default();
        self.send(Method::DELETE, url, &headers, None, Ctx::Normal)
            .await?;
        Ok(())
    }

    /// Creates a calendar collection (MKCALENDAR) for events. Mainly for tests
    /// and self-hosted servers; providers usually create calendars elsewhere.
    pub async fn make_calendar(&self, href: &str, display_name: &str) -> Result<Url> {
        let mut url = self.resolve(href)?;
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><c:mkcalendar {}><d:set><d:prop><d:displayname>{}</d:displayname><c:supported-calendar-component-set><c:comp name="VEVENT"/></c:supported-calendar-component-set></d:prop></d:set></c:mkcalendar>"#,
            crate::xml::NS_DECL,
            escape(display_name)
        );
        let method = Method::from_bytes(b"MKCALENDAR")
            .map_err(|e| Error::Protocol(format!("Methode MKCALENDAR: {e}")))?;
        self.send(
            method,
            url.clone(),
            &[],
            Some(("application/xml; charset=utf-8", body)),
            Ctx::Normal,
        )
        .await?;
        Ok(url)
    }
}

/// Path-only href when the resource lives on the collection's origin,
/// otherwise the absolute URL.
fn href_for_body(calendar: &Url, href: &Url) -> String {
    if calendar.origin() == href.origin() {
        href.path().to_string()
    } else {
        href.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_range_validation() {
        assert!(TimeRange::new("20261023T000000Z", "20261030T000000Z").is_ok());
        assert!(TimeRange::new("2026-10-23", "20261030T000000Z").is_err());
        assert!(TimeRange::new("20261030T000000Z", "20261023T000000Z").is_err());
        assert!(TimeRange::new("20261023T000000", "20261030T000000Z").is_err());
    }

    #[test]
    fn body_href_is_path_on_same_origin() {
        let cal = Url::parse("https://dav.example.org/cal/Privat/").unwrap();
        let same = Url::parse("https://dav.example.org/cal/Privat/a%20b.ics").unwrap();
        let other = Url::parse("https://p01.example.org/cal/Privat/a.ics").unwrap();
        assert_eq!(href_for_body(&cal, &same), "/cal/Privat/a%20b.ics");
        assert_eq!(href_for_body(&cal, &other), other.as_str());
    }
}
