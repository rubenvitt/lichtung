//! WebDAV XML: request body helpers and a namespace-aware multistatus parser
//! that does not care about prefixes.

use url::Url;

use crate::error::{Error, Result};
use crate::href::normalize_href;

pub(crate) const NS_DAV: &str = "DAV:";
pub(crate) const NS_CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
pub(crate) const NS_CS: &str = "http://calendarserver.org/ns/";
pub(crate) const NS_APPLE: &str = "http://apple.com/ns/ical/";

/// XML prolog plus the namespace declarations used by all request bodies.
pub(crate) const NS_DECL: &str = r#"xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/" xmlns:a="http://apple.com/ns/ical/""#;

/// Escapes text for use in XML element content or attribute values.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Owned XML element (namespace URI + local name), detached from the parser.
#[derive(Debug, Clone, Default)]
pub(crate) struct Element {
    pub ns: String,
    pub name: String,
    /// Concatenated direct text children (CDATA included), untrimmed.
    pub text: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
}

impl Element {
    fn from_node(node: roxmltree::Node<'_, '_>) -> Self {
        let mut text = String::new();
        let mut children = Vec::new();
        for child in node.children() {
            if child.is_text() {
                text.push_str(child.text().unwrap_or_default());
            } else if child.is_element() {
                children.push(Element::from_node(child));
            }
        }
        Element {
            ns: node.tag_name().namespace().unwrap_or_default().to_string(),
            name: node.tag_name().name().to_string(),
            text,
            attrs: node
                .attributes()
                .map(|a| (a.name().to_string(), a.value().to_string()))
                .collect(),
            children,
        }
    }

    pub fn is(&self, ns: &str, name: &str) -> bool {
        self.ns == ns && self.name == name
    }

    pub fn child(&self, ns: &str, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.is(ns, name))
    }

    pub fn children_named<'a>(
        &'a self,
        ns: &'a str,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.is(ns, name))
    }

    pub fn has_child(&self, ns: &str, name: &str) -> bool {
        self.child(ns, name).is_some()
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Trimmed text, `None` if empty.
    pub fn text_opt(&self) -> Option<String> {
        let t = self.text.trim();
        (!t.is_empty()).then(|| t.to_string())
    }

    /// Text of the first `DAV:href` child, if any.
    pub fn href_text(&self) -> Option<String> {
        self.child(NS_DAV, "href").and_then(Element::text_opt)
    }
}

/// One `DAV:propstat`: a status and the properties it applies to.
#[derive(Debug, Clone)]
pub(crate) struct PropStat {
    pub status: u16,
    pub props: Vec<Element>,
}

/// One `DAV:response` of a multistatus.
#[derive(Debug, Clone)]
pub(crate) struct DavResponse {
    pub href: Url,
    /// Response-level `DAV:status` (used for deletions, 404, 507 …).
    pub status: Option<u16>,
    pub propstats: Vec<PropStat>,
}

impl DavResponse {
    /// A property reported with a 2xx status, if present.
    pub fn prop(&self, ns: &str, name: &str) -> Option<&Element> {
        self.propstats
            .iter()
            .filter(|ps| (200..300).contains(&ps.status))
            .flat_map(|ps| ps.props.iter())
            .find(|p| p.is(ns, name))
    }

    /// Trimmed text of a 2xx property, `None` if missing or empty.
    pub fn prop_text(&self, ns: &str, name: &str) -> Option<String> {
        self.prop(ns, name).and_then(Element::text_opt)
    }

    /// `true` if `DAV:resourcetype` contains the given element.
    pub fn has_resource_type(&self, ns: &str, name: &str) -> bool {
        self.prop(NS_DAV, "resourcetype")
            .is_some_and(|rt| rt.has_child(ns, name))
    }

    pub fn is_collection(&self) -> bool {
        self.has_resource_type(NS_DAV, "collection")
    }

    /// Effective status: the response-level status, else the first propstat
    /// status, else 200.
    pub fn effective_status(&self) -> u16 {
        self.status
            .or_else(|| self.propstats.first().map(|p| p.status))
            .unwrap_or(200)
    }
}

/// A parsed `DAV:multistatus`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Multistatus {
    pub responses: Vec<DavResponse>,
    /// Top-level `DAV:sync-token` (RFC 6578).
    pub sync_token: Option<String>,
}

