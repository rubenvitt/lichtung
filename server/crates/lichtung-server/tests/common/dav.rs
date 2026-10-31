//! An invented, stateful CalDAV server (wiremock with a custom responder)
//! for the sync and write tests: discovery, calendar listing, ETag listing,
//! `sync-collection`, `calendar-multiget`, GET, PUT with `If-Match` /
//! `If-None-Match`, DELETE. Several calendars, one of them optionally
//! read-only. All data is made up.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use base64::Engine;
use lichtung_server::changes;
use serde_json::{json, Value};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::{Auth, TestServer, BOOTSTRAP};

pub const USER: &str = "anna@example.org";
/// Invented app password of the fake account.
pub const PASSWORD: &str = "app-passwort-1234";
pub const PRINCIPAL: &str = "/dav/principals/anna/";
pub const HOME: &str = "/dav/calendars/anna/";
pub const CALENDAR: &str = "/dav/calendars/anna/privat/";
pub const WORK: &str = "/dav/calendars/anna/arbeit/";
pub const HOLIDAYS: &str = "/dav/calendars/anna/feiertage/";

// ----- invented iCalendar data ---------------------------------------------------------

pub fn vcal(body: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel//Test//DE\r\n{body}END:VCALENDAR\r\n"
    )
}

pub fn single(uid: &str, title: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\n\
         DTSTART;TZID=Europe/Berlin:20261103T140000\r\nDTEND;TZID=Europe/Berlin:20261103T150000\r\n\
         SUMMARY:{title}\r\nLOCATION:Praxis am Markt\r\nEND:VEVENT\r\n"
    ))
}

pub fn series(uid: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\n\
         DTSTART;TZID=Europe/Berlin:20261102T090000\r\nDTEND;TZID=Europe/Berlin:20261102T093000\r\n\
         RRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=4\r\nEXDATE;TZID=Europe/Berlin:20261116T090000\r\n\
         SUMMARY:Team-Sync\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\n\
         RECURRENCE-ID;TZID=Europe/Berlin:20261109T090000\r\n\
         DTSTART;TZID=Europe/Berlin:20261109T100000\r\nDTEND;TZID=Europe/Berlin:20261109T103000\r\n\
         SUMMARY:Team-Sync (verschoben)\r\nEND:VEVENT\r\n"
    ))
}

pub fn all_day(uid: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\nDTSTART;VALUE=DATE:20261105\r\n\
         DTEND;VALUE=DATE:20261107\r\nSUMMARY:Wanderwochenende\r\nTRANSP:TRANSPARENT\r\n\
         END:VEVENT\r\n"
    ))
}

/// An invitation from someone else with the own address as attendee.
pub fn invitation(uid: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261020T070000Z\r\nSEQUENCE:2\r\n\
         DTSTART;TZID=Europe/Berlin:20261104T100000\r\nDTEND;TZID=Europe/Berlin:20261104T110000\r\n\
         RRULE:FREQ=WEEKLY;COUNT=3\r\nSUMMARY:Vereinssitzung\r\n\
         ORGANIZER;CN=Bert Beispiel:mailto:bert@example.org\r\n\
         ATTENDEE;CN=Bert Beispiel;PARTSTAT=ACCEPTED:mailto:bert@example.org\r\n\
         ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{USER}\r\n\
         X-BEISPIEL-UNBEKANNT:bleibt erhalten\r\nEND:VEVENT\r\n"
    ))
}

// ----- the fake server -------------------------------------------------------------------

/// One calendar collection.
#[derive(Debug, Clone)]
pub struct Collection {
    pub display_name: String,
    pub writable: bool,
    /// Resource name → (etag, data).
    pub resources: BTreeMap<String, (String, String)>,
    /// (change number, resource name) in order.
    pub log: Vec<(u64, String)>,
}

impl Collection {
    fn new(display_name: &str, writable: bool) -> Self {
        Collection {
            display_name: display_name.into(),
            writable,
            resources: BTreeMap::new(),
            log: Vec::new(),
        }
    }
}

