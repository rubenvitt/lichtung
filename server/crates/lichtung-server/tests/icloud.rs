//! `kind: icloud` end to end (LIC-13) against an invented, stateful
//! iCloud-style CalDAV server: discovery with a calendar home given as an
//! absolute href with explicit port (like `https://pNN-caldav.icloud.com:443`),
//! own and shared calendars (read-write and read-only), reminders, inbox,
//! outbox and notifications, first and incremental sync with iCloud-style
//! tokens, `403 valid-sync-token`, 401 with the app-password hint, and
//! throttling with `503 Retry-After`.
//!
//! All data is made up (DSIDs, names, addresses under example.org). The
//! switch to a different partition host with credentials is covered in
//! `lichtung-caldav` (`tests/icloud.rs`); here one plain HTTP mock serves
//! both roles, because credentials only follow host changes over HTTPS.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use base64::Engine;
use serde_json::{json, Value};
use url::Url;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use common::{Auth, TestServer, BOOTSTRAP};

const USER: &str = "anna@example.org";
/// Invented app-specific password.
const APP_PASSWORD: &str = "erfundenes-app-passwort";
/// Invented "normal" Apple ID password, which iCloud refuses for CalDAV.
const APPLE_ID_PASSWORD: &str = "mein-apple-id-passwort";
/// Invented replacement app password (the old one was revoked).
const NEW_APP_PASSWORD: &str = "neues-erfundenes-app-passwort";
const DSID: &str = "1234567890";
const PRIVAT: &str = "home";
const FAMILIE: &str = "F0000000-0000-4000-8000-00000000000F";
const VEREIN: &str = "V0000000-0000-4000-8000-00000000000V";

fn vevent(uid: &str, title: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//Test//DE\r\nBEGIN:VEVENT\r\n\
         UID:{uid}\r\nDTSTAMP:20261020T070000Z\r\n\
         DTSTART;TZID=Europe/Berlin:20261103T140000\r\nDTEND;TZID=Europe/Berlin:20261103T150000\r\n\
         SUMMARY:{title}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

// ----- the fake iCloud --------------------------------------------------------------

#[derive(Default)]
struct State {
    base: String,
    password: String,
    /// (calendar, resource) → (etag, data).
    resources: BTreeMap<(String, String), (String, String)>,
    /// (change number, calendar, resource).
    log: Vec<(u64, String, String)>,
    counter: u64,
    tokens_invalid: bool,
    throttled: bool,
    throttled_answers: usize,
    sync_reports: usize,
}

#[derive(Clone, Default)]
struct FakeIcloud(Arc<Mutex<State>>);

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn ms(status: u16, body: String) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:CS="http://calendarserver.org/ns/" xmlns:A="http://apple.com/ns/ical/">{body}</multistatus>"#
        ),
        "text/xml; charset=UTF-8",
    )
}

fn ok(href: &str, props: &str) -> String {
    format!(
        "<response><href>{href}</href><propstat><prop>{props}</prop>\
         <status>HTTP/1.1 200 OK</status></propstat></response>"
    )
}

fn gone(href: &str) -> String {
    format!("<response><href>{href}</href><status>HTTP/1.1 404 Not Found</status></response>")
}

fn home() -> String {
    format!("/{DSID}/calendars/")
}

fn cal(name: &str) -> String {
    format!("/{DSID}/calendars/{name}/")
}

fn token(n: u64) -> String {
    format!("FT=-@RU=00000000-0000-4000-8000-000000000001@S={n}")
}

const RW: &str = "<current-user-privilege-set><privilege><all/></privilege><privilege><read/></privilege><privilege><write/></privilege></current-user-privilege-set>";
const RO: &str = "<current-user-privilege-set><privilege><read/></privilege><privilege><read-current-user-privilege-set/></privilege><privilege><C:read-free-busy/></privilege></current-user-privilege-set>";

