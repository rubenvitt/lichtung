//! iCloud specifics (LIC-13) against invented, iCloud-style responses:
//! discovery that ends on a partition host, the calendar listing with
//! reminders, scheduling collections, notifications and shared calendars,
//! sync with iCloud tokens, and the error statuses iCloud uses.
//!
//! Everything is made up: DSIDs, names, addresses (example.org), tokens.
//! Production iCloud runs on `https://caldav.icloud.com` and
//! `https://pNN-caldav.icloud.com:443`; credentials reach the partition host
//! through the parent-domain rule (unit test in `client.rs`). The plain HTTP
//! test servers here are trusted explicitly instead.

use std::time::Duration;

use lichtung_caldav::{Client, Credentials, Error, Sharing};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const USER: &str = "anna@example.org";
/// Invented app-specific password.
const APP_PASSWORD: &str = "erfundenes-app-passwort";
/// Invented "normal" Apple ID password (refused for CalDAV).
const APPLE_ID_PASSWORD: &str = "kein-app-passwort";
const DSID: &str = "1234567890";
const XML: &str = "application/xml; charset=utf-8";

fn auth() -> String {
    use base64::Engine;
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{APP_PASSWORD}"));
    format!("Basic {token}")
}

fn ms(status: u16, body: impl Into<String>) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.into(), XML)
}

fn home_path() -> String {
    format!("/{DSID}/calendars/")
}

fn cal_path(name: &str) -> String {
    format!("/{DSID}/calendars/{name}/")
}

/// Entry host (`caldav.icloud.com`) and partition host (`pNN-caldav`).
struct Icloud {
    entry: MockServer,
    partition: MockServer,
}

impl Icloud {
    async fn start() -> Self {
        let entry = MockServer::start().await;
        let partition = MockServer::start().await;
        let me = Icloud { entry, partition };
        me.mount_discovery().await;
        // Anything without the right credentials is refused.
        for server in [&me.entry, &me.partition] {
            Mock::given(method("PROPFIND"))
                .respond_with(
                    ResponseTemplate::new(401)
                        .insert_header("WWW-Authenticate", "Basic realm=\"x\""),
                )
                .with_priority(10)
                .mount(server)
                .await;
        }
        me
    }

    fn client(&self) -> Client {
        Client::builder(format!("{}/", self.entry.uri()))
            .credentials(Credentials::basic(USER, APP_PASSWORD))
            .trust_origin(&self.partition.uri())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    /// `/.well-known/caldav` → 301 to the root of the entry host (with an
    /// explicit port, like iCloud's `https://caldav.icloud.com:443/`),
    /// principal on the entry host, calendar home on the partition host.
    async fn mount_discovery(&self) {
        let entry = &self.entry;
        Mock::given(method("PROPFIND"))
            .and(path("/.well-known/caldav"))
            .respond_with(
                ResponseTemplate::new(301).insert_header("Location", format!("{}/", entry.uri())),
            )
            .mount(entry)
            .await;
        Mock::given(method("PROPFIND"))
            .and(path("/"))
            .and(header("authorization", auth().as_str()))
            .respond_with(ms(
                207,
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?><multistatus xmlns="DAV:"><response><href>/</href><propstat><prop><current-user-principal><href>/{DSID}/principal/</href></current-user-principal></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#
                ),
            ))
            .mount(entry)
            .await;
        let p = self.partition.uri();
        Mock::given(method("PROPFIND"))
            .and(path(format!("/{DSID}/principal/")))
            .and(header("authorization", auth().as_str()))
            .and(body_string_contains("calendar-home-set"))
            .respond_with(ms(
                207,
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<multistatus xmlns="DAV:"><response><href>/{DSID}/principal/</href><propstat><prop>
<calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"><href xmlns="DAV:">{p}/{DSID}/calendars/</href></calendar-home-set>
<calendar-user-address-set xmlns="urn:ietf:params:xml:ns:caldav"><href xmlns="DAV:" preferred="1">mailto:anna@example.org</href><href xmlns="DAV:">urn:uuid:00000000-0000-4000-8000-000000000001</href><href xmlns="DAV:">/{DSID}/principal/</href></calendar-user-address-set>
<schedule-inbox-URL xmlns="urn:ietf:params:xml:ns:caldav"><href xmlns="DAV:">{p}/{DSID}/calendars/inbox/</href></schedule-inbox-URL>
<schedule-outbox-URL xmlns="urn:ietf:params:xml:ns:caldav"><href xmlns="DAV:">{p}/{DSID}/calendars/outbox/</href></schedule-outbox-URL>
<displayname>Anna Beispiel</displayname>
</prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#
                ),
            ))
            .mount(entry)
            .await;
        Mock::given(method("OPTIONS"))
            .and(path(home_path()))
            .respond_with(ResponseTemplate::new(200).insert_header(
                "DAV",
                "1, access-control, calendar-access, calendar-schedule, calendar-auto-schedule, \
                 calendar-availability, inbox-availability, calendar-proxy, calendarserver-sharing, \
                 calendarserver-subscribed, calendarserver-home-sync",
            ))
            .mount(&self.partition)
            .await;
    }

    async fn mount_listing(&self) {
        Mock::given(method("PROPFIND"))
            .and(path(home_path()))
            .and(header("authorization", auth().as_str()))
            .and(header("depth", "1"))
            .respond_with(ms(207, listing()))
            .mount(&self.partition)
            .await;
    }
}

