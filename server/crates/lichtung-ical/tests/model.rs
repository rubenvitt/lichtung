//! Lossless model: parse → serialize, typed getters and minimal setters.
//! All fixtures are invented.

mod common;

use lichtung_ical::{
    parse, split_by_uid, AlarmTrigger, CalTime, CalendarObject, Error, EventStatus, IcalDuration,
    PartStat, Transparency,
};

const RICH: &str = "BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//Lichtung Test//DE\r
CALSCALE:GREGORIAN\r
X-WR-CALNAME:Privat\r
BEGIN:VTIMEZONE\r
TZID:Europe/Berlin\r
X-LIC-LOCATION:Europe/Berlin\r
BEGIN:DAYLIGHT\r
TZOFFSETFROM:+0100\r
TZOFFSETTO:+0200\r
TZNAME:CEST\r
DTSTART:19700329T020000\r
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r
END:DAYLIGHT\r
BEGIN:STANDARD\r
TZOFFSETFROM:+0200\r
TZOFFSETTO:+0100\r
TZNAME:CET\r
DTSTART:19701025T030000\r
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r
END:STANDARD\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
UID:4f1c2d3e-0000-4000-8000-000000000001@example.org\r
DTSTAMP:20261001T080000Z\r
CREATED:20261001T080000Z\r
LAST-MODIFIED:20261001T080000Z\r
SEQUENCE:2\r
DTSTART;TZID=Europe/Berlin:20261026T140000\r
DTEND;TZID=Europe/Berlin:20261026T150000\r
SUMMARY:Zahnarzt\\, Kontrolle\\; danach Einkauf\r
LOCATION:Praxis am Beispielplatz 1\\, 12345 Musterstadt\r
DESCRIPTION:Bitte Versichertenkarte mitbringen. Danach noch zum Markt: Äpf\r
 el\\, Birnen und Käse kaufen.\\nZweite Zeile mit Umlauten: äöüß.\r
URL:https://example.org/termine/42\r
STATUS:CONFIRMED\r
TRANSP:OPAQUE\r
CATEGORIES:Gesundheit,Privat\r
ORGANIZER;CN=\"Muster, Erika\":mailto:erika@example.org\r
ATTENDEE;CN=Max Beispiel;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=T\r
 RUE:mailto:max@example.org\r
ATTENDEE;CN=\"Muster, Erika\";ROLE=CHAIR;PARTSTAT=ACCEPTED:mailto:erika@examp\r
 le.org\r
X-APPLE-TRAVEL-DURATION;VALUE=DURATION:PT30M\r
X-LICHTUNG-UNBEKANNT;X-PARAM=\"a:b;c\":irgendein Wert\r
BEGIN:VALARM\r
ACTION:DISPLAY\r
TRIGGER:-PT15M\r
DESCRIPTION:Erinnerung\r
X-WR-ALARMUID:alarm-1@example.org\r
END:VALARM\r
BEGIN:VALARM\r
ACTION:AUDIO\r
TRIGGER;RELATED=END:PT0S\r
END:VALARM\r
BEGIN:X-LICHTUNG-KOMPONENTE\r
X-FOO:bar\r
END:X-LICHTUNG-KOMPONENTE\r
END:VEVENT\r
BEGIN:VTODO\r
UID:todo@example.org\r
SUMMARY:Bleibt erhalten\r
END:VTODO\r
END:VCALENDAR\r
";

#[test]
fn roundtrip_is_byte_stable() {
    let obj = CalendarObject::parse(RICH).unwrap();
    assert_eq!(obj.to_ics(), RICH);
}

#[test]
fn unusual_folding_is_preserved() {
    let ics = common::cal(
        "BEGIN:VEVENT
         UID:fold@example.org
         DTSTAMP:20261001T080000Z
         DTSTART:20261026T140000Z
         END:VEVENT",
    )
    .replace(
        "UID:fold@example.org\r\n",
        "UID:fold@exa\r\n mple.org\r\nSUMMARY:kurz\r\n\tgefaltet\r\n",
    );
    let obj = CalendarObject::parse(&ics).unwrap();
    assert_eq!(obj.to_ics(), ics);
    let ev = obj.events().next().unwrap();
    assert_eq!(ev.uid().as_deref(), Some("fold@example.org"));
    assert_eq!(ev.summary().as_deref(), Some("kurzgefaltet"));
}

#[test]
fn lf_input_is_normalized_to_crlf() {
    let lf = RICH.replace("\r\n", "\n");
    let obj = CalendarObject::parse(&lf).unwrap();
    assert_eq!(obj.to_ics(), RICH);
}

