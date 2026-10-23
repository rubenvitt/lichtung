//! ICS subscriptions (LIC-14) against an invented feed server (wiremock
//! with a stateful responder): creating accounts (valid, 404, no
//! VCALENDAR, Basic auth, `webcal`), conditional fetches with 304, new,
//! changed and removed events with tombstones and `seq`, the size limit,
//! an Outlook-style feed (Windows zones, BUSYSTATUS, overrides without
//! master, duplicate overrides), and that the secret feed URL never shows
//! up in answers or logs. All data is made up.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use axum::http::{Method, StatusCode};
use base64::Engine;
use lichtung_server::changes;
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use common::{Auth, TestServer, BOOTSTRAP};

/// Path of the invented feed; the long segment stands for the key Outlook
/// and Google put into the path.
const FEED_PATH: &str = "/owa/calendar/5d41402abc4b2a76b9719d911017c592/kalender.ics";
const FEED_USER: &str = "kalender-abo";

/// The secret query token (built at runtime, so no scanner mistakes it for
/// a real one).
fn query_token() -> String {
    ["erfunden", "abo", "0815", "4711"].join("-")
}

/// Invented wrong password.
fn wrong_password() -> String {
    ["falsch", "geraten"].join("-")
}

/// Invented Basic-auth password.
fn feed_password() -> String {
    ["nur", "fuer", "tests", "42"].join("-")
}

fn basic_header(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
}

// ----- log capture ----------------------------------------------------------------------

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// All log output of this test binary (debug level, without the fake
/// server's own logs).
fn logs() -> &'static LogBuffer {
    static LOGS: OnceLock<LogBuffer> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = LogBuffer::default();
        let writer = buffer.clone();
        tracing_subscriber::fmt()
            // The fake feed server (wiremock) logs the requests it gets.
            .with_env_filter("debug,wiremock=off")
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .init();
        buffer
    })
}

fn log_text() -> String {
    String::from_utf8_lossy(&logs().0.lock().unwrap()).into_owned()
}

// ----- invented feeds -------------------------------------------------------------------

fn feed(events: &[String]) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Beispiel AG//Kalenderexport//DE\r\n\
         METHOD:PUBLISH\r\nX-WR-CALNAME:Arbeit (Abo)\r\nX-APPLE-CALENDAR-COLOR:#4f46e5\r\n\
         {}END:VCALENDAR\r\n",
        events.concat()
    )
}

fn event(uid: &str, title: &str, day: u32) -> String {
    format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261023T060000Z\r\n\
         DTSTART;TZID=Europe/Berlin:202611{day:02}T090000\r\n\
         DTEND;TZID=Europe/Berlin:202611{day:02}T100000\r\nSUMMARY:{title}\r\nEND:VEVENT\r\n"
    )
}

fn standard_feed() -> String {
    feed(&[
        event("planung@example.org", "Quartalsplanung", 3),
        event("review@example.org", "Code-Review", 4),
    ])
}

