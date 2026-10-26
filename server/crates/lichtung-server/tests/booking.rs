//! The public booking page (LIC-33) and its protection (LIC-34) against
//! the invented, stateful CalDAV server: free times only, languages, time
//! zones, booking with the guest as attendee, one-off links used up,
//! races, refused forms, rate limits, unknown links. All data is made up.

mod common;

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};

use common::dav::{setup_configured, setup_with, vcal, Setup, CALENDAR, USER};
use common::{device_token, Auth, TestResponse, TestServer};

/// A day a week ahead, 00:00 UTC.
fn day() -> DateTime<Utc> {
    let d = (Utc::now() + Duration::days(7)).date_naive();
    d.and_hms_opt(0, 0, 0).unwrap().and_utc()
}

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    day() + Duration::hours(hour.into()) + Duration::minutes(minute.into())
}

fn rfc(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn date(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d").to_string()
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// A busy event 10:00–11:00 UTC on [`day`] with details that must never
/// reach the page.
fn secret_meeting() -> String {
    let d = day().format("%Y%m%d");
    vcal(&format!(
        "BEGIN:VEVENT\r\nUID:geheim@example.org\r\nDTSTAMP:20261020T070000Z\r\n\
         DTSTART:{d}T100000Z\r\nDTEND:{d}T110000Z\r\nSUMMARY:Geheimes Treffen\r\n\
         LOCATION:Hinterzimmer am Markt\r\n\
         ORGANIZER;CN=Cora Clever:mailto:cora@example.org\r\n\
         ATTENDEE;CN=Dora Dritte:mailto:dora@example.org\r\nEND:VEVENT\r\n"
    ))
}

async fn prepared() -> (Setup, String) {
    prepared_with(|_| {}).await
}

async fn prepared_with(adjust: impl FnOnce(&mut lichtung_server::Config)) -> (Setup, String) {
    let t = setup_configured(|dav| dav.put("geheim.ics", &secret_meeting()), adjust).await;
    t.sync().await;
    let device = device_token(&t.s, "iPhone von Anna");
    (t, device)
}

const GUEST: &str = "203.0.113.5:40000";

/// A request from `peer` (as the server sees the connection).
fn request(method: Method, path: &str, peer: &str) -> axum::http::request::Builder {
    let mut builder = Request::builder().method(method).uri(path);
    builder
        .extensions_mut()
        .unwrap()
        .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    builder
}

async fn get_page(s: &TestServer, path: &str) -> TestResponse {
    get_from(s, path, GUEST).await
}

async fn get_from(s: &TestServer, path: &str, peer: &str) -> TestResponse {
    s.send(
        request(Method::GET, path, peer)
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

async fn post_form(s: &TestServer, path: &str, fields: &[(&str, &str)]) -> TestResponse {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    s.send(
        request(Method::POST, path, GUEST)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::ORIGIN, "null")
            .body(Body::from(body))
            .unwrap(),
    )
    .await
}

/// The value of a hidden field in a page.
fn field(html: &str, name: &str) -> String {
    let marker = format!("name=\"{name}\" value=\"");
    let start = html
        .find(&marker)
        .unwrap_or_else(|| panic!("no {name}: {html}"))
        + marker.len();
    let end = html[start..].find('"').unwrap();
    html[start..start + end].to_owned()
}

impl Setup {
    async fn create_opening(&self, device: &str, body: Value) -> Value {
        let r = self
            .s
            .request(
                Method::POST,
                "/api/openings",
                Auth::Bearer(device),
                Some(body),
            )
            .await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
        r.json()
    }

    /// A template on all days 08:00–18:00 UTC, 30 minutes, no notice.
    async fn template(&self, device: &str) -> Value {
        let windows: Vec<Value> = ["mo", "tu", "we", "th", "fr", "sa", "su"]
            .iter()
            .map(|d| json!({"weekday": d, "start": "08:00", "end": "18:00"}))
            .collect();
        self.create_opening(
            device,
            json!({"kind": "template", "title": "30 Minuten Kennenlernen", "duration_minutes": 30,
                   "calendar_id": self.calendar_id, "time_zone": "UTC", "windows": windows,
                   "min_notice_minutes": 0, "video_url": "https://meet.example.org/kennenlernen"}),
        )
        .await
    }

    /// A one-off link with suggestions at 09:00 and 14:00 UTC of [`day`].
    async fn once(&self, device: &str) -> Value {
        self.create_opening(
            device,
            json!({"kind": "once", "title": "Abstimmung", "duration_minutes": 30,
                   "calendar_id": self.calendar_id, "time_zone": "Europe/Berlin",
                   "suggestions": [{"start": rfc(at(9, 0))}, {"start": rfc(at(14, 0))}]}),
        )
        .await
    }

    /// A form token for `start`, issued `age` ago (past the minimum fill
    /// time).
    fn signed(&self, opening: &Value, start: DateTime<Utc>, age: Duration) -> String {
        lichtung_server::booking::guard::sign_form(
            &self.s.app.config().secret_key,
            token_of(opening),
            start.timestamp_millis(),
            (Utc::now() - age).timestamp_millis(),
        )
    }

    /// Books `start` as `name` <`email`> with a valid, aged form token.
    async fn book(
        &self,
        opening: &Value,
        start: DateTime<Utc>,
        name: &str,
        email: &str,
    ) -> TestResponse {
        let form = self.signed(opening, start, Duration::seconds(10));
        post_form(
            &self.s,
            &path_of(opening),
            &[
                ("slot", &rfc(start)),
                ("tz", "Europe/Berlin"),
                ("lang", "de"),
                ("form", &form),
                ("name", name),
                ("email", email),
                ("note", "Ich bringe Unterlagen mit."),
                ("website", ""),
            ],
        )
        .await
    }

    /// Stored objects at the provider whose data contains `needle`
    /// (unfolded).
    fn at_provider(&self, needle: &str) -> Vec<String> {
        self.dav
            .with(|s| {
                s.calendars[CALENDAR]
                    .resources
                    .values()
                    .map(|(_, d)| d.replace("\r\n ", ""))
                    .collect::<Vec<_>>()
            })
            .into_iter()
            .filter(|d| d.contains(needle))
            .collect()
    }
}

fn token_of(opening: &Value) -> &str {
    opening["url"].as_str().unwrap().rsplit('/').next().unwrap()
}

fn path_of(opening: &Value) -> String {
    format!("/o/{}", token_of(opening))
}

#[tokio::test]
async fn the_page_shows_free_times_and_nothing_else() {
    let (t, device) = prepared().await;
    let o = t.template(&device).await;
    let path = path_of(&o);
    let r = get_page(&t.s, &format!("{path}?from={}&tz=UTC&lang=de", date(day()))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let html = r.text();
    assert!(html.contains("30 Minuten Kennenlernen"));
    assert!(html.contains("Wähle einen Termin."));
    assert!(html.contains("<html lang=\"de\">"));
    // 09:30 ends when the meeting starts, 10:00 and 10:30 touch it.
    let slot = |t: DateTime<Utc>| format!("slot={}", enc(&rfc(t)));
    assert!(html.contains(&slot(at(9, 30))), "{html}");
    assert!(!html.contains(&slot(at(10, 0))));
    assert!(!html.contains(&slot(at(10, 30))));
    assert!(html.contains(&slot(at(11, 0))));
    // Nothing of the busy event, nothing of the link's internals.
    for secret in [
        "Geheim",
        "Hinterzimmer",
        "Cora Clever",
        "cora@example.org",
        "Dora Dritte",
        "dora@example.org",
        USER,
        "meet.example.org",
        o["id"].as_str().unwrap(),
        o["calendar_id"].as_str().unwrap(),
    ] {
        assert!(!html.contains(secret), "{secret} leaked");
    }

    // Strict headers.
    let csp = r.header("content-security-policy").unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains("script-src 'sha256-") && !csp.contains("http"));
    assert_eq!(r.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(r.header("x-robots-tag"), Some("noindex, nofollow"));
    assert_eq!(r.header("x-frame-options"), Some("DENY"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    assert!(html.contains("<script>"), "time zone of the browser");

    // English by Accept-Language or ?lang=; ?lang= wins.
    let r =
        t.s.send(
            request(Method::GET, &format!("{path}?tz=UTC"), GUEST)
                .header(header::ACCEPT_LANGUAGE, "en-GB,en;q=0.9,de;q=0.5")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(r.text().contains("Pick a time."), "{}", r.text());
    assert!(r.text().contains("<html lang=\"en\">"));
    let r =
        t.s.send(
            request(Method::GET, &format!("{path}?tz=UTC&lang=de"), GUEST)
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(r.text().contains("Wähle einen Termin."));

    // The form shows the time, not more.
    let r = get_page(
        &t.s,
        &format!("{path}?slot={}&tz=UTC&lang=en", enc(&rfc(at(11, 0)))),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let html = r.text();
    assert!(html.contains("11:00–11:30 (UTC)"), "{html}");
    assert!(html.contains("name=\"website\""), "honeypot");
    assert!(!html.contains("Geheim") && !html.contains("meet.example.org"));
}

#[tokio::test]
async fn times_follow_the_guests_zone() {
    let (t, device) = prepared().await;
    let o = t.template(&device).await;
    let path = path_of(&o);
    // India (+05:30, no daylight saving): 08:00 UTC is 13:30.
    let r = get_page(
        &t.s,
        &format!("{path}?from={}&tz=Asia/Kolkata&lang=en", date(day())),
    )
    .await;
    let html = r.text();
    assert!(html.contains(">13:30</a>"), "{html}");
    assert!(!html.contains(">08:00</a>"));
    assert!(html.contains("<option selected>Asia/Kolkata</option>"));
    // The slot itself stays an instant.
    assert!(html.contains(&format!("slot={}", enc(&rfc(at(8, 0))))));

    // Without ?tz= (no script ran): the link's zone, visible.
    let r = get_page(&t.s, &format!("{path}?from={}", date(day()))).await;
    assert!(r.text().contains("<option selected>UTC</option>"));
    assert!(r.text().contains(">08:00</a>"));
    // An unknown zone falls back as well.
    let r = get_page(&t.s, &format!("{path}?from={}&tz=Mars/Olymp", date(day()))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.text().contains("<option selected>UTC</option>"));
}

#[tokio::test]
async fn booking_invites_the_guest_and_uses_up_a_one_off_link() {
    let (t, device) = prepared().await;
    let o = t.once(&device).await;
    let path = path_of(&o);
    assert_eq!(t.at_provider("SUMMARY:Reserviert: ").len(), 2);

    // Both suggestions are offered, from the first one's day on.
    let r = get_page(&t.s, &format!("{path}?tz=Europe/Berlin&lang=de")).await;
    let html = r.text();
    assert!(
        html.contains(&format!("slot={}", enc(&rfc(at(9, 0))))),
        "{html}"
    );
    assert!(html.contains(&format!("slot={}", enc(&rfc(at(14, 0))))));

    // The form; sent back at once it counts as too fast.
    let r = get_page(
        &t.s,
        &format!(
            "{path}?slot={}&tz=Europe/Berlin&lang=de",
            enc(&rfc(at(9, 0)))
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let form = field(&r.text(), "form");
    let fast = post_form(
        &t.s,
        &path,
        &[
            ("slot", &rfc(at(9, 0))),
            ("tz", "Europe/Berlin"),
            ("lang", "de"),
            ("form", &form),
            ("name", "Bert Beispiel"),
            ("email", "bert@example.org"),
            ("website", ""),
        ],
    )
    .await;
    assert_eq!(fast.status, StatusCode::BAD_REQUEST);
    assert!(fast.text().contains("noch einmal"), "{}", fast.text());
    assert!(fast.text().contains("value=\"Bert Beispiel\""), "kept");
    assert!(t.at_provider("RSVP=TRUE").is_empty());

    let r = t
        .book(&o, at(9, 0), "Bert Beispiel", "bert@example.org")
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let html = r.text();
    assert!(html.contains("Gebucht"));
    assert!(html.contains("bert@example.org"));
    let local = at(9, 0).with_timezone(&chrono_tz::Europe::Berlin);
    assert!(
        html.contains(&format!("{}–", local.format("%H:%M"))),
        "time in the guest's zone: {html}"
    );

    // The event: guest invited, own address organizes, note and title.
    let booked = t.at_provider("SUMMARY:Abstimmung (Bert Beispiel)");
    assert_eq!(booked.len(), 1, "{booked:?}");
    let ics = &booked[0];
    // The account's own address from discovery organizes.
    assert!(
        ics.contains("ORGANIZER:mailto:anna.alt@example.org"),
        "{ics}"
    );
    let attendee = ics
        .lines()
        .find(|l| l.starts_with("ATTENDEE"))
        .expect("attendee");
    assert!(attendee.contains("CN=Bert Beispiel"), "{attendee}");
    assert!(attendee.contains("PARTSTAT=NEEDS-ACTION"));
    assert!(attendee.contains("RSVP=TRUE"));
    assert!(attendee.ends_with(":mailto:bert@example.org"));
    assert!(ics.contains("Ich bringe Unterlagen mit."));
    assert!(ics.contains("TZID=Europe/Berlin"));

    // The link is used up, its placeholders are gone.
    assert!(t.at_provider("SUMMARY:Reserviert: ").is_empty());
    let id = o["id"].as_str().unwrap();
    let now =
        t.s.get(&format!("/api/openings/{id}"), Auth::Bearer(&device))
            .await
            .json();
    assert!(now["booked_event_id"].as_str().unwrap().starts_with("ev_"));
    let r = get_page(&t.s, &path).await;
    assert_eq!(r.status, StatusCode::GONE);
    assert!(r.text().contains("bereits") || r.text().contains("already"));
    let r = t
        .book(&o, at(14, 0), "Cora Clever", "cora@example.org")
        .await;
    assert_eq!(r.status, StatusCode::GONE);
    assert_eq!(t.at_provider("RSVP=TRUE").len(), 1);
}

#[tokio::test]
async fn only_one_of_two_bookings_of_a_time_wins() {
    let (t, device) = prepared().await;
    let o = t.template(&device).await;
    let (a, b) = tokio::join!(
        t.book(&o, at(15, 0), "Bert Beispiel", "bert@example.org"),
        t.book(&o, at(15, 0), "Cora Clever", "cora@example.org"),
    );
    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
    let lost = if a.status == StatusCode::CONFLICT {
        &a
    } else {
        &b
    };
    assert!(lost.text().contains("vergeben"), "{}", lost.text());
    assert!(lost.text().contains("slot="), "fresh times");
    let booked = t.at_provider("RSVP=TRUE");
    assert_eq!(booked.len(), 1);
    assert!(
        booked[0].contains("LOCATION:https://meet.example.org/kennenlernen"),
        "the template's video link: {}",
        booked[0]
    );

    // The template stays bookable, the booked time is gone from the page.
    let r = get_page(
        &t.s,
        &format!("{}?from={}&tz=UTC", path_of(&o), date(day())),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!r.text().contains(&format!("slot={}", enc(&rfc(at(15, 0))))));
    assert!(r.text().contains(&format!("slot={}", enc(&rfc(at(16, 0))))));
    let r = t
        .book(&o, at(16, 0), "Dora Dritte", "dora@example.org")
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
}

#[tokio::test]
async fn past_busy_and_odd_times_are_refused() {
    let (t, device) = prepared().await;
    let o = t.template(&device).await;
    let past = day() - Duration::days(8);
    for start in [past, at(10, 0), at(11, 10), at(20, 0)] {
        let r = t.book(&o, start, "Bert Beispiel", "bert@example.org").await;
        assert_eq!(r.status, StatusCode::CONFLICT, "{start}: {}", r.text());
    }
    // Not a time at all.
    let form = t.signed(&o, at(11, 0), Duration::seconds(10));
    let r = post_form(
        &t.s,
        &path_of(&o),
        &[
            ("slot", "morgen"),
            ("form", &form),
            ("name", "Bert"),
            ("email", "bert@example.org"),
        ],
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    // The form of a busy time on GET: the list with a hint.
    let r = get_page(
        &t.s,
        &format!("{}?slot={}&tz=UTC", path_of(&o), enc(&rfc(at(10, 0)))),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.text().contains("slot=") && !r.text().contains("name=\"form\""));
    assert!(t.at_provider("RSVP=TRUE").is_empty());
}

#[tokio::test]
async fn spam_is_refused() {
    let (t, device) = prepared().await;
    let o = t.template(&device).await;
    let path = path_of(&o);
    let start = rfc(at(11, 0));
    let aged = t.signed(&o, at(11, 0), Duration::seconds(10));
    let base = |form: &str, name: &str, email: &str, website: &str| -> Vec<(String, String)> {
        [
            ("slot", start.as_str()),
            ("tz", "UTC"),
            ("lang", "en"),
            ("form", form),
            ("name", name),
            ("email", email),
            ("website", website),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
    };
    let send = |fields: Vec<(String, String)>| {
        let path = path.clone();
        let s = &t.s;
        async move {
            let pairs: Vec<(&str, &str)> = fields
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            post_form(s, &path, &pairs).await
        }
    };

    // Honeypot.
    let r = send(base(
        &aged,
        "Bert",
        "bert@example.org",
        "https://spam.example.org",
    ))
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    // Forged, foreign or other slot's form token.
    for form in [
        "1.abc".to_owned(),
        String::new(),
        t.signed(&o, at(12, 0), Duration::seconds(10)),
    ] {
        let r = send(base(&form, "Bert", "bert@example.org", "")).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{form}");
        assert!(
            !r.text().contains("name=\"form\""),
            "no fresh form for a forgery"
        );
    }
    // Too old.
    let old = t.signed(&o, at(11, 0), Duration::hours(3));
    let r = send(base(&old, "Bert", "bert@example.org", "")).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.text().contains("expired"));
    // Links in the name, bad address, overlong note.
    let r = send(base(
        &aged,
        "Cheap pills www.spam.example.org",
        "x@example",
        "",
    ))
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.text().contains("Please enter your name"));
    assert!(r.text().contains("valid email"));
    let mut long = base(&aged, "Bert", "bert@example.org", "");
    long.push(("note".into(), "x".repeat(1_001)));
    let r = send(long).await;
    assert!(r.text().contains("1000 characters"), "{}", r.text());
    // From another site.
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(base(&aged, "Bert", "bert@example.org", ""))
        .finish();
    let r =
        t.s.send(
            request(Method::POST, &path, GUEST)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::ORIGIN, "https://boese.example.org")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(t.at_provider("RSVP=TRUE").is_empty(), "nothing booked");
}

#[tokio::test]
async fn bookings_are_capped() {
    let (t, device) = prepared_with(|c| {
        c.booking_limits.bookings_per_email = 1;
        c.booking_limits.bookings_per_link = 2;
    })
    .await;
    let o = t.template(&device).await;
    let r = t
        .book(&o, at(11, 0), "Bert Beispiel", "bert@example.org")
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = t
        .book(&o, at(12, 0), "Bert Beispiel", "BERT@example.org")
        .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS, "per address");
    assert!(r.header("retry-after").is_some());
    let r = t
        .book(&o, at(12, 0), "Cora Clever", "cora@example.org")
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = t
        .book(&o, at(13, 0), "Dora Dritte", "dora@example.org")
        .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS, "per link");
    assert_eq!(t.at_provider("RSVP=TRUE").len(), 2);
}

#[tokio::test]
async fn unknown_links_look_alike_and_probing_is_limited() {
    let t = setup_with(|_| {}).await;
    let device = device_token(&t.s, "iPhone von Anna");
    let o = t.template(&device).await;
    let deleted = t.template(&device).await;
    let id = deleted["id"].as_str().unwrap();
    let r =
        t.s.delete(&format!("/api/openings/{id}"), Auth::Bearer(&device))
            .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);

    let unknown = get_page(&t.s, &format!("/o/{}", "A".repeat(43))).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert!(unknown.text().contains("<html"), "a page, not JSON");
    let gone = get_page(&t.s, &path_of(&deleted)).await;
    let short = get_page(&t.s, "/o/kurz").await;
    for r in [&gone, &short] {
        assert_eq!(r.status, StatusCode::NOT_FOUND);
        assert_eq!(
            r.body, unknown.body,
            "unknown, deleted and malformed look the same"
        );
    }

    // 20 misses per hour; then this address waits, even for a real link.
    for i in 3..20 {
        let r = get_page(&t.s, &format!("/o/{i:0>43}")).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND);
    }
    let r = get_page(&t.s, &path_of(&o)).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.header("retry-after"), Some("3600"));
    assert!(r.text().contains("Too many requests") || r.text().contains("Zu viele"));
    // Another address is not affected; behind the proxy the forwarded one
    // counts.
    let r = get_from(&t.s, &path_of(&o), "198.51.100.7:1234").await;
    assert_eq!(r.status, StatusCode::OK);
    let r =
        t.s.send(
            request(Method::GET, &path_of(&o), "10.0.0.2:5555")
                .header("x-forwarded-for", "203.0.113.5")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn page_views_are_rate_limited() {
    let s = TestServer::with_config(|c| {
        c.booking_limits.views_per_source = 3;
        c.booking_limits.posts_per_source = 2;
    })
    .await;
    let path = format!("/o/{}", "B".repeat(43));
    for _ in 0..3 {
        assert_eq!(get_page(&s, &path).await.status, StatusCode::NOT_FOUND);
    }
    let r = get_page(&s, &path).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.header("retry-after"), Some("600"));
    for _ in 0..2 {
        assert_eq!(
            post_form(&s, &path, &[]).await.status,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        post_form(&s, &path, &[]).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}
