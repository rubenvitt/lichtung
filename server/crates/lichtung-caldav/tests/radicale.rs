//! End-to-end test against a real local CalDAV server (Radicale).
//!
//! Ignored by default. Run with:
//!
//! ```sh
//! LICHTUNG_TEST_CALDAV_URL=http://127.0.0.1:5232/ \
//!   cargo test -p lichtung-caldav --test radicale -- --ignored
//! ```
//!
//! `LICHTUNG_TEST_CALDAV_USER` / `LICHTUNG_TEST_CALDAV_PASSWORD` default to
//! `lichtung` / empty (fine for Radicale with `[auth] type = none`).

use std::time::{SystemTime, UNIX_EPOCH};

use lichtung_caldav::{Client, Credentials, Error, Precondition};

fn ics(uid: &str, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Lichtung//Test//DE\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261023T080000Z\r\nDTSTART:20261024T090000Z\r\nDTEND:20261024T100000Z\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

#[tokio::test]
#[ignore = "needs a local CalDAV server, see LICHTUNG_TEST_CALDAV_URL"]
async fn radicale_round_trip() {
    let Ok(url) = std::env::var("LICHTUNG_TEST_CALDAV_URL") else {
        eprintln!("LICHTUNG_TEST_CALDAV_URL nicht gesetzt, Test übersprungen");
        return;
    };
    let user = std::env::var("LICHTUNG_TEST_CALDAV_USER").unwrap_or_else(|_| "lichtung".into());
    let password = std::env::var("LICHTUNG_TEST_CALDAV_PASSWORD").unwrap_or_default();
    let client = Client::new(url, Credentials::basic(user, password)).unwrap();

    // Discovery.
    let account = client.discover().await.expect("discovery");
    let home = account.calendar_home().expect("calendar home").clone();

    // Fresh calendar.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let cal = client
        .make_calendar(&format!("{home}lichtung-test-{nanos}/"), "Team-Sync")
        .await
        .expect("MKCALENDAR");
    let cal_s = cal.as_str();

    let calendars = client
        .list_calendars(home.as_str(), Some(&account.principal))
        .await
        .expect("list calendars");
    let info = calendars
        .iter()
        .find(|c| c.href == cal)
        .expect("new calendar listed");
    assert_eq!(info.display_name.as_deref(), Some("Team-Sync"));
    assert!(info.supports_events());
    assert!(info.writable);

    // Initial sync: empty.
    let s0 = client.sync_collection(cal_s, None).await.expect("sync 0");
    assert!(s0.changed.is_empty() && s0.deleted.is_empty());

    // Create.
    let uid = format!("{nanos}@example.org");
    let href = client.new_resource_href(cal_s, &uid).unwrap();
    let put1 = client
        .put(href.as_str(), &ics(&uid, "Planung"), Precondition::Create)
        .await
        .expect("PUT neu");
    let e1 = put1.etag.clone().expect("etag nach PUT");
    let err = client
        .put(href.as_str(), &ics(&uid, "doppelt"), Precondition::Create)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::PreconditionFailed), "{err:?}");

    let s1 = client
        .sync_collection(cal_s, Some(&s0.sync_token))
        .await
        .expect("sync 1");
    assert_eq!(s1.changed.len(), 1);
    assert_eq!(s1.changed[0].href, href);
    assert_eq!(s1.changed[0].etag.as_deref(), Some(e1.as_str()));

    // Fetch.
    let missing = format!("{cal}gibt-es-nicht.ics");
    let got = client
        .multiget(cal_s, &[href.as_str(), missing.as_str()])
        .await
        .expect("multiget");
    assert_eq!(got.found.len(), 1);
    assert!(got.found[0].data.contains(&format!("UID:{uid}")));
    assert_eq!(got.missing.len(), 1);
    let single = client.get(href.as_str()).await.expect("GET");
    assert_eq!(single.etag.as_deref(), Some(e1.as_str()));

    // Update, then a conflicting update with the stale ETag.
    let put2 = client
        .put(
            href.as_str(),
            &ics(&uid, "Planung (verschoben)"),
            Precondition::IfMatch(e1.clone()),
        )
        .await
        .expect("PUT ändern");
    let e2 = put2.etag.clone().expect("etag nach Änderung");
    assert_ne!(e1, e2);
    let err = client
        .put(
            href.as_str(),
            &ics(&uid, "veraltet"),
            Precondition::IfMatch(e1.clone()),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::PreconditionFailed), "{err:?}");

    let s2 = client
        .sync_collection(cal_s, Some(&s1.sync_token))
        .await
        .expect("sync 2");
    assert_eq!(s2.changed.len(), 1);
    assert_eq!(s2.changed[0].etag.as_deref(), Some(e2.as_str()));

    // ETag fallback.
    let etags = client.list_etags(cal_s).await.expect("etags");
    assert_eq!(etags.len(), 1);
    assert_eq!(etags[0].href, href);
    assert_eq!(etags[0].etag.as_deref(), Some(e2.as_str()));
    client
        .collection_state(cal_s)
        .await
        .expect("collection state");

    // Delete: stale ETag conflicts, current one works.
    let err = client.delete(href.as_str(), Some(&e1)).await.unwrap_err();
    assert!(matches!(err, Error::PreconditionFailed), "{err:?}");
    client
        .delete(href.as_str(), Some(&e2))
        .await
        .expect("DELETE");
    let s3 = client
        .sync_collection(cal_s, Some(&s2.sync_token))
        .await
        .expect("sync 3");
    assert!(s3.changed.is_empty());
    assert_eq!(s3.deleted, vec![href.clone()]);

    // Unknown token.
    let err = client
        .sync_collection(cal_s, Some("http://radicale.org/ns/sync/0000"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::SyncTokenInvalid), "{err:?}");

    // Clean up.
    client.delete(cal_s, None).await.expect("Kalender löschen");
}