impl FakeIcloud {
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut self.0.lock().unwrap())
    }

    fn put(&self, calendar: &str, name: &str, data: &str) {
        self.with(|s| {
            s.counter += 1;
            let etag = format!("\"C={}@U=00000000-0000-4000-8000-000000000002\"", s.counter);
            s.resources
                .insert((calendar.into(), name.into()), (etag, data.into()));
            let n = s.counter;
            s.log.push((n, calendar.into(), name.into()));
        });
    }

    fn remove(&self, calendar: &str, name: &str) {
        self.with(|s| {
            s.counter += 1;
            s.resources.remove(&(calendar.to_owned(), name.to_owned()));
            let n = s.counter;
            s.log.push((n, calendar.into(), name.into()));
        });
    }

    fn listing(s: &State) -> String {
        let ev = r#"<C:comp name="VEVENT"/>"#;
        let calendar = |name: &str,
                        display: &str,
                        ty: &str,
                        comps: &str,
                        privs: &str,
                        rest: &str| {
            ok(
                &cal(name),
                &format!(
                    "<resourcetype><collection/><C:calendar/>{ty}</resourcetype>\
                     <displayname>{display}</displayname>{privs}\
                     <C:supported-calendar-component-set>{comps}</C:supported-calendar-component-set>\
                     <CS:getctag>{}</CS:getctag><sync-token>{}</sync-token>{rest}",
                    s.counter,
                    token(s.counter)
                ),
            )
        };
        let mut out = ok(&home(), "<resourcetype><collection/></resourcetype>");
        out.push_str(&calendar(
            PRIVAT,
            "Privat",
            "",
            ev,
            RW,
            &format!(
                "<owner><href>/{DSID}/principal/</href></owner>\
                 <A:calendar-color symbolic-color=\"blue\">#1BADF8FF</A:calendar-color>\
                 <A:calendar-order>1</A:calendar-order>"
            ),
        ));
        out.push_str(&calendar(
            FAMILIE,
            "Familie",
            "<CS:shared/>",
            ev,
            RW,
            "<owner><href>/9876543210/principal/</href></owner>\
             <A:calendar-color>#FF2968FF</A:calendar-color><A:calendar-order>2</A:calendar-order>\
             <CS:invite><CS:organizer><href>/9876543210/principal/</href></CS:organizer></CS:invite>",
        ));
        out.push_str(&calendar(
            VEREIN,
            "Verein",
            "<CS:shared/>",
            ev,
            RO,
            "<owner><href>/5555555555/principal/</href></owner>\
             <A:calendar-color>#63DA38FF</A:calendar-color><A:calendar-order>3</A:calendar-order>",
        ));
        out.push_str(&calendar(
            "R0000000-0000-4000-8000-00000000000R",
            "Erinnerungen",
            "",
            r#"<C:comp name="VTODO"/>"#,
            RW,
            "",
        ));
        out.push_str(&ok(
            &cal("inbox"),
            "<resourcetype><collection/><C:schedule-inbox/></resourcetype>",
        ));
        out.push_str(&ok(
            &cal("outbox"),
            "<resourcetype><collection/><C:schedule-outbox/></resourcetype>",
        ));
        out.push_str(&ok(
            &cal("notification"),
            "<resourcetype><collection/><CS:notification/></resourcetype>",
        ));
        out
    }

    fn respond_to(&self, req: &Request) -> ResponseTemplate {
        let mut s = self.0.lock().unwrap();
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{}", s.password))
        );
        let auth = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        let path = req.url.path().to_owned();
        let method = req.method.as_str().to_owned();
        // iCloud redirects the well-known URL before asking for credentials.
        if method == "PROPFIND" && path == "/.well-known/caldav" {
            return ResponseTemplate::new(301).insert_header("Location", format!("{}/", s.base));
        }
        if auth != Some(expected.as_str()) {
            return ResponseTemplate::new(401)
                .insert_header("WWW-Authenticate", "Basic realm=\"iCloud\"");
        }
        let depth = req
            .headers
            .get("depth")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let body = String::from_utf8_lossy(&req.body).into_owned();
        let calendar = path
            .strip_prefix(&home())
            .and_then(|rest| rest.strip_suffix('/'))
            .filter(|rest| !rest.is_empty() && !rest.contains('/'))
            .map(str::to_owned);
        if s.throttled && calendar.is_some() {
            s.throttled_answers += 1;
            return ResponseTemplate::new(503).insert_header("Retry-After", "120");
        }
        let principal = format!("/{DSID}/principal/");
        match (method.as_str(), path.as_str(), calendar) {
            ("PROPFIND", "/", _) => ms(
                207,
                ok(
                    "/",
                    &format!("<current-user-principal><href>{principal}</href></current-user-principal>"),
                ),
            ),
            ("PROPFIND", p, _) if p == principal => {
                // Calendar home as an absolute href with explicit port.
                let base = &s.base;
                ms(
                    207,
                    ok(
                        &principal,
                        &format!(
                            "<C:calendar-home-set><href>{base}{h}</href></C:calendar-home-set>\
                             <C:calendar-user-address-set><href>mailto:{USER}</href>\
                             <href>urn:uuid:00000000-0000-4000-8000-000000000001</href></C:calendar-user-address-set>\
                             <C:schedule-inbox-URL><href>{base}{h}inbox/</href></C:schedule-inbox-URL>\
                             <C:schedule-outbox-URL><href>{base}{h}outbox/</href></C:schedule-outbox-URL>",
                            h = home()
                        ),
                    ),
                )
            }
            ("OPTIONS", _, _) => ResponseTemplate::new(200).insert_header(
                "DAV",
                "1, access-control, calendar-access, calendar-schedule, calendar-auto-schedule, calendarserver-sharing",
            ),
            ("PROPFIND", p, _) if p == home() => ms(207, Self::listing(&s)),
            ("PROPFIND", _, Some(c)) if depth == "0" => ms(
                207,
                ok(
                    &cal(&c),
                    &format!(
                        "<CS:getctag>{}</CS:getctag><sync-token>{}</sync-token>",
                        s.counter,
                        token(s.counter)
                    ),
                ),
            ),
            ("PROPFIND", _, Some(c)) => {
                let mut out = ok(&cal(&c), "<resourcetype><collection/><C:calendar/></resourcetype>");
                for ((rc, name), (etag, _)) in &s.resources {
                    if *rc == c {
                        out.push_str(&ok(
                            &format!("{}{name}", cal(&c)),
                            &format!("<getetag>{}</getetag>", esc(etag)),
                        ));
                    }
                }
                ms(207, out)
            }
            ("REPORT", _, Some(c)) if body.contains("sync-collection") => {
                s.sync_reports += 1;
                let since = body
                    .split("<d:sync-token>")
                    .nth(1)
                    .and_then(|r| r.split("</d:sync-token>").next())
                    .map(|t| t.rsplit("@S=").next().unwrap_or("").parse::<u64>());
                let since = match since {
                    None => 0,
                    Some(Ok(n)) if !s.tokens_invalid && n <= s.counter => n,
                    Some(_) => {
                        return ResponseTemplate::new(403).set_body_raw(
                            r#"<?xml version="1.0" encoding="UTF-8"?><error xmlns="DAV:"><valid-sync-token/></error>"#,
                            "text/xml; charset=UTF-8",
                        )
                    }
                };
                let names: BTreeSet<String> = s
                    .log
                    .iter()
                    .filter(|(n, rc, _)| *n > since && *rc == c)
                    .map(|(_, _, name)| name.clone())
                    .collect();
                let mut out = String::new();
                for name in names {
                    let href = format!("{}{name}", cal(&c));
                    match s.resources.get(&(c.clone(), name)) {
                        Some((etag, _)) => out.push_str(&ok(
                            &href,
                            &format!("<getetag>{}</getetag>", esc(etag)),
                        )),
                        None => out.push_str(&gone(&href)),
                    }
                }
                out.push_str(&format!("<sync-token>{}</sync-token>", token(s.counter)));
                ms(207, out)
            }
            ("REPORT", _, Some(c)) if body.contains("calendar-multiget") => {
                let mut out = String::new();
                for href in body
                    .split("<d:href>")
                    .skip(1)
                    .filter_map(|r| r.split("</d:href>").next())
                {
                    let name = href.rsplit('/').next().unwrap_or("").to_owned();
                    match s.resources.get(&(c.clone(), name)) {
                        Some((etag, data)) => out.push_str(&ok(
                            href,
                            &format!(
                                "<getetag>{}</getetag><C:calendar-data>{}</C:calendar-data>",
                                esc(etag),
                                esc(data)
                            ),
                        )),
                        None => out.push_str(&gone(href)),
                    }
                }
                ms(207, out)
            }
            _ => ResponseTemplate::new(404),
        }
    }
}