pub struct DavState {
    /// Collection path → collection.
    pub calendars: BTreeMap<String, Collection>,
    pub counter: u64,
    pub sync_unsupported: bool,
    pub tokens_invalid: bool,
    pub multiget_status: Option<u16>,
    pub multiget_missing: HashSet<String>,
    pub unauthorized: bool,
    /// Hides [`CALENDAR`] from the listing of the home.
    pub calendar_gone: bool,
    /// Send calendar data with LF line endings only.
    pub lf_only: bool,
    pub multigets: Vec<Vec<String>>,
    pub etag_listings: usize,
    pub sync_reports: usize,
    /// Answer every PUT with this status instead.
    pub put_status: Option<u16>,
    /// Answer every DELETE with this status instead.
    pub delete_status: Option<u16>,
    /// Store a PUT as usual, but answer it with this status (the answer
    /// got lost after the write, e.g. a gateway timeout).
    pub put_status_after_write: Option<u16>,
    /// PUT answers carry the new ETag. Without it the server also changes
    /// the data (adds `X-BEISPIEL-GESEHEN:1`), like servers that normalize.
    pub etag_in_put_response: bool,
    /// Refuse a UID that already exists in another calendar of the home
    /// (`CALDAV:no-uid-conflict`).
    pub uid_conflict_across: bool,
    /// Every successful or refused PUT and DELETE: (method, path).
    pub writes: Vec<(String, String)>,
}

impl Default for DavState {
    fn default() -> Self {
        let mut calendars = BTreeMap::new();
        calendars.insert(CALENDAR.to_owned(), Collection::new("Privat", true));
        DavState {
            calendars,
            counter: 0,
            sync_unsupported: false,
            tokens_invalid: false,
            multiget_status: None,
            multiget_missing: HashSet::new(),
            unauthorized: false,
            calendar_gone: false,
            lf_only: false,
            multigets: Vec::new(),
            etag_listings: 0,
            sync_reports: 0,
            put_status: None,
            delete_status: None,
            put_status_after_write: None,
            etag_in_put_response: true,
            uid_conflict_across: false,
            writes: Vec::new(),
        }
    }
}

impl DavState {
    fn store(&mut self, calendar: &str, name: &str, data: &str) -> String {
        self.counter += 1;
        let n = self.counter;
        let etag = format!("\"e{n}\"");
        let cal = self.calendars.get_mut(calendar).expect("known calendar");
        cal.resources
            .insert(name.into(), (etag.clone(), data.into()));
        cal.log.push((n, name.into()));
        etag
    }

    fn drop_resource(&mut self, calendar: &str, name: &str) {
        self.counter += 1;
        let n = self.counter;
        let cal = self.calendars.get_mut(calendar).expect("known calendar");
        cal.resources.remove(name);
        cal.log.push((n, name.into()));
    }
}

#[derive(Clone, Default)]
pub struct FakeDav(pub Arc<Mutex<DavState>>);

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn ms(body: String) -> ResponseTemplate {
    ResponseTemplate::new(207).set_body_raw(
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:CS="http://calendarserver.org/ns/">{body}</D:multistatus>"#
        ),
        "application/xml; charset=utf-8",
    )
}

fn ok_response(href: &str, props: &str) -> String {
    format!(
        "<D:response><D:href>{href}</D:href><D:propstat><D:prop>{props}</D:prop>\
         <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
    )
}

fn gone_response(href: &str) -> String {
    format!(
        "<D:response><D:href>{href}</D:href><D:status>HTTP/1.1 404 Not Found</D:status></D:response>"
    )
}

fn uid_of(data: &str) -> Option<String> {
    data.lines()
        .find_map(|l| l.trim_end().strip_prefix("UID:").map(str::to_owned))
}

/// The expected `Authorization` header, built at runtime.
fn expected_auth() -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{PASSWORD}"))
    )
}

impl FakeDav {
    pub fn with<T>(&self, f: impl FnOnce(&mut DavState) -> T) -> T {
        f(&mut self.0.lock().unwrap())
    }

    /// Adds a calendar collection to the home.
    pub fn add_calendar(&self, path: &str, display_name: &str, writable: bool) {
        self.with(|s| {
            s.calendars
                .insert(path.into(), Collection::new(display_name, writable))
        });
    }