/// An invented feed in the style of an Exchange/Outlook export.
fn outlook_feed() -> String {
    let tz = "BEGIN:VTIMEZONE\r\nTZID:W. Europe Standard Time\r\nBEGIN:STANDARD\r\n\
              DTSTART:16010101T030000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\n\
              RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=10\r\nEND:STANDARD\r\n\
              BEGIN:DAYLIGHT\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:+0100\r\n\
              TZOFFSETTO:+0200\r\nRRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=3\r\n\
              END:DAYLIGHT\r\nEND:VTIMEZONE\r\n";
    let cdo = |status: &str| {
        format!(
            "X-MICROSOFT-CDO-APPT-SEQUENCE:0\r\nX-MICROSOFT-CDO-BUSYSTATUS:{status}\r\n\
             X-MICROSOFT-CDO-INTENDEDSTATUS:{status}\r\nX-MICROSOFT-CDO-ALLDAYEVENT:FALSE\r\n\
             X-MICROSOFT-CDO-IMPORTANCE:1\r\nX-MICROSOFT-CDO-INSTTYPE:0\r\n\
             X-MICROSOFT-DONOTFORWARDMEETING:FALSE\r\nX-MICROSOFT-DISALLOW-COUNTER:FALSE\r\n"
        )
    };
    let series = format!(
        "BEGIN:VEVENT\r\nDESCRIPTION:\\n\r\n\
         UID:040000008200E00074C5B7101A82E00800000000A1B2C3D4E5F6A70100000000000000001\r\n \
         0000000F1E2D3C4B5A69788\r\n\
         RRULE:FREQ=WEEKLY;UNTIL=20261221T090000Z;INTERVAL=1;BYDAY=MO;WKST=MO\r\n\
         SUMMARY;LANGUAGE=de-DE:Jour fixe Team\r\n\
         DTSTART;TZID=W. Europe Standard Time:20261102T100000\r\n\
         DTEND;TZID=W. Europe Standard Time:20261102T103000\r\n\
         CLASS:PUBLIC\r\nPRIORITY:5\r\nDTSTAMP:20261023T061500Z\r\nTRANSP:OPAQUE\r\n\
         STATUS:CONFIRMED\r\nSEQUENCE:0\r\nLOCATION;LANGUAGE=de-DE:Besprechungsraum 3\r\n{}\
         END:VEVENT\r\n",
        cdo("BUSY")
    );
    let focus = format!(
        "BEGIN:VEVENT\r\nUID:040000008200E00074C5B7101A82E008000000000000000000000000000000000001\r\n\
         SUMMARY;LANGUAGE=de-DE:Fokuszeit\r\n\
         DTSTART;TZID=W. Europe Standard Time:20261103T130000\r\n\
         DTEND;TZID=W. Europe Standard Time:20261103T150000\r\n\
         DTSTAMP:20261023T061500Z\r\nSEQUENCE:0\r\n{}END:VEVENT\r\n",
        cdo("FREE")
    );
    let vacation = format!(
        "BEGIN:VEVENT\r\nUID:040000008200E00074C5B7101A82E008000000000000000000000000000000000002\r\n\
         SUMMARY;LANGUAGE=de-DE:Urlaub\r\nDTSTART;VALUE=DATE:20261116\r\n\
         DTEND;VALUE=DATE:20261121\r\nDTSTAMP:20261023T061500Z\r\nSEQUENCE:0\r\n{}END:VEVENT\r\n",
        cdo("OOF")
    );
    // TRANSP wins over BUSYSTATUS.
    let transparent = format!(
        "BEGIN:VEVENT\r\nUID:040000008200E00074C5B7101A82E008000000000000000000000000000000000003\r\n\
         SUMMARY:Info: Sommerfest\r\nDTSTART;TZID=W. Europe Standard Time:20261106T160000\r\n\
         DTEND;TZID=W. Europe Standard Time:20261106T180000\r\nDTSTAMP:20261023T061500Z\r\n\
         TRANSP:TRANSPARENT\r\n{}END:VEVENT\r\n",
        cdo("BUSY")
    );
    // Two occurrences of a foreign series without master; the first one is
    // exported twice (older SEQUENCE first).
    let foreign_uid = "040000008200E00074C5B7101A82E008000000000000000000000000000000000004";
    let occurrence = |rid: &str, start: &str, end: &str, seq: u32, title: &str| {
        format!(
            "BEGIN:VEVENT\r\nUID:{foreign_uid}\r\n\
             RECURRENCE-ID;TZID=W. Europe Standard Time:{rid}\r\n\
             SUMMARY:{title}\r\nDTSTART;TZID=W. Europe Standard Time:{start}\r\n\
             DTEND;TZID=W. Europe Standard Time:{end}\r\nDTSTAMP:20261023T061500Z\r\n\
             SEQUENCE:{seq}\r\n{}END:VEVENT\r\n",
            cdo("TENTATIVE")
        )
    };
    format!(
        "BEGIN:VCALENDAR\r\nMETHOD:PUBLISH\r\nPRODID:Microsoft Exchange Server 2010\r\n\
         VERSION:2.0\r\nX-WR-CALNAME:Kalender\r\n{tz}{series}{focus}{vacation}{transparent}{}{}{}\
         END:VCALENDAR\r\n",
        occurrence(
            "20261105T140000",
            "20261105T140000",
            "20261105T150000",
            0,
            "Lenkungskreis"
        ),
        occurrence(
            "20261119T140000",
            "20261119T140000",
            "20261119T150000",
            0,
            "Lenkungskreis"
        ),
        occurrence(
            "20261105T140000",
            "20261105T150000",
            "20261105T160000",
            1,
            "Lenkungskreis (verschoben)"
        ),
    )
}