fn response(href: &str, props: &str) -> String {
    format!(
        "<response><href>{href}</href><propstat><prop>{props}</prop>\
         <status>HTTP/1.1 200 OK</status></propstat></response>"
    )
}

const RW: &str = "<current-user-privilege-set><privilege><all/></privilege><privilege><read/></privilege><privilege><write/></privilege><privilege><write-content/></privilege><privilege><bind/></privilege><privilege><unbind/></privilege></current-user-privilege-set>";
const RW_SHARED: &str = "<current-user-privilege-set><privilege><read/></privilege><privilege><read-current-user-privilege-set/></privilege><privilege><write/></privilege><privilege><write-content/></privilege><privilege><write-properties/></privilege><privilege><bind/></privilege><privilege><unbind/></privilege></current-user-privilege-set>";
const RO_SHARED: &str = "<current-user-privilege-set><privilege><read/></privilege><privilege><read-current-user-privilege-set/></privilege><privilege><C:read-free-busy/></privilege></current-user-privilege-set>";

fn calendar(
    name: &str,
    display: &str,
    extra_type: &str,
    comps: &str,
    privileges: &str,
    rest: &str,
) -> String {
    response(
        &cal_path(name),
        &format!(
            "<resourcetype><collection/><C:calendar/>{extra_type}</resourcetype>\
             <displayname>{display}</displayname>{privileges}\
             <C:supported-calendar-component-set>{comps}</C:supported-calendar-component-set>{rest}"
        ),
    )
}

