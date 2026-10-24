//! Table tests: input ICS → expected occurrences. All fixtures are invented.

mod common;

use common::{berlin, cal, run, utc, VTZ_EU};
use lichtung_ical::{expand, CalendarObject, ExpandOptions};

struct Case {
    name: &'static str,
    body: String,
    from: &'static str,
    to: &'static str,
    opts: ExpandOptions,
    expected: &'static [&'static str],
}

fn case(
    name: &'static str,
    body: &str,
    from: &'static str,
    to: &'static str,
    expected: &'static [&'static str],
) -> Case {
    Case {
        name,
        body: body.to_string(),
        from,
        to,
        opts: berlin(),
        expected,
    }
}

fn with_opts(mut c: Case, opts: ExpandOptions) -> Case {
    c.opts = opts;
    c
}

fn cases() -> Vec<Case> {
    let new_york = ExpandOptions::new(chrono_tz::America::New_York);
    let mut with_cancelled = berlin();
    with_cancelled.include_cancelled = true;

    vec![
        // --- Frequencies --------------------------------------------------
        case(
            "täglich mit COUNT",
            "BEGIN:VEVENT
             UID:daily-count@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261012T080000
             DTEND;TZID=Europe/Berlin:20261012T081500
             RRULE:FREQ=DAILY;COUNT=3
             SUMMARY:Stand-up
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &[
                "2026-10-12 08:00 Europe/Berlin [06:00Z] 15min",
                "2026-10-13 08:00 Europe/Berlin [06:00Z] 15min",
                "2026-10-14 08:00 Europe/Berlin [06:00Z] 15min",
            ],
        ),
        case(
            "täglich mit INTERVAL und UNTIL (inklusiv, über die Umstellung)",
            "BEGIN:VEVENT
             UID:daily-until@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261020T180000
             DTEND;TZID=Europe/Berlin:20261020T190000
             RRULE:FREQ=DAILY;INTERVAL=2;UNTIL=20261026T170000Z
             SUMMARY:Laufen
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &[
                "2026-10-20 18:00 Europe/Berlin [16:00Z] 60min",
                "2026-10-22 18:00 Europe/Berlin [16:00Z] 60min",
                "2026-10-24 18:00 Europe/Berlin [16:00Z] 60min",
                "2026-10-26 18:00 Europe/Berlin [17:00Z] 60min",
            ],
        ),
        case(
            "wöchentlich über die Sommerzeit-Umstellung im März (Berlin)",
            "BEGIN:VEVENT
             UID:weekly-march@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20260316T090000
             DTEND;TZID=Europe/Berlin:20260316T093000
             RRULE:FREQ=WEEKLY;COUNT=3
             SUMMARY:Team-Sync
             END:VEVENT",
            "2026-03-01 00:00",
            "2026-04-30 00:00",
            &[
                "2026-03-16 09:00 Europe/Berlin [08:00Z] 30min",
                "2026-03-23 09:00 Europe/Berlin [08:00Z] 30min",
                "2026-03-30 09:00 Europe/Berlin [07:00Z] 30min",
            ],
        ),
        case(
            "wöchentlich über die Winterzeit-Umstellung im Oktober (Berlin)",
            "BEGIN:VEVENT
             UID:weekly-october@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261019T090000
             DTEND;TZID=Europe/Berlin:20261019T093000
             RRULE:FREQ=WEEKLY
             SUMMARY:Team-Sync
             END:VEVENT",
            "2026-10-15 00:00",
            "2026-11-03 00:00",
            &[
                "2026-10-19 09:00 Europe/Berlin [07:00Z] 30min",
                "2026-10-26 09:00 Europe/Berlin [08:00Z] 30min",
                "2026-11-02 09:00 Europe/Berlin [08:00Z] 30min",
            ],
        ),
        case(
            "wöchentlich über die Umstellung im März (New York)",
            "BEGIN:VEVENT
             UID:weekly-ny-march@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=America/New_York:20260302T090000
             DTEND;TZID=America/New_York:20260302T100000
             RRULE:FREQ=WEEKLY;COUNT=3
             SUMMARY:Team-Sync
             END:VEVENT",
            "2026-03-01 00:00",
            "2026-04-01 00:00",
            &[
                "2026-03-02 09:00 America/New_York [14:00Z] 60min",
                "2026-03-09 09:00 America/New_York [13:00Z] 60min",
                "2026-03-16 09:00 America/New_York [13:00Z] 60min",
            ],
        ),
        case(
            "wöchentlich über die Umstellung im November (New York)",
            "BEGIN:VEVENT
             UID:weekly-ny-november@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=America/New_York:20261026T090000
             DTEND;TZID=America/New_York:20261026T100000
             RRULE:FREQ=WEEKLY;COUNT=2
             SUMMARY:Team-Sync
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-12-01 00:00",
            &[
                "2026-10-26 09:00 America/New_York [13:00Z] 60min",
                "2026-11-02 09:00 America/New_York [14:00Z] 60min",
            ],
        ),
        case(
            "monatlich am zweiten Dienstag (BYDAY=2TU)",
            "BEGIN:VEVENT
             UID:monthly-2tu@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261013T100000
             DTEND;TZID=Europe/Berlin:20261013T110000
             RRULE:FREQ=MONTHLY;BYDAY=2TU;COUNT=4
             SUMMARY:Monatsrunde
             END:VEVENT",
            "2026-10-01 00:00",
            "2027-03-01 00:00",
            &[
                "2026-10-13 10:00 Europe/Berlin [08:00Z] 60min",
                "2026-11-10 10:00 Europe/Berlin [09:00Z] 60min",
                "2026-12-08 10:00 Europe/Berlin [09:00Z] 60min",
                "2027-01-12 10:00 Europe/Berlin [09:00Z] 60min",
            ],
        ),
        case(
            "monatlich am Monatsletzten (BYMONTHDAY=-1)",
            "BEGIN:VEVENT
             UID:monthly-last@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20260131T170000
             DTEND;TZID=Europe/Berlin:20260131T173000
             RRULE:FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=4
             SUMMARY:Monatsabschluss
             END:VEVENT",
            "2026-01-01 00:00",
            "2026-12-31 00:00",
            &[
                "2026-01-31 17:00 Europe/Berlin [16:00Z] 30min",
                "2026-02-28 17:00 Europe/Berlin [16:00Z] 30min",
                "2026-03-31 17:00 Europe/Berlin [15:00Z] 30min",
                "2026-04-30 17:00 Europe/Berlin [15:00Z] 30min",
            ],
        ),
        case(
            "jährlich ganztägig",
            "BEGIN:VEVENT
             UID:yearly@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;VALUE=DATE:20261024
             DTEND;VALUE=DATE:20261025
             RRULE:FREQ=YEARLY
             SUMMARY:Jahrestag
             END:VEVENT",
            "2026-01-01 00:00",
            "2029-01-01 00:00",
            &[
                "2026-10-24..2026-10-25 ganztägig",
                "2027-10-24..2027-10-25 ganztägig",
                "2028-10-24..2028-10-25 ganztägig",
            ],
        ),
        case(
            "jährlich am 29. Februar nur in Schaltjahren",
            "BEGIN:VEVENT
             UID:leap@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;VALUE=DATE:20240229
             RRULE:FREQ=YEARLY
             SUMMARY:Schalttag
             END:VEVENT",
            "2024-01-01 00:00",
            "2033-01-01 00:00",
            &[
                "2024-02-29..2024-03-01 ganztägig",
                "2028-02-29..2028-03-01 ganztägig",
                "2032-02-29..2032-03-01 ganztägig",
            ],
        ),
        // --- COUNT / UNTIL --------------------------------------------------
        case(
            "UNTIL als DATE bei DTSTART mit TZID",
            "BEGIN:VEVENT
             UID:until-date@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261026T090000
             DTEND;TZID=Europe/Berlin:20261026T091500
             RRULE:FREQ=DAILY;UNTIL=20261028
             SUMMARY:Stand-up
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-30 00:00",
            &[
                "2026-10-26 09:00 Europe/Berlin [08:00Z] 15min",
                "2026-10-27 09:00 Europe/Berlin [08:00Z] 15min",
                "2026-10-28 09:00 Europe/Berlin [08:00Z] 15min",
            ],
        ),
        case(
            "UNTIL knapp vor dem nächsten Vorkommen (UTC)",
            "BEGIN:VEVENT
             UID:until-utc@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261026T090000
             DTEND;TZID=Europe/Berlin:20261026T091500
             RRULE:FREQ=DAILY;UNTIL=20261027T075959Z
             SUMMARY:Stand-up
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-30 00:00",
            &["2026-10-26 09:00 Europe/Berlin [08:00Z] 15min"],
        ),
        case(
            "ganztägige Serie mit UNTIL",
            "BEGIN:VEVENT
             UID:allday-until@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;VALUE=DATE:20261026
             DTEND;VALUE=DATE:20261027
             RRULE:FREQ=DAILY;UNTIL=20261028
             SUMMARY:Messe
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-30 00:00",
            &[
                "2026-10-26..2026-10-27 ganztägig",
                "2026-10-27..2026-10-28 ganztägig",
                "2026-10-28..2026-10-29 ganztägig",
            ],
        ),
        // --- EXDATE / RDATE -------------------------------------------------
        case(
            "EXDATE in eigener Zone und in UTC",
            "BEGIN:VEVENT
             UID:exdate@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261005T090000
             DTEND;TZID=Europe/Berlin:20261005T093000
             RRULE:FREQ=WEEKLY;COUNT=4
             EXDATE;TZID=Europe/Berlin:20261012T090000
             EXDATE:20261026T080000Z
             SUMMARY:Team-Sync
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-30 00:00",
            &[
                "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min",
                "2026-10-19 09:00 Europe/Berlin [07:00Z] 30min",
            ],
        ),
        case(
            "EXDATE als DATE und in fremder Zone",
            "BEGIN:VEVENT
             UID:exdate-date@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261005T090000
             DTEND;TZID=Europe/Berlin:20261005T093000
             RRULE:FREQ=WEEKLY;COUNT=4
             EXDATE;VALUE=DATE:20261012
             EXDATE;TZID=America/New_York:20261019T030000
             SUMMARY:Team-Sync
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-30 00:00",
            &[
                "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min",
                "2026-10-26 09:00 Europe/Berlin [08:00Z] 30min",
            ],
        ),
        case(
            "RDATE als Zeitpunkt und als PERIOD",
            "BEGIN:VEVENT
             UID:rdate@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261005T090000
             DTEND;TZID=Europe/Berlin:20261005T093000
             RRULE:FREQ=DAILY;COUNT=2
             RDATE;TZID=Europe/Berlin:20261010T140000
             RDATE;VALUE=PERIOD:20261011T100000Z/PT2H
             SUMMARY:Sprechstunde
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-30 00:00",
            &[
                "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min",
                "2026-10-06 09:00 Europe/Berlin [07:00Z] 30min",
                "2026-10-10 14:00 Europe/Berlin [12:00Z] 30min",
                "2026-10-11 12:00 Europe/Berlin [10:00Z] 120min",
            ],
        ),
        case(
            "nur RDATE ohne RRULE",
            "BEGIN:VEVENT
             UID:rdate-only@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261005T090000
             DTEND;TZID=Europe/Berlin:20261005T100000
             RDATE;TZID=Europe/Berlin:20261007T090000,20261009T090000
             SUMMARY:Zahnarzt
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-30 00:00",
            &[
                "2026-10-05 09:00 Europe/Berlin [07:00Z] 60min",
                "2026-10-07 09:00 Europe/Berlin [07:00Z] 60min",
                "2026-10-09 09:00 Europe/Berlin [07:00Z] 60min",
            ],
        ),
        // --- Overrides ------------------------------------------------------
        case(
            "Override aus dem Fenster heraus- und hineingeschoben (Fenster Anfang Oktober)",
            OVERRIDES,
            "2026-10-01 00:00",
            "2026-10-10 00:00",
            &[
                "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min",
                "2026-10-08 14:00 Europe/Berlin [12:00Z] 30min override",
            ],
        ),
        case(
            "Override: ursprüngliche Vorkommen im Fenster sind beide verschoben",
            OVERRIDES,
            "2026-10-11 00:00",
            "2026-10-21 00:00",
            &[],
        ),
        case(
            "Override: nach hinten verschobenes Vorkommen erscheint am neuen Ort",
            OVERRIDES,
            "2026-10-29 00:00",
            "2026-10-31 00:00",
            &["2026-10-30 09:00 Europe/Berlin [08:00Z] 30min override"],
        ),
        case(
            "abgesagtes Override fällt weg",
            CANCELLED,
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &[
                "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min",
                "2026-10-19 09:00 Europe/Berlin [07:00Z] 30min",
            ],
        ),
        with_opts(
            case(
                "abgesagtes Override wird markiert (include_cancelled)",
                CANCELLED,
                "2026-10-01 00:00",
                "2026-11-01 00:00",
                &[
                    "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min",
                    "2026-10-12 09:00 Europe/Berlin [07:00Z] 30min override abgesagt",
                    "2026-10-19 09:00 Europe/Berlin [07:00Z] 30min",
                ],
            ),
            with_cancelled,
        ),
        case(
            "RANGE=THISANDFUTURE verschiebt die folgenden Vorkommen",
            "BEGIN:VEVENT
             UID:taf@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261005T090000
             DTEND;TZID=Europe/Berlin:20261005T093000
             RRULE:FREQ=WEEKLY;COUNT=4
             SUMMARY:Team-Sync
             END:VEVENT
             BEGIN:VEVENT
             UID:taf@example.org
             DTSTAMP:20261001T080000Z
             RECURRENCE-ID;RANGE=THISANDFUTURE;TZID=Europe/Berlin:20261019T090000
             DTSTART;TZID=Europe/Berlin:20261019T100000
             DTEND;TZID=Europe/Berlin:20261019T110000
             SUMMARY:Team-Sync (neu)
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &[
                "2026-10-05 09:00 Europe/Berlin [07:00Z] 30min",
                "2026-10-12 09:00 Europe/Berlin [07:00Z] 30min",
                "2026-10-19 10:00 Europe/Berlin [08:00Z] 60min override",
                "2026-10-26 10:00 Europe/Berlin [09:00Z] 60min override",
            ],
        ),
        // --- DST edge cases ---------------------------------------------------
        case(
            "Termin über die Umstellung im März (Wanduhr 01:00–04:00 = 2 h)",
            "BEGIN:VEVENT
             UID:night-march@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20260329T010000
             DTEND;TZID=Europe/Berlin:20260329T040000
             SUMMARY:Nachtschicht
             END:VEVENT",
            "2026-03-28 00:00",
            "2026-03-30 00:00",
            &["2026-03-29 01:00 Europe/Berlin [00:00Z] 120min"],
        ),
        case(
            "Termin über die Umstellung im Oktober (Wanduhr 01:00–04:00 = 4 h)",
            "BEGIN:VEVENT
             UID:night-october@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261025T010000
             DTEND;TZID=Europe/Berlin:20261025T040000
             SUMMARY:Nachtschicht
             END:VEVENT",
            "2026-10-24 00:00",
            "2026-10-26 00:00",
            &["2026-10-25 01:00 Europe/Berlin [23:00Z] 240min"],
        ),
        case(
            "nicht existierende Zeit (Lücke) wird nach vorn verschoben",
            "BEGIN:VEVENT
             UID:gap@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20260328T023000
             DTEND;TZID=Europe/Berlin:20260328T033000
             RRULE:FREQ=DAILY;COUNT=3
             SUMMARY:Backup
             END:VEVENT",
            "2026-03-27 00:00",
            "2026-04-01 00:00",
            &[
                "2026-03-28 02:30 Europe/Berlin [01:30Z] 60min",
                "2026-03-29 03:30 Europe/Berlin [01:30Z] 60min",
                "2026-03-30 02:30 Europe/Berlin [00:30Z] 60min",
            ],
        ),
        case(
            "doppelte Zeit (Überlappung) nimmt die erste Instanz",
            "BEGIN:VEVENT
             UID:overlap@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261024T023000
             DTEND;TZID=Europe/Berlin:20261024T033000
             RRULE:FREQ=DAILY;COUNT=3
             SUMMARY:Backup
             END:VEVENT",
            "2026-10-23 00:00",
            "2026-10-28 00:00",
            &[
                "2026-10-24 02:30 Europe/Berlin [00:30Z] 60min",
                "2026-10-25 02:30 Europe/Berlin [00:30Z] 120min",
                "2026-10-26 02:30 Europe/Berlin [01:30Z] 60min",
            ],
        ),
        case(
            "DURATION in Tagen ist nominal (23 h am Umstellungstag)",
            "BEGIN:VEVENT
             UID:duration-nominal@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20260328T120000
             DURATION:P1D
             SUMMARY:Ausflug
             END:VEVENT",
            "2026-03-27 00:00",
            "2026-04-01 00:00",
            &["2026-03-28 12:00 Europe/Berlin [11:00Z] 1380min"],
        ),
        case(
            "DURATION in Stunden ist exakt",
            "BEGIN:VEVENT
             UID:duration-exact@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20260328T120000
             DURATION:PT24H
             SUMMARY:Ausflug
             END:VEVENT",
            "2026-03-27 00:00",
            "2026-04-01 00:00",
            &["2026-03-28 12:00 Europe/Berlin [11:00Z] 1440min"],
        ),
        // --- All-day ----------------------------------------------------------
        case(
            "ganztägig eintägig",
            "BEGIN:VEVENT
             UID:allday-one@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;VALUE=DATE:20261024
             DTEND;VALUE=DATE:20261025
             SUMMARY:Umzug
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &["2026-10-24..2026-10-25 ganztägig"],
        ),
        case(
            "ganztägig mehrtägig, Fenster mitten drin",
            "BEGIN:VEVENT
             UID:allday-multi@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;VALUE=DATE:20261024
             DTEND;VALUE=DATE:20261027
             SUMMARY:Urlaub
             END:VEVENT",
            "2026-10-25 12:00",
            "2026-10-25 13:00",
            &["2026-10-24..2026-10-27 ganztägig"],
        ),
        case(
            "ganztägig ohne DTEND dauert einen Tag",
            "BEGIN:VEVENT
             UID:allday-implicit@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;VALUE=DATE:20261024
             SUMMARY:Feiertag
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &["2026-10-24..2026-10-25 ganztägig"],
        ),
        case(
            "ganztägige Serie",
            "BEGIN:VEVENT
             UID:allday-series@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;VALUE=DATE:20261024
             DTEND;VALUE=DATE:20261025
             RRULE:FREQ=WEEKLY;COUNT=3
             SUMMARY:Markttag
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-12-01 00:00",
            &[
                "2026-10-24..2026-10-25 ganztägig",
                "2026-10-31..2026-11-01 ganztägig",
                "2026-11-07..2026-11-08 ganztägig",
            ],
        ),
        // --- Zones ------------------------------------------------------------
        case(
            "floating in der Standardzone Berlin",
            "BEGIN:VEVENT
             UID:floating@example.org
             DTSTAMP:20261001T080000Z
             DTSTART:20261024T100000
             DTEND:20261024T110000
             SUMMARY:Lesen
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &["2026-10-24 10:00 Europe/Berlin [08:00Z] 60min"],
        ),
        with_opts(
            case(
                "floating in der Standardzone New York",
                "BEGIN:VEVENT
                 UID:floating-ny@example.org
                 DTSTAMP:20261001T080000Z
                 DTSTART:20261024T100000
                 DTEND:20261024T110000
                 SUMMARY:Lesen
                 END:VEVENT",
                "2026-10-01 00:00",
                "2026-11-01 00:00",
                &["2026-10-24 10:00 America/New_York [14:00Z] 60min"],
            ),
            new_york,
        ),
        case(
            "UTC",
            "BEGIN:VEVENT
             UID:utc@example.org
             DTSTAMP:20261001T080000Z
             DTSTART:20261024T100000Z
             DTEND:20261024T103000Z
             SUMMARY:Telefonat
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &["2026-10-24 10:00 UTC [10:00Z] 30min"],
        ),
        case(
            "Windows-Zonenname (Outlook)",
            &format!(
                "{VTZ_EU}
                 BEGIN:VEVENT
                 UID:windows@example.org
                 DTSTAMP:20261001T080000Z
                 DTSTART;TZID=W. Europe Standard Time:20261019T090000
                 DTEND;TZID=W. Europe Standard Time:20261019T093000
                 RRULE:FREQ=WEEKLY;COUNT=2
                 SUMMARY:Abteilungsrunde
                 END:VEVENT"
            ),
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &[
                "2026-10-19 09:00 Europe/Berlin [07:00Z] 30min",
                "2026-10-26 09:00 Europe/Berlin [08:00Z] 30min",
            ],
        ),
        case(
            "Mozilla-Präfix vor der IANA-Zone",
            "BEGIN:VEVENT
             UID:mozilla@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=/mozilla.org/20050126_1/Europe/Berlin:20261026T090000
             DTEND;TZID=/mozilla.org/20050126_1/Europe/Berlin:20261026T100000
             SUMMARY:Zahnarzt
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &["2026-10-26 09:00 Europe/Berlin [08:00Z] 60min"],
        ),
        case(
            "unbekannte TZID mit eigener VTIMEZONE (Nordhalbkugel)",
            &format!(
                "{}
                 BEGIN:VEVENT
                 UID:custom-north@example.org
                 DTSTAMP:20261001T080000Z
                 DTSTART;TZID=Lichtung Mitteleuropa:20260323T090000
                 DTEND;TZID=Lichtung Mitteleuropa:20260323T093000
                 RRULE:FREQ=WEEKLY;COUNT=2
                 SUMMARY:Team-Sync
                 END:VEVENT",
                VTZ_EU.replace("W. Europe Standard Time", "Lichtung Mitteleuropa")
            ),
            "2026-03-01 00:00",
            "2026-04-30 00:00",
            &[
                "2026-03-23 09:00 Lichtung Mitteleuropa [08:00Z] 30min",
                "2026-03-30 09:00 Lichtung Mitteleuropa [07:00Z] 30min",
            ],
        ),
        case(
            "unbekannte TZID mit eigener VTIMEZONE (Südhalbkugel)",
            "BEGIN:VTIMEZONE
             TZID:Lichtung Süd
             BEGIN:STANDARD
             DTSTART:20000402T030000
             TZOFFSETFROM:+1100
             TZOFFSETTO:+1000
             RRULE:FREQ=YEARLY;BYMONTH=4;BYDAY=1SU
             END:STANDARD
             BEGIN:DAYLIGHT
             DTSTART:20001001T020000
             TZOFFSETFROM:+1000
             TZOFFSETTO:+1100
             RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=1SU
             END:DAYLIGHT
             END:VTIMEZONE
             BEGIN:VEVENT
             UID:custom-south@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Lichtung Süd:20260330T090000
             DTEND;TZID=Lichtung Süd:20260330T093000
             RRULE:FREQ=WEEKLY;COUNT=2
             SUMMARY:Team-Sync
             END:VEVENT",
            "2026-03-01 00:00",
            "2026-04-30 00:00",
            &[
                "2026-03-30 09:00 Lichtung Süd [22:00Z] 30min",
                "2026-04-06 09:00 Lichtung Süd [23:00Z] 30min",
            ],
        ),
        case(
            "unbekannte TZID ohne VTIMEZONE fällt auf die Standardzone zurück",
            "BEGIN:VEVENT
             UID:unknown-tz@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Lichtung Nirgendwo:20261026T090000
             DTEND;TZID=Lichtung Nirgendwo:20261026T100000
             SUMMARY:Zahnarzt
             END:VEVENT",
            "2026-10-01 00:00",
            "2026-11-01 00:00",
            &["2026-10-26 09:00 Europe/Berlin [08:00Z] 60min"],
        ),
        // --- Window ------------------------------------------------------------
        case(
            "laufender Termin am Fensteranfang zählt mit",
            "BEGIN:VEVENT
             UID:running@example.org
             DTSTAMP:20261001T080000Z
             DTSTART;TZID=Europe/Berlin:20261026T090000
             DTEND;TZID=Europe/Berlin:20261026T110000
             SUMMARY:Workshop
             END:VEVENT",
            "2026-10-26 09:00",
            "2026-10-26 09:30",
            &["2026-10-26 09:00 Europe/Berlin [08:00Z] 120min"],
        ),
        case(
            "Fensterende ist exklusiv",
            "BEGIN:VEVENT
             UID:exclusive@example.org
             DTSTAMP:20261001T080000Z
             DTSTART:20261026T100000Z
             DTEND:20261026T110000Z
             SUMMARY:Workshop
             END:VEVENT",
            "2026-10-26 09:00",
            "2026-10-26 10:00",
            &[],
        ),
    ]
}