// ----- the fake feed server -------------------------------------------------------------

struct FeedState {
    body: String,
    version: u32,
    etag: bool,
    last_modified: bool,
    status: Option<u16>,
    required_auth: Option<String>,
    /// (If-None-Match, If-Modified-Since, Authorization present) per request.
    requests: Vec<(Option<String>, Option<String>, bool)>,
}

#[derive(Clone)]
struct FakeFeed(Arc<Mutex<FeedState>>);

impl FakeFeed {
    fn new(body: String) -> Self {
        FakeFeed(Arc::new(Mutex::new(FeedState {
            body,
            version: 1,
            etag: true,
            last_modified: true,
            status: None,
            required_auth: None,
            requests: Vec::new(),
        })))
    }

    fn with<T>(&self, f: impl FnOnce(&mut FeedState) -> T) -> T {
        f(&mut self.0.lock().unwrap())
    }

    fn set_body(&self, body: String) {
        self.with(|s| {
            s.body = body;
            s.version += 1;
        });
    }

    fn requests(&self) -> Vec<(Option<String>, Option<String>, bool)> {
        self.with(|s| s.requests.clone())
    }
}

impl Respond for FakeFeed {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let header = |name: &str| {
            request
                .headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        self.with(|s| {
            let auth = header("authorization");
            s.requests.push((
                header("if-none-match"),
                header("if-modified-since"),
                auth.is_some(),
            ));
            if let Some(status) = s.status {
                return ResponseTemplate::new(status).set_body_string("Fehler");
            }
            if let Some(required) = &s.required_auth {
                if auth.as_ref() != Some(required) {
                    return ResponseTemplate::new(401)
                        .insert_header("WWW-Authenticate", "Basic realm=\"Kalender\"");
                }
            }
            let etag = format!("\"v{}\"", s.version);
            let last_modified = format!("Mon, {:02} Oct 2026 06:00:00 GMT", s.version % 28 + 1);
            let not_modified = (s.etag && header("if-none-match").as_ref() == Some(&etag))
                || (!s.etag
                    && s.last_modified
                    && header("if-modified-since").as_ref() == Some(&last_modified));
            let mut r = if not_modified {
                ResponseTemplate::new(304)
            } else {
                ResponseTemplate::new(200)
                    .set_body_raw(s.body.clone(), "text/calendar; charset=utf-8")
            };
            if s.etag {
                r = r.insert_header("ETag", etag.as_str());
            }
            if s.last_modified {
                r = r.insert_header("Last-Modified", last_modified.as_str());
            }
            r
        })
    }
}

// ----- helpers --------------------------------------------------------------------------

struct Setup {
    s: TestServer,
    session: String,
    server: MockServer,
    feed: FakeFeed,
}

async fn setup(body: String) -> Setup {
    logs();
    let server = MockServer::start().await;
    let feed = FakeFeed::new(body);
    Mock::given(method("GET"))
        .and(path(FEED_PATH))
        .and(query_param("token", query_token()))
        .respond_with(feed.clone())
        .mount(&server)
        .await;
    let s = TestServer::new().await;
    let session = s.session().await;
    Setup {
        s,
        session,
        server,
        feed,
    }
}

impl Setup {
    fn url(&self) -> String {
        format!("{}{FEED_PATH}?token={}", self.server.uri(), query_token())
    }

