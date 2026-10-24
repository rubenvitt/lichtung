//! Tests against synthetic server responses (invented, in the style of
//! Fastmail/Cyrus and iCloud).

use std::time::Duration;

use lichtung_caldav::{Client, Credentials, Error, Precondition, Sharing, TimeRange, Transparency};
use wiremock::matchers::{body_string_contains, header, header_exists, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const USER: &str = "anna@example.org";
const PASSWORD: &str = "app-passwort";

/// Expected `Authorization` header for the invented test credentials,
/// built at runtime so no encoded credential sits in the source.
fn auth() -> String {
    use base64::Engine;
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{PASSWORD}"));
    format!("Basic {token}")
}
const XML: &str = "application/xml; charset=utf-8";
const HOME: &str = "/dav/calendars/user/anna%40example.org/";
const CAL: &str = "/dav/calendars/user/anna%40example.org/Privat/";

fn client(server: &MockServer) -> Client {
    Client::builder(format!("{}/dav/", server.uri()))
        .credentials(Credentials::basic(USER, PASSWORD))
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

fn ms(status: u16, body: impl Into<String>) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.into(), XML)
}

fn multistatus(inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">{inner}</D:multistatus>"#
    )
}

fn etag_response(href: &str, etag: &str) -> String {
    format!(
        "<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:getetag>{etag}</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
    )
}

fn gone(href: &str) -> String {
    format!("<D:response><D:href>{href}</D:href><D:status>HTTP/1.1 404 Not Found</D:status></D:response>")
}