#[test]
fn typed_getters() {
    let obj = CalendarObject::parse(RICH).unwrap();
    let ev = obj.master().unwrap();
    assert_eq!(
        ev.uid().as_deref(),
        Some("4f1c2d3e-0000-4000-8000-000000000001@example.org")
    );
    assert_eq!(
        ev.summary().as_deref(),
        Some("Zahnarzt, Kontrolle; danach Einkauf")
    );
    assert_eq!(
        ev.description().as_deref(),
        Some("Bitte Versichertenkarte mitbringen. Danach noch zum Markt: Äpfel, Birnen und Käse kaufen.\nZweite Zeile mit Umlauten: äöüß.")
    );
    assert_eq!(
        ev.location().as_deref(),
        Some("Praxis am Beispielplatz 1, 12345 Musterstadt")
    );
    assert_eq!(ev.url().as_deref(), Some("https://example.org/termine/42"));
    assert_eq!(ev.status(), Some(EventStatus::Confirmed));
    assert_eq!(ev.transparency(), Transparency::Opaque);
    assert_eq!(ev.sequence(), 2);
    assert_eq!(
        ev.start().unwrap(),
        CalTime::zoned(
            common::date("2026-10-26").and_hms_opt(14, 0, 0).unwrap(),
            "Europe/Berlin"
        )
    );
    assert!(!ev.is_all_day());
    assert!(!ev.is_recurring());

    let org = ev.organizer().unwrap();
    assert_eq!(org.cn.as_deref(), Some("Muster, Erika"));
    assert_eq!(org.address, "mailto:erika@example.org");

    let att = ev.attendees();
    assert_eq!(att.len(), 2);
    assert_eq!(att[0].email(), "max@example.org");
    assert_eq!(att[0].cn.as_deref(), Some("Max Beispiel"));
    assert_eq!(att[0].partstat, Some(PartStat::NeedsAction));
    assert_eq!(att[0].role.as_deref(), Some("REQ-PARTICIPANT"));
    assert_eq!(att[0].rsvp, Some(true));
    assert_eq!(att[1].email(), "erika@example.org");
    assert_eq!(att[1].partstat, Some(PartStat::Accepted));

    let alarms = ev.alarms();
    assert_eq!(alarms.len(), 2);
    assert_eq!(
        alarms[0].trigger,
        Some(AlarmTrigger::Relative {
            offset: IcalDuration::parse("-PT15M").unwrap(),
            related_to_end: false
        })
    );
    assert_eq!(
        alarms[1].trigger,
        Some(AlarmTrigger::Relative {
            offset: IcalDuration::default(),
            related_to_end: true
        })
    );
    assert_eq!(
        ev.travel_duration(),
        Some(IcalDuration::parse("PT30M").unwrap())
    );
    // Unknown things are still there.
    assert_eq!(
        ev.component()
            .property("X-LICHTUNG-UNBEKANNT")
            .unwrap()
            .param("X-PARAM"),
        Some("a:b;c")
    );
    assert_eq!(ev.component().components().len(), 3);
}

/// Lines that differ between two serializations (ignoring order-preserving
/// equal lines).
fn changed_lines(a: &str, b: &str) -> Vec<String> {
    let la: Vec<&str> = a.split("\r\n").collect();
    let lb: Vec<&str> = b.split("\r\n").collect();
    let mut out = Vec::new();
    for l in &lb {
        if !la.contains(l) {
            out.push(format!("+{l}"));
        }
    }
    for l in &la {
        if !lb.contains(l) {
            out.push(format!("-{l}"));
        }
    }
    out
}

