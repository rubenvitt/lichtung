//! Href handling: servers send hrefs relative or absolute and with varying
//! percent-encoding. Everything leaving this crate is an absolute [`Url`]
//! whose path is in one canonical encoding, so equal resources compare equal
//! as strings.

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use url::Url;

use crate::error::{Error, Result};

/// Characters encoded when re-encoding a decoded path segment. Everything
/// that is a valid `pchar` (RFC 3986) stays literal; `/` is encoded because a
/// decoded `%2F` must not turn into a separator.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// Characters encoded in resource names derived from a UID: everything but
/// unreserved characters and `@` (common in UIDs, harmless in paths).
const UID_NAME: &AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'@');

/// Resolves `href` against `base` and brings the path into canonical
/// percent-encoding. Fragments are dropped.
pub fn normalize_href(base: &Url, href: &str) -> Result<Url> {
    let href = href.trim();
    if href.is_empty() {
        return Err(Error::Protocol("leere href".into()));
    }
    let mut url = base
        .join(href)
        .map_err(|e| Error::Protocol(format!("ungültige href {href:?}: {e}")))?;
    canonicalize(&mut url);
    Ok(url)
}

/// Brings an already absolute URL into canonical form (see module docs).
pub fn canonicalize(url: &mut Url) {
    url.set_fragment(None);
    let path = url.path().to_string();
    let canonical = path
        .split('/')
        .map(canonical_segment)
        .collect::<Vec<_>>()
        .join("/");
    if canonical != path {
        url.set_path(&canonical);
    }
}

fn canonical_segment(segment: &str) -> String {
    match percent_decode_str(segment).decode_utf8() {
        Ok(decoded) => utf8_percent_encode(&decoded, SEGMENT).to_string(),
        // Not UTF-8 once decoded: keep the server's bytes untouched.
        Err(_) => segment.to_string(),
    }
}

/// Decoded path of a URL, used for comparisons.
pub fn decoded_path(url: &Url) -> String {
    percent_decode_str(url.path())
        .decode_utf8_lossy()
        .into_owned()
}

/// `true` if both URLs name the same resource: same origin and the same
/// decoded path, ignoring a trailing slash.
pub fn same_resource(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
        && a.port_or_known_default() == b.port_or_known_default()
        && decoded_path(a).trim_end_matches('/') == decoded_path(b).trim_end_matches('/')
}

/// File name (`<encoded uid>.ics`) for a new resource with the given UID.
///
/// Everything except unreserved characters and `@` is percent-encoded, so
/// UIDs containing `/`, spaces or non-ASCII characters are safe.
pub fn resource_name_for_uid(uid: &str) -> Result<String> {
    let uid = uid.trim();
    if uid.is_empty() {
        return Err(Error::InvalidInput("UID ist leer".into()));
    }
    Ok(format!("{}.ics", utf8_percent_encode(uid, UID_NAME)))
}

/// URL of a new resource `<calendar>/<encoded uid>.ics`.
pub fn new_resource_href(calendar: &Url, uid: &str) -> Result<Url> {
    let name = resource_name_for_uid(uid)?;
    let mut collection = calendar.clone();
    if !collection.path().ends_with('/') {
        let path = format!("{}/", collection.path());
        collection.set_path(&path);
    }
    let mut url = collection
        .join(&name)
        .map_err(|e| Error::InvalidInput(format!("Ressourcenname {name:?}: {e}")))?;
    canonicalize(&mut url);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://dav.example.org/dav/calendars/user/anna@example.org/").unwrap()
    }

    #[test]
    fn relative_and_absolute_hrefs_resolve_to_the_same_url() {
        let a = normalize_href(&base(), "/dav/calendars/user/anna%40example.org/Privat/").unwrap();
        let b = normalize_href(
            &base(),
            "https://dav.example.org/dav/calendars/user/anna@example.org/Privat/",
        )
        .unwrap();
        let c = normalize_href(&base(), "Privat/").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(
            a.as_str(),
            "https://dav.example.org/dav/calendars/user/anna@example.org/Privat/"
        );
    }

    #[test]
    fn encoding_is_canonical() {
        let a = normalize_href(&base(), "/cal/Team%2DSync/ein%20termin.ics").unwrap();
        let b = normalize_href(&base(), "/cal/Team-Sync/ein termin.ics").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.path(), "/cal/Team-Sync/ein%20termin.ics");
        // Encoded slash stays encoded, umlauts are encoded consistently.
        let c = normalize_href(&base(), "/cal/a%2fb/M%C3%BCnchen.ics").unwrap();
        let d = normalize_href(&base(), "/cal/a%2Fb/München.ics").unwrap();
        assert_eq!(c, d);
        assert_eq!(c.path(), "/cal/a%2Fb/M%C3%BCnchen.ics");
    }

    #[test]
    fn same_resource_ignores_trailing_slash_and_encoding() {
        let a = Url::parse("https://dav.example.org/cal/Privat").unwrap();
        let b = Url::parse("https://DAV.example.org:443/cal/%50rivat/").unwrap();
        assert!(same_resource(&a, &b));
        let c = Url::parse("https://other.example.org/cal/Privat/").unwrap();
        assert!(!same_resource(&a, &c));
    }

    #[test]
    fn uid_names_are_safe() {
        assert_eq!(
            resource_name_for_uid("abc-123@example.org").unwrap(),
            "abc-123@example.org.ics"
        );
        assert_eq!(
            resource_name_for_uid("a/b c?d#e").unwrap(),
            "a%2Fb%20c%3Fd%23e.ics"
        );
        assert_eq!(resource_name_for_uid("..").unwrap(), "...ics");
        assert!(resource_name_for_uid("  ").is_err());
    }

    #[test]
    fn new_href_is_stable_under_normalization() {
        let cal = Url::parse("https://dav.example.org/cal/Privat").unwrap();
        let href = new_resource_href(&cal, "x/y@example.org").unwrap();
        assert_eq!(
            href.as_str(),
            "https://dav.example.org/cal/Privat/x%2Fy@example.org.ics"
        );
        let again = normalize_href(&cal, href.as_str()).unwrap();
        assert_eq!(href, again);
    }
}