/// Invented iCloud-style PROPFIND Depth 1 on the calendar home: own
/// calendar, calendar shared by me, read-write and read-only shares,
/// reminders, inbox, outbox and notifications.
fn listing() -> String {
    let ev = r#"<C:comp name="VEVENT"/>"#;
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:CS="http://calendarserver.org/ns/" xmlns:A="http://apple.com/ns/ical/">"#,
    );
    out.push_str(&response(
        &home_path(),
        "<resourcetype><collection/></resourcetype>",
    ));
    out.push_str(&calendar(
        "home",
        "Privat",
        "",
        ev,
        RW,
        &format!(
            "<owner><href>/{DSID}/principal/</href></owner>\
             <A:calendar-color symbolic-color=\"blue\">#1BADF8FF</A:calendar-color>\
             <A:calendar-order>1</A:calendar-order>\
             <CS:getctag>HwoQEgwAAAAAAAAAAAAAABgBGAEoAQ==</CS:getctag>\
             <sync-token>FT=-@RU=00000000-0000-4000-8000-000000000001@S=17</sync-token>\
             <C:schedule-calendar-transp><C:opaque/></C:schedule-calendar-transp>"
        ),
    ));
    out.push_str(&calendar(
        "A0000000-0000-4000-8000-00000000000A",
        "Arbeit",
        "<CS:shared-owner/>",
        ev,
        RW,
        &format!(
            "<owner><href>/{DSID}/principal/</href></owner>\
             <A:calendar-color>#CC73E1FF</A:calendar-color><A:calendar-order>2</A:calendar-order>\
             <CS:invite><CS:user><href>mailto:ben@example.org</href><CS:common-name>Ben Beispiel</CS:common-name>\
             <CS:invite-accepted/><CS:access><CS:read/></CS:access></CS:user></CS:invite>"
        ),
    ));
    out.push_str(&calendar(
        "F0000000-0000-4000-8000-00000000000F",
        "Familie",
        "<CS:shared/>",
        ev,
        RW_SHARED,
        "<owner><href>/9876543210/principal/</href></owner>\
         <A:calendar-color>#FF2968FF</A:calendar-color><A:calendar-order>3</A:calendar-order>\
         <CS:invite><CS:organizer><href>/9876543210/principal/</href><CS:common-name>Clara Beispiel</CS:common-name></CS:organizer>\
         <CS:user><href>mailto:anna@example.org</href><CS:invite-accepted/><CS:access><CS:read-write/></CS:access></CS:user></CS:invite>",
    ));
    out.push_str(&calendar(
        "V0000000-0000-4000-8000-00000000000V",
        "Verein",
        "<CS:shared/>",
        ev,
        RO_SHARED,
        "<owner><href>/5555555555/principal/</href></owner>\
         <A:calendar-color>#63DA38FF</A:calendar-color><A:calendar-order>4</A:calendar-order>\
         <CS:invite><CS:organizer><href>/5555555555/principal/</href><CS:common-name>Verein Beispiel</CS:common-name></CS:organizer>\
         <CS:user><href>mailto:anna@example.org</href><CS:invite-accepted/><CS:access><CS:read/></CS:access></CS:user></CS:invite>",
    ));
    // A share recognisable only through `cs:invite` (no `cs:shared` type).
    out.push_str(&calendar(
        "N0000000-0000-4000-8000-00000000000N",
        "Nachbarn",
        "",
        ev,
        RO_SHARED,
        "<CS:invite><CS:organizer><href>/7777777777/principal/</href></CS:organizer></CS:invite>",
    ));
    out.push_str(&calendar(
        "R0000000-0000-4000-8000-00000000000R",
        "Erinnerungen",
        "",
        r#"<C:comp name="VTODO"/>"#,
        RW,
        "<A:calendar-color>#FF9500FF</A:calendar-color>",
    ));
    out.push_str(&response(
        &cal_path("inbox"),
        "<resourcetype><collection/><C:schedule-inbox/></resourcetype>",
    ));
    out.push_str(&response(
        &cal_path("outbox"),
        "<resourcetype><collection/><C:schedule-outbox/></resourcetype>",
    ));
    out.push_str(&response(
        &cal_path("notification"),
        "<resourcetype><collection/><CS:notification/></resourcetype><displayname>notification</displayname>",
    ));
    out.push_str("</multistatus>");
    out
}