const OVERRIDES: &str = "BEGIN:VEVENT
    UID:overrides@example.org
    DTSTAMP:20261001T080000Z
    DTSTART;TZID=Europe/Berlin:20261005T090000
    DTEND;TZID=Europe/Berlin:20261005T093000
    RRULE:FREQ=WEEKLY;COUNT=3
    SUMMARY:Team-Sync
    END:VEVENT
    BEGIN:VEVENT
    UID:overrides@example.org
    DTSTAMP:20261001T080000Z
    RECURRENCE-ID;TZID=Europe/Berlin:20261012T090000
    DTSTART;TZID=Europe/Berlin:20261030T090000
    DTEND;TZID=Europe/Berlin:20261030T093000
    SUMMARY:Team-Sync (verschoben)
    END:VEVENT
    BEGIN:VEVENT
    UID:overrides@example.org
    DTSTAMP:20261001T080000Z
    RECURRENCE-ID;TZID=Europe/Berlin:20261019T090000
    DTSTART;TZID=Europe/Berlin:20261008T140000
    DTEND;TZID=Europe/Berlin:20261008T143000
    SUMMARY:Team-Sync (vorgezogen)
    END:VEVENT";

const CANCELLED: &str = "BEGIN:VEVENT
    UID:cancelled@example.org
    DTSTAMP:20261001T080000Z
    DTSTART;TZID=Europe/Berlin:20261005T090000
    DTEND;TZID=Europe/Berlin:20261005T093000
    RRULE:FREQ=WEEKLY;COUNT=3
    SUMMARY:Team-Sync
    END:VEVENT
    BEGIN:VEVENT
    UID:cancelled@example.org
    DTSTAMP:20261001T080000Z
    RECURRENCE-ID;TZID=Europe/Berlin:20261012T090000
    DTSTART;TZID=Europe/Berlin:20261012T090000
    DTEND;TZID=Europe/Berlin:20261012T093000
    STATUS:CANCELLED
    SUMMARY:Team-Sync
    END:VEVENT";