    /// Stores a resource in [`CALENDAR`].
    pub fn put(&self, name: &str, data: &str) {
        self.put_in(CALENDAR, name, data);
    }

    pub fn put_in(&self, calendar: &str, name: &str, data: &str) {
        self.with(|s| {
            s.store(calendar, name, data);
        });
    }

    /// Reports the resource as changed without changing it.
    pub fn touch(&self, name: &str) {
        self.with(|s| {
            s.counter += 1;
            let n = s.counter;
            s.calendars
                .get_mut(CALENDAR)
                .unwrap()
                .log
                .push((n, name.into()));
        });
    }

    pub fn remove(&self, name: &str) {
        self.with(|s| s.drop_resource(CALENDAR, name));
    }

    /// (etag, data) of a resource.
    pub fn get_in(&self, calendar: &str, name: &str) -> Option<(String, String)> {
        self.with(|s| s.calendars.get(calendar)?.resources.get(name).cloned())
    }

    /// Names of the resources in a calendar.
    pub fn names_in(&self, calendar: &str) -> Vec<String> {
        self.with(|s| {
            s.calendars
                .get(calendar)
                .map(|c| c.resources.keys().cloned().collect())
                .unwrap_or_default()
        })
    }

    /// The one resource of a calendar whose data has this UID.
    pub fn find_uid(&self, calendar: &str, uid: &str) -> Option<(String, String, String)> {
        self.with(|s| {
            s.calendars
                .get(calendar)?
                .resources
                .iter()
                .find_map(|(n, (e, d))| {
                    (uid_of(d).as_deref() == Some(uid)).then(|| (n.clone(), e.clone(), d.clone()))
                })
        })
    }

    /// Number of PUTs so far.
    pub fn put_count(&self) -> usize {
        self.with(|s| s.writes.iter().filter(|(m, _)| m == "PUT").count())
    }

    pub fn token(n: u64) -> String {
        format!("https://example.org/ns/sync/{n}")
    }

    pub fn multiget_count(&self) -> usize {
        self.with(|s| s.multigets.len())
    }