    async fn create(&self, body: Value) -> common::TestResponse {
        self.s
            .post("/api/accounts", Auth::Web(&self.session), body)
            .await
    }

    async fn create_ok(&self) -> String {
        let r = self
            .create(json!({"kind": "ics", "url": self.url(), "name": "Arbeit"}))
            .await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
        r.json()["id"].as_str().unwrap().to_owned()
    }

    async fn calendar(&self) -> Value {
        let cals = self
            .s
            .get("/api/calendars", Auth::Bearer(BOOTSTRAP))
            .await
            .json();
        assert_eq!(cals.as_array().unwrap().len(), 1, "{cals}");
        cals[0].clone()
    }

    async fn events(&self) -> Vec<Value> {
        let id = self.calendar().await["id"].as_str().unwrap().to_owned();
        self.s.app.calendar_events(&id).await.unwrap()
    }

    async fn event(&self, uid: &str) -> Value {
        self.events()
            .await
            .into_iter()
            .find(|e| e["uid"] == uid)
            .unwrap_or_else(|| panic!("no event {uid}"))
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.s.app.config().db_path()).unwrap()
    }

    fn event_seq(&self, uid: &str) -> i64 {
        self.db()
            .query_row("SELECT seq FROM events WHERE uid = ?1", [uid], |r| r.get(0))
            .unwrap()
    }

    fn event_tombstones(&self) -> Vec<String> {
        changes::tombstones_since(&self.db(), 0)
            .unwrap()
            .into_iter()
            .filter(|t| t.kind == "event")
            .map(|t| t.id)
            .collect()
    }

    async fn status(&self, id: &str) -> (String, Option<String>) {
        let a = self.s.app.account(id).await.unwrap();
        (a.status.state.to_owned(), a.status.message)
    }
}

fn assert_no_secret(text: &str) {
    assert!(!text.contains(&query_token()), "query token leaked: {text}");
    assert!(
        !text.contains("5d41402abc4b2a76b9719d911017c592"),
        "path key leaked: {text}"
    );
    assert!(!text.contains(&feed_password()), "password leaked: {text}");
}

// ----- tests ----------------------------------------------------------------------------

#[tokio::test]
async fn creating_a_feed_account_imports_one_read_only_calendar() {
    let t = setup(standard_feed()).await;
    let r = t
        .create(json!({"kind": "ics", "url": t.url(), "name": "Arbeit"}))
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert_no_secret(&r.text());
    let account = r.json();
    assert_eq!(account["kind"], "ics");
    assert_eq!(account["name"], "Arbeit");
    assert_eq!(account["status"]["state"], "ok");
    assert!(account["status"]["last_sync_at"].is_string());
    assert!(account.get("url").is_none(), "no url field for ics");
    assert_eq!(
        account["username"],
        format!("{}/owa/calendar/…/kalender.ics", t.server.uri())
    );

    let cal = t.calendar().await;
    assert_eq!(cal["name"], "Arbeit (Abo)", "X-WR-CALNAME");
    assert_eq!(cal["provider_color"], "#4F46E5");
    assert_eq!(cal["color"], "#4F46E5");
    assert_eq!(cal["writable"], false);
    assert_eq!(cal["can_invite"], false);
    assert_eq!(cal["mcp_write"], false);

    let events = t.events().await;
    assert_eq!(events.len(), 2);
    let planning = t.event("planung@example.org").await;
    assert_eq!(planning["title"], "Quartalsplanung");
    assert_eq!(planning["version"], 1);
    assert_eq!(
        planning["start"],
        json!({"date_time": "2026-11-03T09:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-03T08:00:00Z"})
    );

    // The same feed again: 409.
    let r = t.create(json!({"kind": "ics", "url": t.url()})).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text());

    // Without a name the account takes X-WR-CALNAME (here: a second,
    // different feed URL on the same server is unknown → 422).
    let other = format!("{}{FEED_PATH}?token=anders", t.server.uri());
    let r = t.create(json!({"kind": "ics", "url": other})).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(r.error_code(), "invalid_feed");
}