fn ics(uid: &str, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Lichtung//Test//DE\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261023T080000Z\r\nDTSTART:20261024T090000Z\r\nDTEND:20261024T100000Z\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

// ---------------------------------------------------------------- discovery

async fn mount_principal(server: &MockServer, home_href: &str) {
    Mock::given(method("PROPFIND"))
        .and(path("/dav/principals/user/anna@example.org/"))
        .and(header("authorization", auth().as_str()))
        .and(body_string_contains("calendar-home-set"))
        .respond_with(ms(
            207,
            format!(
                r#"<?xml version="1.0"?><x:multistatus xmlns:x="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav">
<x:response><x:href>/dav/principals/user/anna%40example.org/</x:href>
<x:propstat><x:prop>
 <cal:calendar-home-set><x:href>{home_href}</x:href></cal:calendar-home-set>
 <cal:calendar-user-address-set><x:href>mailto:anna@example.org</x:href><x:href>mailto:a.beispiel@example.org</x:href></cal:calendar-user-address-set>
 <cal:schedule-inbox-URL><x:href>/dav/calendars/user/anna%40example.org/Inbox/</x:href></cal:schedule-inbox-URL>
 <cal:schedule-outbox-URL><x:href>/dav/calendars/user/anna%40example.org/Outbox/</x:href></cal:schedule-outbox-URL>
 <x:displayname>Anna Beispiel</x:displayname>
</x:prop><x:status>HTTP/1.1 200 OK</x:status></x:propstat>
</x:response></x:multistatus>"#
            ),
        ))
        .mount(server)
        .await;
}

#[tokio::test]
async fn discovery_follows_well_known_redirect() {
    let server = MockServer::start().await;
    Mock::given(method("PROPFIND"))
        .and(path("/.well-known/caldav"))
        .respond_with(ResponseTemplate::new(301).insert_header("Location", "/dav/"))
        .mount(&server)
        .await;
    Mock::given(method("PROPFIND"))
        .and(path("/dav/"))
        .and(header("authorization", auth().as_str()))
        .and(header("depth", "0"))
        .and(header("user-agent", lichtung_caldav::USER_AGENT))
        .respond_with(ms(
            207,
            multistatus(
                "<D:response><D:href>/dav/</D:href><D:propstat><D:prop><D:current-user-principal><D:href>/dav/principals/user/anna%40example.org/</D:href></D:current-user-principal></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
            ),
        ))
        .mount(&server)
        .await;
    mount_principal(&server, HOME).await;
    Mock::given(method("OPTIONS"))
        .and(path("/dav/calendars/user/anna@example.org/"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("DAV", "1, 2, 3, access-control, extended-mkcol")
                .append_header("DAV", "calendar-access, calendar-auto-schedule"),
        )
        .mount(&server)
        .await;

    let d = client(&server).discover().await.unwrap();
    assert_eq!(
        d.principal.as_str(),
        format!("{}/dav/principals/user/anna@example.org/", server.uri())
    );
    assert_eq!(
        d.calendar_home().unwrap().as_str(),
        format!("{}/dav/calendars/user/anna@example.org/", server.uri())
    );
    assert_eq!(
        d.email_addresses(),
        vec!["anna@example.org", "a.beispiel@example.org"]
    );
    assert!(d
        .schedule_inbox
        .as_ref()
        .unwrap()
        .path()
        .ends_with("/Inbox/"));
    assert!(d
        .schedule_outbox
        .as_ref()
        .unwrap()
        .path()
        .ends_with("/Outbox/"));
    assert_eq!(d.display_name.as_deref(), Some("Anna Beispiel"));
    assert!(d.auto_schedule());
    assert!(d.has_capability("calendar-access"));
}

#[tokio::test]
async fn discovery_falls_back_to_base_url() {
    let server = MockServer::start().await;
    // No well-known: wiremock answers 404.
    Mock::given(method("PROPFIND"))
        .and(path("/dav/"))
        .respond_with(ms(
            207,
            multistatus(
                "<D:response><D:href>/dav/</D:href><D:propstat><D:prop><D:current-user-principal><D:href>/dav/principals/user/anna%40example.org/</D:href></D:current-user-principal></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
            ),
        ))
        .mount(&server)
        .await;
    mount_principal(&server, HOME).await;
    let d = client(&server).discover().await.unwrap();
    assert!(d.calendar_home().is_some());
    assert!(!d.auto_schedule(), "no OPTIONS mock, no capabilities");
}

/// iCloud style: the well-known redirect leads to a partition host. Plain
/// HTTP test servers only receive credentials when trusted explicitly.
async fn icloud_setup() -> (MockServer, MockServer) {
    let entry = MockServer::start().await;
    let partition = MockServer::start().await;
    Mock::given(method("PROPFIND"))
        .and(path("/.well-known/caldav"))
        .respond_with(
            ResponseTemplate::new(301).insert_header("Location", format!("{}/", partition.uri())),
        )
        .mount(&entry)
        .await;
    Mock::given(method("PROPFIND"))
        .and(path("/"))
        .and(header("authorization", auth().as_str()))
        .respond_with(ms(
            207,
            r#"<?xml version="1.0" encoding="UTF-8"?><multistatus xmlns="DAV:"><response><href>/</href><propstat><prop><current-user-principal><href>/123456789/principal/</href></current-user-principal></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#,
        ))
        .mount(&partition)
        .await;
    Mock::given(method("PROPFIND"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&partition)
        .await;
    Mock::given(method("PROPFIND"))
        .and(path("/123456789/principal/"))
        .and(header("authorization", auth().as_str()))
        .respond_with(ms(
            207,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><multistatus xmlns="DAV:"><response><href>/123456789/principal/</href><propstat><prop><calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"><href xmlns="DAV:">{}/123456789/calendars/</href></calendar-home-set><calendar-user-address-set xmlns="urn:ietf:params:xml:ns:caldav"><href xmlns="DAV:">mailto:anna@example.org</href><href xmlns="DAV:">urn:uuid:00000000-0000-4000-8000-000000000000</href></calendar-user-address-set></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#,
                partition.uri()
            ),
        ))
        .mount(&partition)
        .await;
    (entry, partition)
}

#[tokio::test]
async fn discovery_follows_redirect_to_other_host() {
    let (entry, partition) = icloud_setup().await;
    let c = Client::builder(format!("{}/", entry.uri()))
        .credentials(Credentials::basic(USER, PASSWORD))
        .trust_origin(&partition.uri())
        .build()
        .unwrap();
    let d = c.discover().await.unwrap();
    assert_eq!(
        d.principal.as_str(),
        format!("{}/123456789/principal/", partition.uri())
    );
    assert_eq!(
        d.calendar_home().unwrap().as_str(),
        format!("{}/123456789/calendars/", partition.uri())
    );
    assert_eq!(d.email_addresses(), vec!["anna@example.org"]);
    assert_eq!(d.calendar_user_addresses.len(), 2);
}

#[tokio::test]
async fn credentials_are_not_forwarded_to_untrusted_plain_http_host() {
    let (entry, _partition) = icloud_setup().await;
    let c = Client::new(
        format!("{}/", entry.uri()),
        Credentials::basic(USER, PASSWORD),
    )
    .unwrap();
    let err = c.discover().await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized), "{err:?}");
}