    fn respond_to(&self, req: &Request) -> ResponseTemplate {
        let auth = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        let mut s = self.0.lock().unwrap();
        if s.unauthorized || auth != Some(expected_auth().as_str()) {
            return ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Basic");
        }
        let path = req.url.path().to_owned();
        let method = req.method.as_str().to_owned();
        let header = |name: &str| {
            req.headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let depth = header("depth").unwrap_or_default();
        let body = String::from_utf8_lossy(&req.body).into_owned();

        // Collection and resource name of the path.
        let (collection, name) = match path.rsplit_once('/') {
            Some((dir, name)) => (format!("{dir}/"), name.to_owned()),
            None => (String::new(), String::new()),
        };
        let visible = |s: &DavState, c: &str| {
            s.calendars.contains_key(c) && !(s.calendar_gone && c == CALENDAR)
        };

        match method.as_str() {
            "PROPFIND" if path == "/.well-known/caldav" || path == "/dav/" => {
                return ms(ok_response(
                    "/dav/",
                    &format!("<D:current-user-principal><D:href>{PRINCIPAL}</D:href></D:current-user-principal>"),
                ))
            }
            "PROPFIND" if path == PRINCIPAL => {
                return ms(ok_response(
                    PRINCIPAL,
                    &format!(
                        "<C:calendar-home-set><D:href>{HOME}</D:href></C:calendar-home-set>\
                         <C:calendar-user-address-set><D:href>mailto:anna.alt@example.org</D:href>\
                         <D:href>mailto:{USER}</D:href></C:calendar-user-address-set>"
                    ),
                ))
            }
            "OPTIONS" => {
                return ResponseTemplate::new(200)
                    .insert_header("DAV", "1, 2, 3, calendar-access, calendar-auto-schedule")
            }
            "PROPFIND" if path == HOME => {
                let mut out = ok_response(HOME, "<D:resourcetype><D:collection/></D:resourcetype>");
                let token = if s.sync_unsupported {
                    String::new()
                } else {
                    format!("<D:sync-token>{}</D:sync-token>", Self::token(s.counter))
                };
                for (cal_path, cal) in &s.calendars {
                    if !visible(&s, cal_path) {
                        continue;
                    }
                    let write = if cal.writable {
                        "<D:privilege><D:write/></D:privilege>"
                    } else {
                        ""
                    };
                    out.push_str(&ok_response(
                        cal_path,
                        &format!(
                            "<D:resourcetype><D:collection/><C:calendar/></D:resourcetype>\
                             <D:displayname>{}</D:displayname>\
                             <D:current-user-privilege-set><D:privilege><D:read/></D:privilege>\
                             {write}</D:current-user-privilege-set>\
                             <C:supported-calendar-component-set><C:comp name=\"VEVENT\"/></C:supported-calendar-component-set>\
                             <CS:getctag>{}</CS:getctag>{token}",
                            cal.display_name, s.counter
                        ),
                    ));
                }
                return ms(out);
            }
            _ => {}
        }

        // Requests on a calendar collection.
        if name.is_empty() && visible(&s, &collection) {
            let token = if s.sync_unsupported {
                String::new()
            } else {
                format!("<D:sync-token>{}</D:sync-token>", Self::token(s.counter))
            };
            return match method.as_str() {
                "PROPFIND" if depth == "0" => ms(ok_response(
                    &collection,
                    &format!("<CS:getctag>{}</CS:getctag>{token}", s.counter),
                )),
                "PROPFIND" => {
                    s.etag_listings += 1;
                    let mut out = ok_response(
                        &collection,
                        "<D:resourcetype><D:collection/><C:calendar/></D:resourcetype>",
                    );
                    for (name, (etag, _)) in &s.calendars[&collection].resources {
                        out.push_str(&ok_response(
                            &format!("{collection}{name}"),
                            &format!("<D:getetag>{}</D:getetag>", escape(etag)),
                        ));
                    }
                    ms(out)
                }
                "REPORT" if body.contains("sync-collection") => {
                    s.sync_reports += 1;
                    if s.sync_unsupported {
                        return ResponseTemplate::new(403).set_body_raw(
                            r#"<?xml version="1.0"?><D:error xmlns:D="DAV:"><D:supported-report/></D:error>"#,
                            "application/xml",
                        );
                    }
                    let since = body
                        .split("<d:sync-token>")
                        .nth(1)
                        .and_then(|r| r.split("</d:sync-token>").next())
                        .map(|t| t.rsplit('/').next().unwrap_or("").parse::<u64>());
                    let since = match since {
                        None => 0,
                        Some(Ok(n)) if !s.tokens_invalid && n <= s.counter => n,
                        Some(_) => {
                            return ResponseTemplate::new(403).set_body_raw(
                                r#"<?xml version="1.0"?><D:error xmlns:D="DAV:"><D:valid-sync-token/></D:error>"#,
                                "application/xml",
                            )
                        }
                    };
                    let cal = &s.calendars[&collection];
                    let names: BTreeSet<String> = if since == 0 {
                        cal.resources.keys().cloned().collect()
                    } else {
                        cal.log
                            .iter()
                            .filter(|(n, _)| *n > since)
                            .map(|(_, name)| name.clone())
                            .collect()
                    };
                    let mut out = String::new();
                    for name in names {
                        let href = format!("{collection}{name}");
                        match cal.resources.get(&name) {
                            Some((etag, _)) => out.push_str(&ok_response(
                                &href,
                                &format!("<D:getetag>{}</D:getetag>", escape(etag)),
                            )),
                            None => out.push_str(&gone_response(&href)),
                        }
                    }
                    out.push_str(&format!(
                        "<D:sync-token>{}</D:sync-token>",
                        Self::token(s.counter)
                    ));
                    ms(out)
                }
                "REPORT" if body.contains("calendar-multiget") => {
                    let hrefs: Vec<String> = body
                        .split("<d:href>")
                        .skip(1)
                        .filter_map(|r| r.split("</d:href>").next())
                        .map(str::to_owned)
                        .collect();
                    s.multigets.push(hrefs.clone());
                    if let Some(status) = s.multiget_status {
                        return ResponseTemplate::new(status);
                    }
                    let cal = &s.calendars[&collection];
                    let mut out = String::new();
                    for href in hrefs {
                        let name = href.rsplit('/').next().unwrap_or("").to_owned();
                        match cal.resources.get(&name) {
                            Some((etag, data)) if !s.multiget_missing.contains(&name) => {
                                let data = if s.lf_only {
                                    data.replace("\r\n", "\n")
                                } else {
                                    data.clone()
                                };
                                out.push_str(&ok_response(
                                    &href,
                                    &format!(
                                        "<D:getetag>{}</D:getetag><C:calendar-data>{}</C:calendar-data>",
                                        escape(etag),
                                        escape(&data)
                                    ),
                                ));
                            }
                            _ => out.push_str(&gone_response(&href)),
                        }
                    }
                    ms(out)
                }
                _ => ResponseTemplate::new(404),
            };
        }

        // Requests on a resource.
        if name.is_empty() || !visible(&s, &collection) {
            return ResponseTemplate::new(404);
        }
        let current = s.calendars[&collection].resources.get(&name).cloned();
        match method.as_str() {
            "GET" => match current {
                Some((etag, data)) => ResponseTemplate::new(200)
                    .insert_header("ETag", etag.as_str())
                    .set_body_raw(data, "text/calendar; charset=utf-8"),
                None => ResponseTemplate::new(404),
            },
            "PROPFIND" => match current {
                Some((etag, _)) => ms(ok_response(
                    &path,
                    &format!("<D:getetag>{}</D:getetag>", escape(&etag)),
                )),
                None => ResponseTemplate::new(404),
            },
            "PUT" => {
                s.writes.push(("PUT".into(), path.clone()));
                if let Some(status) = s.put_status {
                    return ResponseTemplate::new(status);
                }
                if !s.calendars[&collection].writable {
                    return ResponseTemplate::new(403).set_body_raw(
                        r#"<?xml version="1.0"?><D:error xmlns:D="DAV:"><D:need-privileges/></D:error>"#,
                        "application/xml",
                    );
                }
                if header("if-none-match").as_deref() == Some("*") && current.is_some() {
                    return ResponseTemplate::new(412);
                }
                if let Some(wanted) = header("if-match") {
                    if current.as_ref().map(|(e, _)| e.as_str()) != Some(wanted.as_str()) {
                        return ResponseTemplate::new(412);
                    }
                }
                if s.uid_conflict_across {
                    let uid = uid_of(&body);
                    let clash = s.calendars.iter().any(|(p, c)| {
                        p != &collection && c.resources.values().any(|(_, d)| uid_of(d) == uid)
                    });
                    if clash {
                        return ResponseTemplate::new(403).set_body_raw(
                            r#"<?xml version="1.0"?><D:error xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><C:no-uid-conflict/></D:error>"#,
                            "application/xml",
                        );
                    }
                }
                let exact = s.etag_in_put_response;
                let data = if exact {
                    body.clone()
                } else {
                    body.replacen("END:VEVENT", "X-BEISPIEL-GESEHEN:1\r\nEND:VEVENT", 1)
                };
                let etag = s.store(&collection, &name, &data);
                if let Some(status) = s.put_status_after_write {
                    return ResponseTemplate::new(status);
                }
                let status = if current.is_some() { 204 } else { 201 };
                let response = ResponseTemplate::new(status);
                if exact {
                    response.insert_header("ETag", etag.as_str())
                } else {
                    response
                }
            }
            "DELETE" => {
                s.writes.push(("DELETE".into(), path.clone()));
                if let Some(status) = s.delete_status {
                    return ResponseTemplate::new(status);
                }
                let Some((etag, _)) = current else {
                    return ResponseTemplate::new(404);
                };
                if let Some(wanted) = header("if-match") {
                    if etag != wanted {
                        return ResponseTemplate::new(412);
                    }
                }
                s.drop_resource(&collection, &name);
                ResponseTemplate::new(204)
            }
            _ => ResponseTemplate::new(405),
        }
    }
}

impl Respond for FakeDav {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.respond_to(request)
    }
}