#[tokio::test]
async fn unusable_feeds_are_refused_with_german_messages() {
    let t = setup(standard_feed()).await;

    // 404: no feed under this address.
    let missing = format!(
        "{}/gibt-es-nicht.ics?token={}",
        t.server.uri(),
        query_token()
    );
    let r = t.create(json!({"kind": "ics", "url": missing})).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    assert_eq!(r.error_code(), "invalid_feed");
    assert!(r.text().contains("HTTP 404"), "{}", r.text());
    assert_no_secret(&r.text());

    // 200, but an HTML login page.
    t.feed
        .set_body("<!doctype html><html><body>Bitte anmelden</body></html>".into());
    let r = t.create(json!({"kind": "ics", "url": t.url()})).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(r.error_code(), "invalid_feed");
    assert!(r.text().contains("kein Kalender"), "{}", r.text());

    // Broken iCalendar.
    t.feed
        .set_body("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:kaputt\r\n".into());
    let r = t.create(json!({"kind": "ics", "url": t.url()})).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(r.text().contains("nicht lesbar"), "{}", r.text());

    // Server error → 502.
    t.feed.with(|s| s.status = Some(503));
    let r = t.create(json!({"kind": "ics", "url": t.url()})).await;
    assert_eq!(r.status, StatusCode::BAD_GATEWAY);
    assert_eq!(r.error_code(), "provider_unavailable");

    // Login required → 422 invalid_credentials.
    t.feed.with(|s| s.status = Some(401));
    let r = t.create(json!({"kind": "ics", "url": t.url()})).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(r.error_code(), "invalid_credentials");

    // Unknown scheme, password without user.
    let r = t
        .create(json!({"kind": "ics", "url": "ftp://feeds.example.org/k.ics"}))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = t
        .create(json!({"kind": "ics", "url": t.url(), "password": feed_password()}))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_no_secret(&r.text());

    // Nothing was stored.
    let list =
        t.s.get("/api/accounts", Auth::Bearer(BOOTSTRAP))
            .await
            .json();
    assert_eq!(list, json!([]));
    assert_eq!(t.s.app.change_cursor(), 0);
}