#[test]
fn setters_touch_only_their_property() {
    let mut obj = CalendarObject::parse(RICH).unwrap();
    {
        let mut ev = obj.master_mut().unwrap();
        ev.set_summary(Some("Zahnarzt (verschoben)"));
    }
    assert_eq!(
        changed_lines(RICH, &obj.to_ics()),
        vec![
            "+SUMMARY:Zahnarzt (verschoben)".to_string(),
            "-SUMMARY:Zahnarzt\\, Kontrolle\\; danach Einkauf".to_string()
        ]
    );

    let mut obj = CalendarObject::parse(RICH).unwrap();
    {
        let mut ev = obj.master_mut().unwrap();
        assert!(ev.set_partstat("MAX@example.org", &PartStat::Accepted));
    }
    let diff = changed_lines(RICH, &obj.to_ics());
    let removed: Vec<&str> = diff.iter().filter_map(|l| l.strip_prefix('-')).collect();
    assert_eq!(
        removed,
        [
            "ATTENDEE;CN=Max Beispiel;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=T",
            " RUE:mailto:max@example.org"
        ]
    );
    let added: String = diff
        .iter()
        .filter_map(|l| l.strip_prefix('+'))
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .collect();
    assert_eq!(
        added,
        "ATTENDEE;CN=Max Beispiel;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED:mailto:max@example.org"
    );
    let ev = obj.master().unwrap();
    assert_eq!(ev.attendees()[0].partstat, Some(PartStat::Accepted));
    assert_eq!(ev.attendees()[0].rsvp, None);
    assert_eq!(ev.attendees()[1].partstat, Some(PartStat::Accepted));
    // Erika's line is untouched (still folded as before).
    assert!(obj
        .to_ics()
        .contains("PARTSTAT=ACCEPTED:mailto:erika@examp\r\n le.org\r\n"));

    // A no-op setter keeps the original bytes.
    let mut obj = CalendarObject::parse(RICH).unwrap();
    {
        let mut ev = obj.master_mut().unwrap();
        let d = ev.description();
        ev.set_description(d.as_deref());
        let s = ev.start().unwrap();
        ev.set_start(&s);
    }
    assert_eq!(obj.to_ics(), RICH);
}

#[test]
fn new_long_lines_are_folded_utf8_safe() {
    let mut obj = CalendarObject::parse(RICH).unwrap();
    let text = "Überlänge: ".to_string() + &"äöü€ ".repeat(40);
    obj.master_mut().unwrap().set_description(Some(&text));
    let out = obj.to_ics();
    for line in out.split("\r\n") {
        assert!(line.len() <= 75, "zu lang ({}): {line}", line.len());
    }
    let again = CalendarObject::parse(&out).unwrap();
    assert_eq!(
        again.master().unwrap().description().as_deref(),
        Some(text.as_str())
    );
    assert_eq!(again.to_ics(), out);
}

#[test]
fn parse_errors_are_reported() {
    let cases: &[(&str, &str)] = &[
        (
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x@example.org\r\nEND:VCALENDAR\r\n",
            "END:VCALENDAR passt nicht zu BEGIN:VEVENT",
        ),
        (
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x@example.org\r\nEND:VEVENT\r\n",
            "END:VCALENDAR fehlt",
        ),
        ("UID:x@example.org\r\n", "außerhalb einer Komponente"),
        (
            "BEGIN:VCALENDAR\r\nKEIN DOPPELPUNKT\r\nEND:VCALENDAR\r\n",
            "Zeile 2",
        ),
        ("", "keine Komponente"),
        (
            "BEGIN:VCALENDAR\r\nATTENDEE;CN=\"offen:mailto:a@example.org\r\nEND:VCALENDAR\r\n",
            "nicht geschlossen",
        ),
    ];
    for (input, needle) in cases {
        let err = parse(input).unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
        assert!(
            err.to_string().contains(needle),
            "{err} sollte „{needle}“ enthalten"
        );
    }
}

#[test]
fn split_feed_by_uid() {
    let feed = common::cal(&format!(
        "{}
         BEGIN:VEVENT
         UID:a@example.org
         DTSTAMP:20261001T080000Z
         DTSTART;TZID=W. Europe Standard Time:20261005T090000
         RRULE:FREQ=WEEKLY;COUNT=3
         SUMMARY:Team-Sync
         END:VEVENT
         BEGIN:VEVENT
         UID:b@example.org
         DTSTAMP:20261001T080000Z
         DTSTART:20261006T090000Z
         SUMMARY:Zahnarzt
         END:VEVENT
         BEGIN:VEVENT
         UID:a@example.org
         DTSTAMP:20261001T080000Z
         RECURRENCE-ID;TZID=W. Europe Standard Time:20261012T090000
         DTSTART;TZID=W. Europe Standard Time:20261012T100000
         SUMMARY:Team-Sync (später)
         END:VEVENT",
        common::VTZ_EU
    ));
    let objects = split_by_uid(&feed).unwrap();
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[0].uid().as_deref(), Some("a@example.org"));
    assert_eq!(objects[0].events().count(), 2);
    assert_eq!(
        objects[0].calendar().components_named("VTIMEZONE").count(),
        1
    );
    assert_eq!(objects[1].uid().as_deref(), Some("b@example.org"));
    assert_eq!(
        objects[1].calendar().components_named("VTIMEZONE").count(),
        0
    );
    assert_eq!(
        objects[1].calendar().property("PRODID").unwrap().value(),
        "-//Lichtung Test//DE"
    );
}