// ----- setup -----------------------------------------------------------------------------

pub struct Setup {
    pub s: TestServer,
    pub dav: FakeDav,
    pub server: MockServer,
    pub account_id: String,
    /// The calendar at [`CALENDAR`].
    pub calendar_id: String,
}

/// A server with a CalDAV account on a fresh fake server, prepared by
/// `prepare` before the account is connected.
pub async fn setup_with(prepare: impl FnOnce(&FakeDav)) -> Setup {
    setup_configured(prepare, |_| {}).await
}

/// [`setup_with`] with an adjusted server configuration.
pub async fn setup_configured(
    prepare: impl FnOnce(&FakeDav),
    adjust: impl FnOnce(&mut lichtung_server::Config),
) -> Setup {
    let server = MockServer::start().await;
    let dav = FakeDav::default();
    prepare(&dav);
    Mock::given(wiremock::matchers::any())
        .respond_with(dav.clone())
        .mount(&server)
        .await;
    let s = TestServer::with_config(adjust).await;
    let session = s.session().await;
    let r = s
        .post(
            "/api/accounts",
            Auth::Web(&session),
            json!({"kind": "caldav", "url": format!("{}/dav/", server.uri()),
                   "username": USER, "password": PASSWORD, "name": "Beispiel"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let account_id = r.json()["id"].as_str().unwrap().to_owned();
    let mut setup = Setup {
        s,
        dav,
        server,
        account_id,
        calendar_id: String::new(),
    };
    setup.calendar_id = setup.calendar_named("Privat").await;
    setup
}

/// Two events in [`CALENDAR`]: a single one and a series with override.
pub async fn setup() -> Setup {
    setup_with(|dav| {
        dav.put("zahnarzt.ics", &single("zahnarzt@example.org", "Zahnarzt"));
        dav.put("team.ics", &series("team-sync@example.org"));
    })
    .await
}

impl Setup {
    pub async fn sync(&self) -> lichtung_server::sync::SyncReport {
        self.s.app.sync_account(&self.account_id).await.unwrap()
    }

    pub async fn calendar_named(&self, name: &str) -> String {
        let cals = self
            .s
            .get("/api/calendars", Auth::Bearer(BOOTSTRAP))
            .await
            .json();
        cals.as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no calendar {name}: {cals}"))["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    pub async fn events(&self) -> Vec<Value> {
        self.s.app.calendar_events(&self.calendar_id).await.unwrap()
    }

    pub async fn event_by_uid(&self, uid: &str) -> Value {
        self.events()
            .await
            .into_iter()
            .find(|e| e["uid"] == uid)
            .unwrap_or_else(|| panic!("no event {uid}"))
    }

    pub fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.s.app.config().db_path()).unwrap()
    }

    pub fn sync_token(&self) -> Option<String> {
        self.db()
            .query_row(
                "SELECT sync_token FROM calendars WHERE id = ?1",
                [&self.calendar_id],
                |r| r.get(0),
            )
            .unwrap()
    }

    pub fn tombstones_of(&self, kind: &str) -> Vec<String> {
        changes::tombstones_since(&self.db(), 0)
            .unwrap()
            .into_iter()
            .filter(|t| t.kind == kind)
            .map(|t| t.id)
            .collect()
    }

    pub fn event_tombstones(&self) -> Vec<String> {
        self.tombstones_of("event")
    }

    pub fn uids(&self) -> Vec<String> {
        let db = self.db();
        let mut stmt = db.prepare("SELECT uid FROM events ORDER BY uid").unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// The stored original of an event.
    pub fn stored_ics(&self, event_id: &str) -> String {
        self.db()
            .query_row("SELECT ics FROM events WHERE id = ?1", [event_id], |r| {
                r.get(0)
            })
            .unwrap()
    }

    pub async fn status(&self) -> (String, Option<String>) {
        let a = self.s.app.account(&self.account_id).await.unwrap();
        (a.status.state.to_owned(), a.status.message)
    }
}