#[tokio::test]
async fn unauthorized_is_typed() {
    let server = MockServer::start().await;
    Mock::given(method("PROPFIND"))
        .respond_with(
            ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Basic realm=\"x\""),
        )
        .mount(&server)
        .await;
    let err = client(&server).discover().await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized));
    assert!(!err.is_retryable());
    let err = client(&server)
        .list_calendars(HOME, None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Unauthorized));
}

// ---------------------------------------------------------------- calendars

#[tokio::test]
async fn lists_fastmail_calendars() {
    let server = MockServer::start().await;
    Mock::given(method("PROPFIND"))
        .and(path("/dav/calendars/user/anna@example.org/"))
        .and(header("depth", "1"))
        .and(body_string_contains("current-user-privilege-set"))
        .respond_with(ms(207, include_str!("fixtures/fastmail_calendars.xml")))
        .mount(&server)
        .await;
    let c = client(&server);
    let principal = c
        .resolve("/dav/principals/user/anna%40example.org/")
        .unwrap();
    let cals = c.list_calendars(HOME, Some(&principal)).await.unwrap();
    let names: Vec<_> = cals
        .iter()
        .map(|c| c.display_name.clone().unwrap())
        .collect();
    assert_eq!(names, vec!["Privat", "Team-Sync", "Aufgaben"]);

    let privat = &cals[0];
    assert_eq!(
        privat.href.as_str(),
        format!(
            "{}/dav/calendars/user/anna@example.org/Privat/",
            server.uri()
        )
    );
    assert_eq!(privat.color.as_deref(), Some("#3A7BD5"));
    assert_eq!(privat.order, Some(1));
    assert_eq!(privat.description.as_deref(), Some("Termine zu Hause"));
    assert_eq!(privat.components, vec!["VEVENT", "VTODO"]);
    assert!(privat.supports_events());
    assert!(privat.supports_sync_collection());
    assert!(privat.writable);
    assert_eq!(privat.sharing, Sharing::NotShared);
    assert_eq!(
        privat.sync_token.as_deref(),
        Some("https://example.org/ns/sync/2001")
    );
    assert_eq!(privat.ctag.as_deref(), Some("2001"));
    assert_eq!(privat.transparency, Some(Transparency::Opaque));

    let team = &cals[1];
    assert!(!team.writable);
    assert_eq!(team.sharing, Sharing::SharedWithMe);
    assert_eq!(team.description, None);
    assert_eq!(team.order, None);
    assert_eq!(team.color.as_deref(), Some("#2E8B57"));
    assert_eq!(team.transparency, Some(Transparency::Transparent));
    assert_eq!(
        team.privileges.as_deref(),
        Some(&["read".to_string(), "read-free-busy".to_string()][..])
    );

    let tasks = &cals[2];
    assert!(!tasks.supports_events());
    assert!(tasks.writable, "no privilege set reported: assume writable");
    assert!(tasks.privileges.is_none());
    assert!(!tasks.supports_sync_collection());
}

#[tokio::test]
async fn lists_icloud_calendars_with_owner_comparison() {
    let server = MockServer::start().await;
    let body = include_str!("fixtures/icloud_calendars.xml").replace("{BASE}", &server.uri());
    Mock::given(method("PROPFIND"))
        .and(path("/123456789/calendars/"))
        .respond_with(ms(207, body))
        .mount(&server)
        .await;
    let c = client(&server);
    let principal = c.resolve("/123456789/principal/").unwrap();
    let cals = c
        .list_calendars("/123456789/calendars/", Some(&principal))
        .await
        .unwrap();
    assert_eq!(cals.len(), 2);
    assert_eq!(cals[0].display_name.as_deref(), Some("Privat"));
    assert_eq!(cals[0].color.as_deref(), Some("#1BADF8"));
    assert_eq!(cals[0].order, Some(0));
    assert!(cals[0].writable);
    assert_eq!(cals[0].sharing, Sharing::NotShared);
    assert_eq!(cals[1].display_name.as_deref(), Some("Familie"));
    assert_eq!(cals[1].sharing, Sharing::SharedWithMe);
    assert!(cals[1].writable, "write-content suffices");
}