#[tokio::test]
async fn basic_auth_is_checked_and_stored_encrypted() {
    let t = setup(standard_feed()).await;
    let expected = basic_header(FEED_USER, &feed_password());
    t.feed.with(|s| s.required_auth = Some(expected));

    let r = t
        .create(json!({"kind": "ics", "url": t.url(), "username": FEED_USER,
                       "password": wrong_password()}))
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(r.error_code(), "invalid_credentials");

    let r = t
        .create(json!({"kind": "ics", "url": t.url(), "username": FEED_USER,
                       "password": feed_password()}))
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert_no_secret(&r.text());
    let id = r.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(t.events().await.len(), 2);

    // Stored only encrypted: neither URL token nor password in the database.
    let db = t.db();
    let (username, base_url, blob): (String, String, Vec<u8>) = db
        .query_row(
            "SELECT username, base_url, credentials FROM accounts WHERE id = ?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_no_secret(&username);
    assert_no_secret(&base_url);
    assert_no_secret(&String::from_utf8_lossy(&blob));

    // The regular run sends the credentials as well.
    t.feed.set_body(feed(&[event("neu@example.org", "Neu", 9)]));
    let report = t.s.app.sync_account(&id).await.unwrap();
    assert_eq!(
        (report.events.added, report.events.deleted),
        (1, 2),
        "{report:?}"
    );
    assert!(t.feed.requests().iter().all(|(_, _, auth)| *auth));

    // Credentials withdrawn at the server: auth_failed.
    t.feed
        .with(|s| s.required_auth = Some(basic_header(FEED_USER, "neu")));
    t.feed.set_body(standard_feed());
    let err = t.s.app.sync_account(&id).await.unwrap_err();
    assert_eq!(err.code(), "invalid_credentials");
    let (state, message) = t.status(&id).await;
    assert_eq!(state, "auth_failed");
    assert!(message.unwrap().contains("HTTP 401"));
}

#[tokio::test]
async fn unchanged_feed_answers_304_and_changes_nothing() {
    let t = setup(standard_feed()).await;
    let id = t.create_ok().await;
    let before = t.events().await;
    let cursor = t.s.app.change_cursor();

    let report = t.s.app.sync_account(&id).await.unwrap();
    assert_eq!(report.events, Default::default());
    let requests = t.feed.requests();
    let (inm, ims, _) = requests.last().unwrap();
    assert_eq!(inm.as_deref(), Some("\"v1\""), "If-None-Match sent");
    assert!(ims.is_some(), "If-Modified-Since sent");
    assert_eq!(t.events().await, before);
    let (state, _) = t.status(&id).await;
    assert_eq!(state, "ok");
    // Only the account (status) changed, no event.
    let db = t.db();
    let changed: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM events WHERE seq > ?1",
            [cursor],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(changed, 0);

    // Server without ETag: Last-Modified alone.
    t.feed.with(|s| s.etag = false);
    t.feed.set_body(standard_feed());
    t.s.app.sync_account(&id).await.unwrap();
    let report = t.s.app.sync_account(&id).await.unwrap();
    let (inm, ims, _) = t.feed.requests().last().unwrap().clone();
    assert_eq!(inm, None);
    assert!(ims.is_some());
    assert_eq!(report.events, Default::default());
}

#[tokio::test]
async fn new_changed_and_removed_events_with_tombstones_and_seq() {
    let t = setup(standard_feed()).await;
    let id = t.create_ok().await;
    let planning = t.event("planung@example.org").await;
    let review = t.event("review@example.org").await;
    let cursor = t.s.app.change_cursor();

    // A new DTSTAMP alone (many feeds stamp every request): no new version.
    t.feed
        .set_body(standard_feed().replace("DTSTAMP:20261023T060000Z", "DTSTAMP:20261023T071500Z"));
    let report = t.s.app.sync_account(&id).await.unwrap();
    assert_eq!(report.events.unchanged, 2);
    assert_eq!((report.events.added, report.events.updated), (0, 0));
    assert_eq!(t.event("planung@example.org").await["version"], 1);

    t.feed.set_body(feed(&[
        event("planung@example.org", "Quartalsplanung (verschoben)", 5),
        event("offsite@example.org", "Offsite", 12),
    ]));
    let report = t.s.app.sync_account(&id).await.unwrap();
    assert_eq!(
        (
            report.events.added,
            report.events.updated,
            report.events.deleted
        ),
        (1, 1, 1),
        "{report:?}"
    );
    let changed = t.event("planung@example.org").await;
    assert_eq!(changed["id"], planning["id"], "same event, new version");
    assert_eq!(changed["version"], 2);
    assert_eq!(changed["title"], "Quartalsplanung (verschoben)");
    assert!(t.event_seq("planung@example.org") > cursor);
    assert!(t.event_seq("offsite@example.org") > cursor);
    assert_eq!(t.event_tombstones(), [review["id"].as_str().unwrap()]);
    assert_eq!(t.events().await.len(), 2);

    // The stored original is the event's own VCALENDAR.
    let ics: String = t
        .db()
        .query_row(
            "SELECT ics FROM events WHERE uid = 'offsite@example.org'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
    assert!(ics.contains("X-WR-CALNAME:Arbeit (Abo)"));
    assert!(!ics.contains("planung@example.org"));

    // Feed emptied: everything goes, each with a tombstone.
    t.feed.set_body(feed(&[]));
    let report = t.s.app.sync_account(&id).await.unwrap();
    assert_eq!(report.events.deleted, 2);
    assert!(t.events().await.is_empty());
    assert_eq!(t.event_tombstones().len(), 3);

    // POST /api/accounts/{id}/sync starts a run (202).
    let r =
        t.s.post(
            &format!("/api/accounts/{id}/sync"),
            Auth::Bearer(BOOTSTRAP),
            json!({}),
        )
        .await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text());
}

#[tokio::test]
async fn feeds_over_the_size_limit_are_refused() {
    let filler = format!(
        "X-FUELLTEXT:{}\r\n",
        "x".repeat(lichtung_server::ics_feed::MAX_FEED_BYTES + 10)
    );
    let big = standard_feed().replace("METHOD:PUBLISH\r\n", &format!("METHOD:PUBLISH\r\n{filler}"));
    let t = setup(big).await;
    let r = t.create(json!({"kind": "ics", "url": t.url()})).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    assert_eq!(r.error_code(), "invalid_feed");
    assert!(r.text().contains("größer als 20 MB"), "{}", r.text());

    // An account whose feed grows too large: status error, events stay.
    t.feed.set_body(standard_feed());
    let id = t.create_ok().await;
    let big = standard_feed().replace("METHOD:PUBLISH\r\n", &format!("METHOD:PUBLISH\r\n{filler}"));
    t.feed.set_body(big);
    assert!(t.s.app.sync_account(&id).await.is_err());
    let (state, message) = t.status(&id).await;
    assert_eq!(state, "error");
    assert!(message.unwrap().contains("20 MB"));
    assert_eq!(t.events().await.len(), 2);
}

#[tokio::test]
async fn webcal_urls_work() {
    let t = setup(standard_feed()).await;
    // webcal:// tries HTTPS first; the mock speaks plain HTTP only.
    let url = t.url().replacen("http://", "webcal://", 1);
    let r = t
        .create(json!({"kind": "ics", "url": url, "name": "Abo"}))
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert!(r.json()["username"]
        .as_str()
        .unwrap()
        .starts_with("http://127.0.0.1:"));
    assert_eq!(t.events().await.len(), 2);

    // webcals:// is HTTPS only: the plain-HTTP mock cannot answer.
    let url = t.url().replacen("http://", "webcals://", 1);
    let r = t.create(json!({"kind": "ics", "url": url})).await;
    assert_eq!(r.status, StatusCode::BAD_GATEWAY, "{}", r.text());
}

#[tokio::test]
async fn redirects_are_followed_up_to_five_times() {
    let t = setup(standard_feed()).await;
    let base = t.server.uri();
    for i in 0..7 {
        let target = if i == 6 {
            t.url()
        } else {
            format!("{base}/umleitung/{}", i + 1)
        };
        Mock::given(method("GET"))
            .and(path(format!("/umleitung/{i}")))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", target.as_str()))
            .mount(&t.server)
            .await;
    }
    // 2 hops: fine.
    let r = t
        .create(json!({"kind": "ics", "url": format!("{base}/umleitung/5")}))
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    // 7 hops: too many.
    let r = t
        .create(json!({"kind": "ics", "url": format!("{base}/umleitung/0")}))
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    assert!(r.text().contains("weiter"), "{}", r.text());
}

#[tokio::test]
async fn outlook_style_feed_is_read_robustly() {
    let t = setup(outlook_feed()).await;
    t.create_ok().await;
    let events = t.events().await;
    assert_eq!(events.len(), 5, "{events:#?}");
    assert!(events.iter().all(|e| e.get("parse_error").is_none()));

    let series = t
        .event("040000008200E00074C5B7101A82E00800000000A1B2C3D4E5F6A701000000000000000010000000F1E2D3C4B5A69788")
        .await;
    assert_eq!(series["title"], "Jour fixe Team");
    assert_eq!(
        series["start"],
        json!({"date_time": "2026-11-02T10:00:00", "time_zone": "Europe/Berlin", "utc": "2026-11-02T09:00:00Z"})
    );
    assert_eq!(series["availability"], "busy");
    assert!(series["recurrence"]["rrule"]
        .as_str()
        .unwrap()
        .starts_with("FREQ=WEEKLY"));

    let uid =
        |n: u32| format!("040000008200E00074C5B7101A82E00800000000000000000000000000000000000{n}");
    let focus = t.event(&uid(1)).await;
    assert_eq!(
        focus["availability"], "free",
        "BUSYSTATUS:FREE without TRANSP"
    );
    let vacation = t.event(&uid(2)).await;
    assert_eq!(vacation["availability"], "busy", "OOF");
    assert_eq!(vacation["all_day"], true);
    let info = t.event(&uid(3)).await;
    assert_eq!(info["availability"], "free", "TRANSP wins over BUSYSTATUS");

    // Foreign series without master, one occurrence exported twice.
    let foreign = t.event(&uid(4)).await;
    assert_eq!(foreign["recurrence"]["rrule"], Value::Null);
    let overrides = foreign["overrides"].as_array().unwrap();
    assert_eq!(overrides.len(), 2, "{foreign:#}");
    assert_eq!(overrides[0]["title"], "Lenkungskreis (verschoben)");
    assert_eq!(overrides[0]["availability"], "busy", "TENTATIVE blocks");
    assert_eq!(
        overrides[0]["start"]["utc"], "2026-11-05T14:00:00Z",
        "newer SEQUENCE wins"
    );

    // The server-side expansion sees every occurrence once.
    let r =
        t.s.get(
            "/api/occurrences?from=2026-11-01&to=2026-12-01&time_zone=Europe/Berlin",
            Auth::Bearer(BOOTSTRAP),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let occurrences = r.json()["occurrences"].as_array().unwrap().clone();
    let count = |title: &str| occurrences.iter().filter(|o| o["title"] == title).count();
    assert_eq!(count("Jour fixe Team"), 5);
    assert_eq!(count("Lenkungskreis (verschoben)"), 1);
    assert_eq!(count("Lenkungskreis"), 1);
    assert_eq!(count("Fokuszeit"), 1);
}

#[tokio::test]
async fn the_secret_url_never_shows_up_in_answers_or_logs() {
    let t = setup(standard_feed()).await;
    let id = t.create_ok().await;

    // Error states carry no URL either.
    t.feed.with(|s| s.status = Some(500));
    assert!(t.s.app.sync_account(&id).await.is_err());
    let (state, message) = t.status(&id).await;
    assert_eq!(state, "error");
    assert_no_secret(&message.unwrap());
    t.feed.with(|s| s.status = None);
    t.s.app.sync_account(&id).await.unwrap();

    for path in ["/api/accounts", "/api/calendars"] {
        let r = t.s.get(path, Auth::Bearer(BOOTSTRAP)).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_no_secret(&r.text());
    }

    // Renaming keeps the URL; a new URL is checked before it is stored.
    let r =
        t.s.request(
            Method::PATCH,
            &format!("/api/accounts/{id}"),
            Auth::Web(&t.session),
            Some(json!({"name": "Arbeit (Outlook)"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["name"], "Arbeit (Outlook)");
    assert_no_secret(&r.text());
    let wrong = format!("{}{FEED_PATH}?token=falsch", t.server.uri());
    let r =
        t.s.request(
            Method::PATCH,
            &format!("/api/accounts/{id}"),
            Auth::Web(&t.session),
            Some(json!({"url": wrong})),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    t.s.app.sync_account(&id).await.unwrap();

    // A Basic-auth password added later is checked against the feed too.
    t.feed
        .with(|s| s.required_auth = Some(basic_header(FEED_USER, &feed_password())));
    let r =
        t.s.request(
            Method::PATCH,
            &format!("/api/accounts/{id}"),
            Auth::Web(&t.session),
            Some(json!({"username": FEED_USER, "password": feed_password()})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_no_secret(&r.text());
    t.s.app.sync_account(&id).await.unwrap();

    let logs = log_text();
    assert!(logs.contains("account connected"), "logs are captured");
    assert_no_secret(&logs);
}

#[tokio::test]
async fn feed_accounts_use_their_own_interval() {
    use lichtung_server::accounts::AccountKind;
    use lichtung_server::sync::{sync_interval, SYNC_INTERVAL};
    assert_eq!(
        sync_interval(AccountKind::Ics),
        std::time::Duration::from_secs(15 * 60)
    );
    assert_eq!(sync_interval(AccountKind::Fastmail), SYNC_INTERVAL);
}