#[tokio::test]
async fn discovery_ends_on_the_partition_host() {
    let ic = Icloud::start().await;
    let d = ic.client().discover().await.unwrap();
    assert_eq!(
        d.principal.as_str(),
        format!("{}/{DSID}/principal/", ic.entry.uri())
    );
    assert_eq!(
        d.calendar_home().unwrap().as_str(),
        format!("{}/{DSID}/calendars/", ic.partition.uri())
    );
    assert_eq!(
        d.schedule_inbox.as_ref().unwrap().as_str(),
        format!("{}/{DSID}/calendars/inbox/", ic.partition.uri())
    );
    assert!(d.auto_schedule());
    assert_eq!(d.email_addresses(), vec!["anna@example.org"]);
    assert_eq!(d.display_name.as_deref(), Some("Anna Beispiel"));
    // OPTIONS went to the partition host, with credentials.
    let reqs = ic.partition.received_requests().await.unwrap();
    assert!(reqs.iter().any(|r| r.method.as_str() == "OPTIONS"));
    assert!(reqs.iter().all(
        |r| r.headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(auth().as_str())
    ));
}

#[tokio::test]
async fn calendar_listing_with_shares_reminders_and_scheduling_collections() {
    let ic = Icloud::start().await;
    ic.mount_listing().await;
    let c = ic.client();
    let d = c.discover().await.unwrap();
    let before = ic.entry.received_requests().await.unwrap().len();
    let cals = c
        .list_calendars(d.calendar_home().unwrap().as_str(), Some(&d.principal))
        .await
        .unwrap();
    // Everything after discovery goes to the partition host.
    assert_eq!(ic.entry.received_requests().await.unwrap().len(), before);

    let names: Vec<_> = cals
        .iter()
        .map(|c| c.display_name.clone().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "Privat",
            "Arbeit",
            "Familie",
            "Verein",
            "Nachbarn",
            "Erinnerungen"
        ],
        "inbox, outbox and notification are no calendars"
    );
    let by = |n: &str| {
        cals.iter()
            .find(|c| c.display_name.as_deref() == Some(n))
            .unwrap()
    };

    let privat = by("Privat");
    assert_eq!(
        privat.href.as_str(),
        format!("{}/{DSID}/calendars/home/", ic.partition.uri())
    );
    // DAV:owner resolves on the partition host, the principal lives on the
    // entry host: still the own calendar.
    assert_eq!(privat.sharing, Sharing::NotShared);
    assert!(privat.writable);
    assert_eq!(privat.color.as_deref(), Some("#1BADF8"));
    assert_eq!(privat.order, Some(1));
    assert_eq!(
        privat.sync_token.as_deref(),
        Some("FT=-@RU=00000000-0000-4000-8000-000000000001@S=17")
    );

    let arbeit = by("Arbeit");
    assert_eq!(arbeit.sharing, Sharing::SharedByMe);
    assert!(arbeit.writable);
    assert_eq!(arbeit.color.as_deref(), Some("#CC73E1"));

    let familie = by("Familie");
    assert_eq!(familie.sharing, Sharing::SharedWithMe);
    assert!(familie.writable);
    assert_eq!(familie.color.as_deref(), Some("#FF2968"));

    let verein = by("Verein");
    assert_eq!(verein.sharing, Sharing::SharedWithMe);
    assert!(!verein.writable, "read-only share");
    assert!(verein.supports_events());

    let nachbarn = by("Nachbarn");
    assert_eq!(nachbarn.sharing, Sharing::SharedWithMe);
    assert!(!nachbarn.writable);

    let reminders = by("Erinnerungen");
    assert!(!reminders.supports_events());
    assert_eq!(reminders.components, vec!["VTODO"]);
}

fn sync_answer(items: &str, token: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><multistatus xmlns="DAV:">{items}<sync-token>{token}</sync-token></multistatus>"#
    )
}

fn changed(name: &str, etag: &str) -> String {
    response(
        &format!("{}{name}", cal_path("home")),
        &format!("<getetag>&quot;{etag}&quot;</getetag>"),
    )
}