#[test]
fn expansion_table() {
    let mut failures = Vec::new();
    for c in cases() {
        let ics = cal(&c.body);
        let got = run(&ics, c.from, c.to, &c.opts);
        if got != c.expected {
            failures.push(format!(
                "## {}\n   erwartet: {:#?}\n   erhalten: {:#?}",
                c.name, c.expected, got
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn table_has_enough_cases() {
    assert!(cases().len() >= 40);
}

#[test]
fn endless_rule_is_capped() {
    let ics = cal("BEGIN:VEVENT
         UID:minutely@example.org
         DTSTAMP:20261001T080000Z
         DTSTART:20261001T000000Z
         RRULE:FREQ=MINUTELY
         SUMMARY:Ticker
         END:VEVENT");
    let obj = CalendarObject::parse(&ics).unwrap();
    let mut opts = berlin();
    opts.max_occurrences = 50;
    let res = expand(
        &obj,
        utc("2026-10-01 00:00"),
        utc("2027-10-01 00:00"),
        &opts,
    )
    .unwrap();
    assert_eq!(res.occurrences.len(), 50);
    assert!(res.truncated);
    assert_eq!(res.occurrences[49].start, utc("2026-10-01 00:49"));
}

#[test]
fn never_matching_rule_does_not_hang() {
    let ics = cal("BEGIN:VEVENT
         UID:never@example.org
         DTSTAMP:20261001T080000Z
         DTSTART;VALUE=DATE:20260101
         RRULE:FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30
         SUMMARY:Nie
         END:VEVENT");
    let obj = CalendarObject::parse(&ics).unwrap();
    let res = expand(
        &obj,
        utc("2025-01-01 00:00"),
        utc("2040-01-01 00:00"),
        &berlin(),
    )
    .unwrap();
    // DTSTART is always an instance.
    assert_eq!(res.occurrences.len(), 1);
}

#[test]
fn invalid_rule_yields_dtstart_and_warning() {
    let ics = cal("BEGIN:VEVENT
         UID:broken@example.org
         DTSTAMP:20261001T080000Z
         DTSTART:20261001T090000Z
         RRULE:FREQ=WEEKLY;BYDAY=XX
         SUMMARY:Kaputt
         END:VEVENT");
    let obj = CalendarObject::parse(&ics).unwrap();
    let res = expand(
        &obj,
        utc("2026-01-01 00:00"),
        utc("2027-01-01 00:00"),
        &berlin(),
    )
    .unwrap();
    assert_eq!(res.occurrences.len(), 1);
    assert!(
        res.warnings.iter().any(|w| w.contains("RRULE")),
        "{:?}",
        res.warnings
    );
}

#[test]
fn unknown_zone_is_reported() {
    let ics = cal("BEGIN:VEVENT
         UID:unknown-tz@example.org
         DTSTAMP:20261001T080000Z
         DTSTART;TZID=Lichtung Nirgendwo:20261026T090000
         SUMMARY:Zahnarzt
         END:VEVENT");
    let obj = CalendarObject::parse(&ics).unwrap();
    let res = expand(
        &obj,
        utc("2026-01-01 00:00"),
        utc("2027-01-01 00:00"),
        &berlin(),
    )
    .unwrap();
    assert!(res
        .warnings
        .iter()
        .any(|w| w.contains("Lichtung Nirgendwo")));
}

#[test]
fn occurrence_metadata() {
    let ics = cal(OVERRIDES);
    let obj = CalendarObject::parse(&ics).unwrap();
    let res = expand(
        &obj,
        utc("2026-10-01 00:00"),
        utc("2026-11-01 00:00"),
        &berlin(),
    )
    .unwrap();
    let moved = res
        .occurrences
        .iter()
        .find(|o| o.is_override && o.local_start.format("%d").to_string() == "30")
        .unwrap();
    assert_eq!(moved.recurrence_key.as_deref(), Some("20261012T070000Z"));
    assert_eq!(moved.uid.as_deref(), Some("overrides@example.org"));
    let ev = obj.event_at(moved.component_index).unwrap();
    assert_eq!(ev.summary().as_deref(), Some("Team-Sync (verschoben)"));
    let first = &res.occurrences[0];
    assert!(!first.is_override);
    assert_eq!(
        obj.event_at(first.component_index)
            .unwrap()
            .summary()
            .as_deref(),
        Some("Team-Sync")
    );
}