impl Respond for FakeIcloud {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.respond_to(request)
    }
}

// ----- helpers ---------------------------------------------------------------------------

struct Setup {
    s: TestServer,
    dav: FakeIcloud,
    _server: MockServer,
    session: String,
}

async fn setup() -> Setup {
    let server = MockServer::start().await;
    let dav = FakeIcloud::default();
    dav.with(|s| {
        s.base = server.uri();
        s.password = APP_PASSWORD.to_owned();
    });
    dav.put(
        PRIVAT,
        "zahnarzt.ics",
        &vevent("zahnarzt@example.org", "Zahnarzt"),
    );
    dav.put(
        FAMILIE,
        "geburtstag.ics",
        &vevent("geburtstag@example.org", "Geburtstag Oma"),
    );
    dav.put(
        VEREIN,
        "sommerfest.ics",
        &vevent("sommerfest@example.org", "Sommerfest"),
    );
    Mock::given(wiremock::matchers::any())
        .respond_with(dav.clone())
        .mount(&server)
        .await;
    let base = Url::parse(&format!("{}/", server.uri())).unwrap();
    let s = TestServer::with_config(|c| {
        c.icloud_url = base.clone();
        c.fastmail_url = base;
    })
    .await;
    let session = s.session().await;
    Setup {
        s,
        dav,
        _server: server,
        session,
    }
}