#[tokio::test]
async fn collection_state_and_ctag() {
    let server = MockServer::start().await;
    Mock::given(method("PROPFIND"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/"))
        .and(header("depth", "0"))
        .respond_with(ms(
            207,
            r#"<multistatus xmlns="DAV:"><response><href>/dav/calendars/user/anna%40example.org/Privat/</href><propstat><prop><getctag xmlns="http://calendarserver.org/ns/">2002</getctag></prop><status>HTTP/1.1 200 OK</status></propstat><propstat><prop><sync-token/></prop><status>HTTP/1.1 404 Not Found</status></propstat></response></multistatus>"#,
        ))
        .mount(&server)
        .await;
    let c = client(&server);
    let state = c.collection_state(CAL).await.unwrap();
    assert_eq!(state.ctag.as_deref(), Some("2002"));
    assert_eq!(state.sync_token, None);
    assert_eq!(c.ctag(CAL).await.unwrap().as_deref(), Some("2002"));
}

// --------------------------------------------------------------------- sync

fn sync_body(inner: &str, token: &str) -> String {
    multistatus(&format!("{inner}<D:sync-token>{token}</D:sync-token>"))
}

#[tokio::test]
async fn sync_initial_and_incremental() {
    let server = MockServer::start().await;
    Mock::given(method("REPORT"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/"))
        .and(body_string_contains("<d:sync-token/>"))
        .and(body_string_contains("<d:sync-level>1</d:sync-level>"))
        .respond_with(ms(
            207,
            sync_body(
                &[
                    etag_response(&format!("{CAL}eins.ics"), "\"e1\""),
                    etag_response(&format!("{CAL}zwei%20b.ics"), "W/\"e2\""),
                    // Some servers list the collection itself; it is ignored.
                    etag_response(CAL, "\"col\""),
                ]
                .concat(),
                "https://example.org/ns/sync/10",
            ),
        ))
        .mount(&server)
        .await;
    Mock::given(method("REPORT"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/"))
        .and(body_string_contains(
            "<d:sync-token>https://example.org/ns/sync/10</d:sync-token>",
        ))
        .respond_with(ms(
            207,
            sync_body(
                &[
                    etag_response(&format!("{}{CAL}eins.ics", server.uri()), "\"e1b\""),
                    gone(&format!("{CAL}zwei b.ics")),
                ]
                .concat(),
                "https://example.org/ns/sync/11",
            ),
        ))
        .mount(&server)
        .await;

    let c = client(&server);
    let first = c.sync_collection(CAL, None).await.unwrap();
    assert_eq!(first.sync_token, "https://example.org/ns/sync/10");
    assert_eq!(first.pages, 1);
    assert!(first.deleted.is_empty());
    let etags: Vec<_> = first
        .changed
        .iter()
        .map(|c| c.etag.clone().unwrap())
        .collect();
    assert_eq!(etags, vec!["\"e1\"", "W/\"e2\""]);

    let next = c
        .sync_collection(CAL, Some(&first.sync_token))
        .await
        .unwrap();
    assert_eq!(next.sync_token, "https://example.org/ns/sync/11");
    assert_eq!(next.changed.len(), 1);
    assert_eq!(
        next.changed[0].href, first.changed[0].href,
        "absolute vs. relative href"
    );
    assert_eq!(next.changed[0].etag.as_deref(), Some("\"e1b\""));
    assert_eq!(
        next.deleted,
        vec![first.changed[1].href.clone()],
        "encoding-independent"
    );
}

#[tokio::test]
async fn sync_follows_truncated_results() {
    let server = MockServer::start().await;
    Mock::given(method("REPORT"))
        .and(body_string_contains("<d:sync-token>t0</d:sync-token>"))
        .respond_with(ms(
            207,
            sync_body(
                &[
                    etag_response(&format!("{CAL}a.ics"), "\"a1\""),
                    etag_response(&format!("{CAL}b.ics"), "\"b1\""),
                    format!("<D:response><D:href>{CAL}</D:href><D:status>HTTP/1.1 507 Insufficient Storage</D:status><D:error><D:number-of-matches-within-limits/></D:error></D:response>"),
                ]
                .concat(),
                "t1",
            ),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("REPORT"))
        .and(body_string_contains("<d:sync-token>t1</d:sync-token>"))
        .respond_with(ms(
            207,
            sync_body(
                &[
                    etag_response(&format!("{CAL}c.ics"), "\"c1\""),
                    gone(&format!("{CAL}b.ics")),
                ]
                .concat(),
                "t2",
            ),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server)
        .sync_collection(CAL, Some("t0"))
        .await
        .unwrap();
    assert_eq!(r.pages, 2);
    assert_eq!(r.sync_token, "t2");
    let changed: Vec<_> = r
        .changed
        .iter()
        .map(|c| c.href.path().rsplit('/').next().unwrap().to_string())
        .collect();
    assert_eq!(changed, vec!["a.ics", "c.ics"]);
    assert_eq!(r.deleted.len(), 1);
    assert!(r.deleted[0].path().ends_with("/b.ics"), "later page wins");
}

#[tokio::test]
async fn sync_truncation_without_progress_is_a_protocol_error() {
    let server = MockServer::start().await;
    Mock::given(method("REPORT"))
        .respond_with(ms(
            207,
            sync_body(
                &format!("<D:response><D:href>{CAL}</D:href><D:status>HTTP/1.1 507 Insufficient Storage</D:status></D:response>"),
                "t0",
            ),
        ))
        .mount(&server)
        .await;
    let err = client(&server)
        .sync_collection(CAL, Some("t0"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Protocol(_)), "{err:?}");
}

#[tokio::test]
async fn invalid_sync_token_is_typed() {
    for status in [403u16, 409] {
        let server = MockServer::start().await;
        Mock::given(method("REPORT"))
            .respond_with(ms(
                status,
                r#"<?xml version="1.0"?><D:error xmlns:D="DAV:"><D:valid-sync-token/></D:error>"#,
            ))
            .mount(&server)
            .await;
        let err = client(&server)
            .sync_collection(CAL, Some("veraltet"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::SyncTokenInvalid), "{status}: {err:?}");
        assert!(!err.is_retryable());
    }
}

#[tokio::test]
async fn sync_without_token_in_answer_is_a_protocol_error() {
    let server = MockServer::start().await;
    Mock::given(method("REPORT"))
        .respond_with(ms(207, multistatus("")))
        .mount(&server)
        .await;
    let err = client(&server)
        .sync_collection(CAL, None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Protocol(_)));
}

#[tokio::test]
async fn list_etags_skips_collections() {
    let server = MockServer::start().await;
    Mock::given(method("PROPFIND"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/"))
        .and(header("depth", "1"))
        .respond_with(ms(
            207,
            multistatus(&format!(
                r#"<D:response><D:href>{CAL}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:calendar/></D:resourcetype><D:getetag/></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>{CAL}eins.ics</D:href><D:propstat><D:prop><D:resourcetype/><D:getetag>"e1"</D:getetag><D:getcontenttype>text/calendar; charset=utf-8</D:getcontenttype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>{CAL}ohne-endung</D:href><D:propstat><D:prop><D:resourcetype/><D:getetag>W/"e2"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>{CAL}unterordner/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"#
            )),
        ))
        .mount(&server)
        .await;
    let etags = client(&server).list_etags(CAL).await.unwrap();
    assert_eq!(etags.len(), 2);
    assert_eq!(etags[0].etag.as_deref(), Some("\"e1\""));
    assert_eq!(etags[1].etag.as_deref(), Some("W/\"e2\""));
}

// -------------------------------------------------------------------- fetch

#[tokio::test]
async fn multiget_reports_missing_and_chunks() {
    let server = MockServer::start().await;
    let uri = server.uri();
    Mock::given(method("REPORT"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/"))
        .and(header("depth", "1"))
        .and(body_string_contains("calendar-multiget"))
        .respond_with(move |req: &Request| {
            // Echo every requested href: odd numbers exist, even ones are gone,
            // `n-149` is silently left out.
            let body = String::from_utf8_lossy(&req.body).to_string();
            let mut inner = String::new();
            for part in body.split("<d:href>").skip(1) {
                let href = part.split("</d:href>").next().unwrap();
                let name = href.rsplit('/').next().unwrap();
                let n: usize = name.trim_start_matches("n-").trim_end_matches(".ics").parse().unwrap();
                if n == 149 {
                    continue;
                }
                if n.is_multiple_of(2) {
                    inner.push_str(&gone(href));
                } else {
                    inner.push_str(&format!(
                        "<D:response><D:href>{uri}{href}</D:href><D:propstat><D:prop><D:getetag>\"{n}\"</D:getetag><C:calendar-data><![CDATA[{}]]></C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
                        ics(&format!("uid-{n}"), "Termin")
                    ));
                }
            }
            ResponseTemplate::new(207).set_body_raw(multistatus(&inner), XML)
        })
        .expect(2)
        .mount(&server)
        .await;
    let hrefs: Vec<String> = (0..150).map(|n| format!("{CAL}n-{n}.ics")).collect();
    let r = client(&server).multiget(CAL, &hrefs).await.unwrap();
    assert_eq!(r.found.len(), 74, "odd numbers without 149");
    assert_eq!(r.missing.len(), 76, "75 gone + 1 left out");
    assert!(r.found[0].data.contains("UID:uid-1"));
    assert_eq!(r.found[0].etag.as_deref(), Some("\"1\""));
    assert!(r.missing.iter().any(|m| m.path().ends_with("/n-149.ics")));
}

#[tokio::test]
async fn get_returns_body_and_etag() {
    let server = MockServer::start().await;
    let data = ics("abc@example.org", "Zahnarzt");
    Mock::given(method("GET"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/abc.ics"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"x-1\"")
                .set_body_raw(data.clone(), "text/calendar; charset=utf-8"),
        )
        .mount(&server)
        .await;
    let c = client(&server);
    let obj = c.get(&format!("{CAL}abc.ics")).await.unwrap();
    assert_eq!(obj.etag.as_deref(), Some("\"x-1\""));
    assert_eq!(obj.data, data, "CRLF preserved");
    let err = c.get(&format!("{CAL}fehlt.ics")).await.unwrap_err();
    assert!(matches!(err, Error::NotFound));
}

#[tokio::test]
async fn calendar_query_sends_time_range() {
    let server = MockServer::start().await;
    Mock::given(method("REPORT"))
        .and(body_string_contains(
            r#"<c:time-range start="20261019T000000Z" end="20261026T000000Z"/>"#,
        ))
        .respond_with(ms(
            207,
            multistatus(&format!(
                "<D:response><D:href>{CAL}w.ics</D:href><D:propstat><D:prop><D:getetag>\"w\"</D:getetag><C:calendar-data>{}</C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
                ics("w@example.org", "Wochentermin")
            )),
        ))
        .mount(&server)
        .await;
    let range = TimeRange::new("20261019T000000Z", "20261026T000000Z").unwrap();
    let objs = client(&server).calendar_query(CAL, &range).await.unwrap();
    assert_eq!(objs.len(), 1);
    assert!(objs[0].data.contains("SUMMARY:Wochentermin"));
}

// -------------------------------------------------------------------- write

#[tokio::test]
async fn put_new_with_if_none_match() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(
            "/dav/calendars/user/anna@example.org/Privat/neu%2F1@example.org.ics",
        ))
        .and(header("if-none-match", "*"))
        .and(header("content-type", "text/calendar; charset=utf-8"))
        .and(header("authorization", auth().as_str()))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("ETag", "\"n1\"")
                .insert_header("Schedule-Tag", "\"s1\""),
        )
        .mount(&server)
        .await;
    let c = client(&server);
    let href = c.new_resource_href(CAL, "neu/1@example.org").unwrap();
    let r = c
        .put(
            href.as_str(),
            &ics("neu/1@example.org", "Neu"),
            Precondition::Create,
        )
        .await
        .unwrap();
    assert_eq!(r.etag.as_deref(), Some("\"n1\""));
    assert!(r.etag_from_response);
    assert_eq!(r.schedule_tag.as_deref(), Some("\"s1\""));
    assert_eq!(r.href, href);
}

#[tokio::test]
async fn put_update_without_etag_header_fetches_it() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/abc.ics"))
        .and(header("if-match", "W/\"alt\""))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("PROPFIND"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/abc.ics"))
        .and(header("depth", "0"))
        .respond_with(ms(
            207,
            multistatus(&format!(
                "<D:response><D:href>{CAL}abc.ics</D:href><D:propstat><D:prop><D:getetag>\"neu\"</D:getetag><C:schedule-tag>\"s2\"</C:schedule-tag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
            )),
        ))
        .mount(&server)
        .await;
    let r = client(&server)
        .put(
            &format!("{CAL}abc.ics"),
            &ics("abc", "Geändert"),
            Precondition::IfMatch("W/\"alt\"".into()),
        )
        .await
        .unwrap();
    assert_eq!(r.etag.as_deref(), Some("\"neu\""));
    assert!(!r.etag_from_response);
    assert_eq!(r.schedule_tag.as_deref(), Some("\"s2\""));
}

#[tokio::test]
async fn put_conflict_is_precondition_failed() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(412))
        .mount(&server)
        .await;
    let err = client(&server)
        .put(
            &format!("{CAL}abc.ics"),
            &ics("abc", "x"),
            Precondition::IfMatch("\"alt\"".into()),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::PreconditionFailed));
    assert_eq!(
        err.to_string(),
        "Konflikt: Der Termin wurde beim Anbieter inzwischen geändert"
    );
}

#[tokio::test]
async fn put_rejected_data_and_read_only_calendar() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/kaputt.ics"))
        .respond_with(ms(
            403,
            r#"<?xml version="1.0"?><D:error xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><C:valid-calendar-data>Missing DTSTART</C:valid-calendar-data></D:error>"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/text.ics"))
        .respond_with(ResponseTemplate::new(415).set_body_string("Unsupported Media Type"))
        .mount(&server)
        .await;
    let c = client(&server);
    let err = c
        .put(
            &format!("{CAL}kaputt.ics"),
            "BEGIN:VCALENDAR",
            Precondition::Create,
        )
        .await
        .unwrap_err();
    match err {
        Error::Forbidden { detail } => {
            assert_eq!(detail, "CALDAV:valid-calendar-data (Missing DTSTART)")
        }
        other => panic!("{other:?}"),
    }
    let err = c
        .put(&format!("{CAL}text.ics"), "x", Precondition::Create)
        .await
        .unwrap_err();
    match err {
        Error::Rejected { status, body } => {
            assert_eq!(status, 415);
            assert_eq!(body, "Unsupported Media Type");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn delete_with_and_without_if_match() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/a.ics"))
        .and(header("if-match", "\"a1\""))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/a.ics"))
        .and(header_exists("if-match"))
        .respond_with(ResponseTemplate::new(412))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/b.ics"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let c = client(&server);
    c.delete(&format!("{CAL}a.ics"), Some("\"a1\""))
        .await
        .unwrap();
    let err = c
        .delete(&format!("{CAL}a.ics"), Some("\"alt\""))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::PreconditionFailed));
    c.delete(&format!("{CAL}b.ics"), None).await.unwrap();
}

// ------------------------------------------------------------- availability

#[tokio::test]
async fn server_errors_are_retryable() {
    let server = MockServer::start().await;
    Mock::given(method("REPORT"))
        .and(path("/dav/calendars/user/anna@example.org/Privat/"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    Mock::given(method("REPORT"))
        .and(path("/dav/calendars/user/anna@example.org/Team-Sync/"))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "120"))
        .mount(&server)
        .await;
    Mock::given(method("REPORT"))
        .and(path("/dav/calendars/user/anna@example.org/Langsam/"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "7"))
        .mount(&server)
        .await;
    let c = client(&server);
    let err = c.sync_collection(CAL, None).await.unwrap_err();
    assert!(matches!(err, Error::Unavailable { .. }));
    assert!(err.is_retryable());
    let err = c
        .sync_collection("/dav/calendars/user/anna%40example.org/Team-Sync/", None)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::RateLimited { retry_after: Some(d) } if d == Duration::from_secs(120)
    ));
    let err = c
        .sync_collection("/dav/calendars/user/anna%40example.org/Langsam/", None)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::RateLimited { retry_after: Some(d) } if d == Duration::from_secs(7)
    ));
    assert!(err.is_retryable());
}

#[tokio::test]
async fn timeouts_and_network_errors_are_retryable() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .mount(&server)
        .await;
    let c = Client::builder(format!("{}/dav/", server.uri()))
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let err = c.get(&format!("{CAL}a.ics")).await.unwrap_err();
    assert!(matches!(err, Error::Unavailable { .. }), "{err:?}");
    assert!(err.is_retryable());

    // Nothing listens on port 9 (discard) of localhost in the test setup.
    let c = Client::builder("http://127.0.0.1:9/dav/").build().unwrap();
    let err = c.get("a.ics").await.unwrap_err();
    assert!(err.is_retryable(), "{err:?}");
}

#[tokio::test]
async fn non_xml_answer_is_a_protocol_error() {
    let server = MockServer::start().await;
    Mock::given(method("PROPFIND"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html><body>Login</body></html>"))
        .mount(&server)
        .await;
    let err = client(&server).list_etags(CAL).await.unwrap_err();
    assert!(matches!(err, Error::Protocol(_)), "{err:?}");
}