#[tokio::test]
async fn sync_with_icloud_tokens() {
    let ic = Icloud::start().await;
    let cal = format!("{}{}", ic.partition.uri(), cal_path("home"));
    let t1 = "FT=-@RU=00000000-0000-4000-8000-000000000001@S=17";
    let t2 = "FT=-@RU=00000000-0000-4000-8000-000000000001@S=19";
    Mock::given(method("REPORT"))
        .and(path(cal_path("home")))
        .and(header("authorization", auth().as_str()))
        .and(body_string_contains("<d:sync-token/>"))
        .respond_with(ms(
            207,
            sync_answer(
                &format!(
                    "{}{}",
                    changed("aaaa-1111.ics", "C=17@U=x"),
                    changed("bbbb-2222.ics", "C=16@U=x")
                ),
                t1,
            ),
        ))
        .mount(&ic.partition)
        .await;
    Mock::given(method("REPORT"))
        .and(path(cal_path("home")))
        .and(body_string_contains(format!("<d:sync-token>{t1}</d:sync-token>")))
        .respond_with(ms(
            207,
            sync_answer(
                &format!(
                    "{}<response><href>{}bbbb-2222.ics</href><status>HTTP/1.1 404 Not Found</status></response>",
                    changed("aaaa-1111.ics", "C=19@U=x"),
                    cal_path("home")
                ),
                t2,
            ),
        ))
        .mount(&ic.partition)
        .await;
    Mock::given(method("REPORT"))
        .and(path(cal_path("home")))
        .and(body_string_contains("S=abgelaufen"))
        .respond_with(ms(
            403,
            r#"<?xml version="1.0" encoding="UTF-8"?><error xmlns="DAV:"><valid-sync-token/></error>"#,
        ))
        .mount(&ic.partition)
        .await;

    let c = ic.client();
    let first = c.sync_collection(&cal, None).await.unwrap();
    assert_eq!(first.sync_token, t1);
    assert_eq!(first.changed.len(), 2);
    assert_eq!(first.changed[0].etag.as_deref(), Some("\"C=17@U=x\""));
    assert!(first.deleted.is_empty());

    let next = c.sync_collection(&cal, Some(t1)).await.unwrap();
    assert_eq!(next.sync_token, t2);
    assert_eq!(next.changed.len(), 1);
    assert_eq!(next.deleted.len(), 1);
    assert!(next.deleted[0].as_str().ends_with("/bbbb-2222.ics"));

    let err = c
        .sync_collection(
            &cal,
            Some("FT=-@RU=00000000-0000-4000-8000-000000000001@S=abgelaufen"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::SyncTokenInvalid), "{err:?}");
}

#[tokio::test]
async fn icloud_error_statuses() {
    let ic = Icloud::start().await;

    // Wrong (e.g. the normal Apple ID) password: 401 during discovery.
    let wrong = Client::builder(format!("{}/", ic.entry.uri()))
        .credentials(Credentials::basic(USER, APPLE_ID_PASSWORD))
        .trust_origin(&ic.partition.uri())
        .build()
        .unwrap();
    let err = wrong.discover().await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized), "{err:?}");
    assert!(!err.is_retryable());

    // A share that was withdrawn: 403 on the collection.
    Mock::given(method("REPORT"))
        .and(path(cal_path("F0000000-0000-4000-8000-00000000000F")))
        .respond_with(ms(
            403,
            r#"<?xml version="1.0" encoding="UTF-8"?><error xmlns="DAV:"><need-privileges/></error>"#,
        ))
        .mount(&ic.partition)
        .await;
    let c = ic.client();
    let shared = format!(
        "{}{}",
        ic.partition.uri(),
        cal_path("F0000000-0000-4000-8000-00000000000F")
    );
    let err = c.sync_collection(&shared, None).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden { .. }), "{err:?}");
    assert!(err.to_string().contains("Keine Berechtigung"));

    // Throttled: 503 with Retry-After asks for backoff.
    Mock::given(method("REPORT"))
        .and(path(cal_path("home")))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "120"))
        .mount(&ic.partition)
        .await;
    let home = format!("{}{}", ic.partition.uri(), cal_path("home"));
    let err = c.sync_collection(&home, None).await.unwrap_err();
    match err {
        Error::RateLimited { retry_after } => {
            assert_eq!(retry_after, Some(Duration::from_secs(120)))
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}