/// Parses `HTTP/1.1 404 Not Found` into `404`.
pub(crate) fn parse_status_line(line: &str) -> Option<u16> {
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Parses a multistatus body. Hrefs are resolved against `base` (the URL the
/// request finally went to) and normalized.
pub(crate) fn parse_multistatus(body: &str, base: &Url) -> Result<Multistatus> {
    let doc = roxmltree::Document::parse(body)
        .map_err(|e| Error::Protocol(format!("XML nicht lesbar: {e}")))?;
    let root = doc.root_element();
    if !(root.tag_name().namespace() == Some(NS_DAV) && root.tag_name().name() == "multistatus") {
        return Err(Error::Protocol(format!(
            "multistatus erwartet, {} erhalten",
            root.tag_name().name()
        )));
    }
    let root = Element::from_node(root);
    let mut out = Multistatus {
        sync_token: root.child(NS_DAV, "sync-token").and_then(Element::text_opt),
        ..Default::default()
    };
    for resp in root.children_named(NS_DAV, "response") {
        let status = resp
            .child(NS_DAV, "status")
            .and_then(|s| parse_status_line(&s.text));
        let propstats: Vec<PropStat> = resp
            .children_named(NS_DAV, "propstat")
            .map(|ps| PropStat {
                status: ps
                    .child(NS_DAV, "status")
                    .and_then(|s| parse_status_line(&s.text))
                    .unwrap_or(200),
                props: ps
                    .child(NS_DAV, "prop")
                    .map(|p| p.children.clone())
                    .unwrap_or_default(),
            })
            .collect();
        // RFC 4918 allows several hrefs sharing one status.
        for href in resp.children_named(NS_DAV, "href") {
            let Some(text) = href.text_opt() else {
                continue;
            };
            out.responses.push(DavResponse {
                href: normalize_href(base, &text)?,
                status,
                propstats: propstats.clone(),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_any_prefix_and_propstats() {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<X:multistatus xmlns:X="DAV:" xmlns:Y="urn:ietf:params:xml:ns:caldav">
  <X:response>
    <X:href>/cal/Privat/</X:href>
    <X:propstat>
      <X:prop><X:displayname>Privat</X:displayname></X:prop>
      <X:status>HTTP/1.1 200 OK</X:status>
    </X:propstat>
    <X:propstat>
      <X:prop><Y:calendar-description/></X:prop>
      <X:status>HTTP/1.1 404 Not Found</X:status>
    </X:propstat>
  </X:response>
  <response xmlns="DAV:"><href>/cal/a.ics</href><href>/cal/b.ics</href><status>HTTP/1.1 404 Not Found</status></response>
  <X:sync-token>tok-1</X:sync-token>
</X:multistatus>"#;
        let base = Url::parse("https://dav.example.org/").unwrap();
        let ms = parse_multistatus(body, &base).unwrap();
        assert_eq!(ms.sync_token.as_deref(), Some("tok-1"));
        assert_eq!(ms.responses.len(), 3);
        let r = &ms.responses[0];
        assert_eq!(
            r.prop_text(NS_DAV, "displayname").as_deref(),
            Some("Privat")
        );
        assert!(r.prop(NS_CALDAV, "calendar-description").is_none());
        assert_eq!(ms.responses[2].status, Some(404));
        assert_eq!(ms.responses[2].href.path(), "/cal/b.ics");
    }

    #[test]
    fn keeps_cdata_text() {
        let body = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/x.ics</d:href><d:propstat><d:prop><c:calendar-data><![CDATA[BEGIN:VCALENDAR
END:VCALENDAR
]]></c:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let base = Url::parse("https://dav.example.org/").unwrap();
        let ms = parse_multistatus(body, &base).unwrap();
        let data = &ms.responses[0]
            .prop(NS_CALDAV, "calendar-data")
            .unwrap()
            .text;
        assert_eq!(data, "BEGIN:VCALENDAR\nEND:VCALENDAR\n");
    }

    #[test]
    fn rejects_non_multistatus() {
        let base = Url::parse("https://dav.example.org/").unwrap();
        assert!(parse_multistatus("<html/>", &base).is_err());
        assert!(parse_multistatus("nicht xml", &base).is_err());
    }

    #[test]
    fn escapes() {
        assert_eq!(escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
    }
}