impl Setup {
    async fn create(&self, password: &str) -> common::TestResponse {
        self.s
            .post(
                "/api/accounts",
                Auth::Web(&self.session),
                json!({"kind": "icloud", "username": USER, "password": password}),
            )
            .await
    }

    async fn calendars(&self) -> Vec<Value> {
        let r = self.s.get("/api/calendars", Auth::Bearer(BOOTSTRAP)).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        r.json().as_array().unwrap().clone()
    }

    async fn titles(&self, calendar_id: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .s
            .app
            .calendar_events(calendar_id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e["title"].as_str().unwrap_or_default().to_owned())
            .collect();
        out.sort();
        out
    }
}

fn by_name<'a>(cals: &'a [Value], name: &str) -> &'a Value {
    cals.iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no calendar {name}: {cals:?}"))
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_owned()
}

// ----- tests ---------------------------------------------------------------------------

#[tokio::test]
async fn icloud_account_with_shared_calendars_end_to_end() {
    let t = setup().await;
    let r = t.create(APP_PASSWORD).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert!(!r.text().contains(APP_PASSWORD));
    let account = r.json();
    assert_eq!(account["kind"], "icloud");
    assert_eq!(account["name"], "iCloud");
    let account_id = id(&account);

    // Shares appear as calendars of their own; reminders, inbox, outbox
    // and notifications do not.
    let cals = t.calendars().await;
    let names: Vec<&str> = cals.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Privat", "Familie", "Verein"]);

    let privat = by_name(&cals, "Privat");
    assert_eq!(
        privat["shared"], false,
        "own calendar despite partition host"
    );
    assert_eq!(privat["writable"], true);
    assert_eq!(privat["can_invite"], true);
    assert_eq!(privat["mcp_write"], true);
    assert_eq!(privat["provider_color"], "#1BADF8");

    let familie = by_name(&cals, "Familie");
    assert_eq!(familie["shared"], true);
    assert_eq!(familie["writable"], true);
    assert_eq!(familie["provider_color"], "#FF2968");

    let verein = by_name(&cals, "Verein");
    assert_eq!(verein["shared"], true);
    assert_eq!(verein["writable"], false, "read-only share");
    assert_eq!(verein["can_invite"], false);
    assert_eq!(verein["mcp_write"], false);

    // Claude cannot be allowed to write into the read-only share.
    let r =
        t.s.request(
            Method::PATCH,
            &format!("/api/calendars/{}", id(verein)),
            Auth::Web(&t.session),
            Some(json!({"mcp_write": true})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["mcp_write"], false);

    // First sync: full comparison, everything arrives.
    let r = t.s.app.sync_account(&account_id).await.unwrap();
    assert!(r.events.full);
    assert_eq!((r.events.added, r.events.deleted), (3, 0));
    assert_eq!(t.titles(&id(privat)).await, ["Zahnarzt"]);
    assert_eq!(t.titles(&id(familie)).await, ["Geburtstag Oma"]);
    assert_eq!(t.titles(&id(verein)).await, ["Sommerfest"]);

    // Incremental: sync-collection with the stored iCloud token.
    t.dav.put(
        FAMILIE,
        "elternabend.ics",
        &vevent("elternabend@example.org", "Elternabend"),
    );
    t.dav.remove(PRIVAT, "zahnarzt.ics");
    let before = t.dav.with(|s| s.sync_reports);
    let r = t.s.app.sync_account(&account_id).await.unwrap();
    assert!(!r.events.full, "incremental with sync-collection");
    assert_eq!((r.events.added, r.events.deleted), (1, 1));
    assert!(t.dav.with(|s| s.sync_reports) > before);
    assert!(t.titles(&id(privat)).await.is_empty());
    assert_eq!(
        t.titles(&id(familie)).await,
        ["Elternabend", "Geburtstag Oma"]
    );

    // iCloud forgot the tokens (403 valid-sync-token): full comparison,
    // nothing lost, nothing doubled.
    t.dav.with(|s| s.tokens_invalid = true);
    t.dav.put(
        VEREIN,
        "vorstand.ics",
        &vevent("vorstand@example.org", "Vorstandssitzung"),
    );
    let r = t.s.app.sync_account(&account_id).await.unwrap();
    assert!(r.events.full);
    assert_eq!((r.events.added, r.events.deleted), (1, 0));
    assert_eq!(
        t.titles(&id(verein)).await,
        ["Sommerfest", "Vorstandssitzung"]
    );
    let a = t.s.app.account(&account_id).await.unwrap();
    assert_eq!(a.status.state, "ok");
}

#[tokio::test]
async fn apple_id_password_gets_the_app_specific_password_hint() {
    let t = setup().await;
    let r = t.create(APPLE_ID_PASSWORD).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    assert_eq!(r.error_code(), "invalid_credentials");
    let message = r.json()["error"]["message"].as_str().unwrap().to_owned();
    assert!(message.starts_with("iCloud hat die Zugangsdaten abgelehnt"));
    assert!(message.contains("App-spezifisches Passwort"), "{message}");
    assert!(message.contains("appleid.apple.com"), "{message}");
    assert!(!r.text().contains(APPLE_ID_PASSWORD));

    // Revoked app password during a sync: auth_failed with the same hint.
    let r = t.create(APP_PASSWORD).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let account_id = id(&r.json());
    t.dav.with(|s| s.password = NEW_APP_PASSWORD.to_owned());
    let err = t.s.app.sync_account(&account_id).await.unwrap_err();
    assert_eq!(err.code(), "invalid_credentials");
    let a = t.s.app.account(&account_id).await.unwrap();
    assert_eq!(a.status.state, "auth_failed");
    assert!(a.status.message.unwrap().contains("appleid.apple.com"));

    // Fastmail: the same mistake, pointed at Fastmail's app passwords.
    let r =
        t.s.post(
            "/api/accounts",
            Auth::Web(&t.session),
            json!({"kind": "fastmail", "username": USER, "password": APPLE_ID_PASSWORD}),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    let message = r.json()["error"]["message"].as_str().unwrap().to_owned();
    assert!(message.starts_with("Fastmail hat die Zugangsdaten abgelehnt"));
    assert!(message.contains("App-Passwort"), "{message}");
    assert!(message.contains("App-Passwörter"), "{message}");
}

#[tokio::test]
async fn throttling_stops_the_run_and_reports_it() {
    let t = setup().await;
    let r = t.create(APP_PASSWORD).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let account_id = id(&r.json());
    t.s.app.sync_account(&account_id).await.unwrap();

    t.dav.with(|s| s.throttled = true);
    let err = t.s.app.sync_account(&account_id).await.unwrap_err();
    assert_eq!(err.code(), "provider_unavailable");
    assert_eq!(
        t.dav.with(|s| s.throttled_answers),
        1,
        "remaining calendars are left for the next run"
    );
    let a = t.s.app.account(&account_id).await.unwrap();
    assert_eq!(a.status.state, "error");
    assert!(a.status.message.unwrap().contains("drosselt"));

    // Events stay; the next run after the pause works again.
    t.dav.with(|s| s.throttled = false);
    let r = t.s.app.sync_account(&account_id).await.unwrap();
    assert_eq!(r.events.added, 0);
    assert_eq!(
        t.s.app.account(&account_id).await.unwrap().status.state,
        "ok"
    );
}
