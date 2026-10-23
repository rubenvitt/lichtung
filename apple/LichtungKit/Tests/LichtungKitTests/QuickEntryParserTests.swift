import Foundation
import LichtungKit
import Testing

/// Alle Fälle laufen gegen dasselbe feste „Jetzt“: Freitag, 23. Oktober 2026,
/// 10:00 Uhr in Berlin. Zwei Tage später endet die Sommerzeit (So 25.10.).
/// Termine, Namen und Orte sind erfunden.
let berlin = TimeZone(identifier: "Europe/Berlin")!
let fixedNow = berlinDate(2026, 10, 23, 10, 0)

func berlinDate(_ y: Int, _ m: Int, _ d: Int, _ h: Int = 0, _ min: Int = 0) -> Date {
    Calendar.lichtung(timeZone: berlin).date(from: DateComponents(year: y, month: m, day: d, hour: h, minute: min))!
}

let testCalendars = [
    CalendarChoice(id: "cal-arbeit", name: "Arbeit"),
    CalendarChoice(id: "cal-privat", name: "Privat"),
    CalendarChoice(id: "cal-paar", name: "Paar"),
]

struct ParserCase: CustomTestStringConvertible, Sendable {
    let input: String
    let title: String
    /// `2026-10-27 14:00–15:00`, `2026-11-03..2026-11-07` (ganztägig),
    /// `2026-10-23` (ein ganzer Tag) oder `–` (kein Tag).
    let when: String
    var rrule: String? = nil
    var location: String? = nil
    var calendarID: String? = nil

    var testDescription: String { input }
}

/// Kurzform eines Ergebnisses, damit die Tabelle lesbar bleibt.
func summary(_ e: QuickEntry) -> String {
    let cal = Calendar.lichtung(timeZone: berlin)
    func stamp(_ d: Date) -> (day: String, time: String) {
        let c = cal.dateComponents([.year, .month, .day, .hour, .minute], from: d)
        return (String(format: "%04d-%02d-%02d", c.year!, c.month!, c.day!), String(format: "%02d:%02d", c.hour!, c.minute!))
    }
    switch e.timing {
    case .unspecified:
        return "–"
    case let .allDay(first, last):
        return first == last ? "\(first)" : "\(first)..\(last)"
    case let .timed(start, end):
        let s = stamp(start), t = stamp(end)
        return s.day == t.day ? "\(s.day) \(s.time)–\(t.time)" : "\(s.day) \(s.time)–\(t.day) \(t.time)"
    }
}

let cases: [ParserCase] = [
    // Beispiele aus der Spec (§5)
    .init(input: "Zahnarzt Di 14 Uhr", title: "Zahnarzt", when: "2026-10-27 14:00–15:00"),
    .init(input: "Team-Sync morgen 9–9:30 in Raum 2", title: "Team-Sync", when: "2026-10-24 09:00–09:30", location: "Raum 2"),
    .init(input: "jeden Montag 7:30 Review", title: "Review", when: "2026-10-26 07:30–08:30", rrule: "FREQ=WEEKLY;BYDAY=MO"),
    .init(input: "Urlaub 3.–7.11.", title: "Urlaub", when: "2026-11-03..2026-11-07"),

    // Uhrzeit ohne Tag: heute, oder morgen, wenn vorbei
    .init(input: "Meeting 11 Uhr", title: "Meeting", when: "2026-10-23 11:00–12:00"),
    .init(input: "Meeting 9 Uhr", title: "Meeting", when: "2026-10-24 09:00–10:00"),
    .init(input: "Call 10:00", title: "Call", when: "2026-10-23 10:00–11:00"),
    .init(input: "um acht Frühstück", title: "Frühstück", when: "2026-10-24 08:00–09:00"),
    .init(input: "Check-in 14 Uhr", title: "Check-in", when: "2026-10-23 14:00–15:00"),

    // Tageszeiten
    .init(input: "Kino heute Abend", title: "Kino", when: "2026-10-23 19:00–20:00"),
    .init(input: "Essen Fr 8 abends", title: "Essen", when: "2026-10-23 20:00–21:00"),
    .init(input: "Frühstück morgen früh", title: "Frühstück", when: "2026-10-24 08:00–09:00"),
    .init(input: "Spieleabend Dienstagabend", title: "Spieleabend", when: "2026-10-27 19:00–20:00"),
    .init(input: "Lesung am Nachmittag", title: "Lesung", when: "2026-10-23 15:00–16:00"),

    // Gesprochene Uhrzeiten
    .init(input: "Telefonat um halb 3 für 30 min", title: "Telefonat", when: "2026-10-23 14:30–15:00"),
    .init(input: "Bus halb 8", title: "Bus", when: "2026-10-24 07:30–08:30"),
    .init(input: "viertel nach 9 Call", title: "Call", when: "2026-10-24 09:15–10:15"),

    // Wochentage
    .init(input: "Fr 9 Uhr Standup", title: "Standup", when: "2026-10-30 09:00–10:00"),
    .init(input: "Fr 16 Uhr Wochenrückblick", title: "Wochenrückblick", when: "2026-10-23 16:00–17:00"),
    .init(input: "nächsten Freitag Abgabe", title: "Abgabe", when: "2026-10-30"),
    .init(input: "Mo. 10 Uhr Arzt", title: "Arzt", when: "2026-10-26 10:00–11:00"),
    .init(input: "Termin bei Dr. Muster Mi 8:00", title: "Termin bei Dr. Muster", when: "2026-10-28 08:00–09:00"),
    .init(input: "Retro nächste Woche Mi 16 Uhr", title: "Retro", when: "2026-10-28 16:00–17:00"),
    .init(input: "Elternabend übernächste Woche Do 19:30", title: "Elternabend", when: "2026-11-05 19:30–20:30"),
    .init(input: "Planung nächste Woche", title: "Planung", when: "2026-10-26"),
    .init(input: "Kaffee mit Anna Sa 15 Uhr im Café Sonne", title: "Kaffee mit Anna", when: "2026-10-24 15:00–16:00", location: "Café Sonne"),
    .init(input: "so ein Tag", title: "so ein Tag", when: "–"),

    // Kalenderdaten
    .init(input: "Geburtstag Jona 3.1.", title: "Geburtstag Jona", when: "2027-01-03"),
    .init(input: "Steuer 31.10.2026", title: "Steuer", when: "2026-10-31"),
    .init(input: "Frist 15.11.26", title: "Frist", when: "2026-11-15"),
    .init(input: "Deadline 31.4.", title: "Deadline", when: "2027-04-30"),
    .init(input: "Treffen 14.11. 15:00-16:30", title: "Treffen", when: "2026-11-14 15:00–16:30"),
    .init(input: "Workshop 5. November 10 Uhr für 3 Stunden", title: "Workshop", when: "2026-11-05 10:00–13:00"),
    .init(input: "Abflug 2026-11-20 6:45", title: "Abflug", when: "2026-11-20 06:45–07:45"),
    .init(input: "Termin am 2.", title: "Termin", when: "2026-11-02"),
    .init(input: "Lesung 12. Dez 2027", title: "Lesung", when: "2027-12-12"),
    .init(input: "Di 3.11. Elternsprechtag", title: "Elternsprechtag", when: "2026-11-03"),

    // Relative Tage
    .init(input: "Arzt in 3 Tagen 8:15", title: "Arzt", when: "2026-10-26 08:15–09:15"),
    .init(input: "Tanken in 2 Wochen", title: "Tanken", when: "2026-11-06"),
    .init(input: "Nachsorge in einem Monat", title: "Nachsorge", when: "2026-11-23"),
    .init(input: "Bereitschaft ganztägig Do", title: "Bereitschaft", when: "2026-10-29"),
    .init(input: "Übermorgen Fahrrad holen", title: "Fahrrad holen", when: "2026-10-25"),

    // Spannen
    .init(input: "Konferenz 3. bis 5. Dezember", title: "Konferenz", when: "2026-12-03..2026-12-05"),
    .init(input: "Messe vom 10.11. bis 12.11.", title: "Messe", when: "2026-11-10..2026-11-12"),
    .init(input: "Ferien 28.12.-2.1.", title: "Ferien", when: "2026-12-28..2027-01-02"),
    .init(input: "Seminar 9.-11. Nov", title: "Seminar", when: "2026-11-09..2026-11-11"),
    .init(input: "Party 31.12. 20 Uhr bis 2 Uhr", title: "Party", when: "2026-12-31 20:00–2027-01-01 02:00"),
    .init(input: "Nachtschicht 22-6 Uhr", title: "Nachtschicht", when: "2026-10-23 22:00–2026-10-24 06:00"),
    .init(input: "Lesen 22-24 Uhr", title: "Lesen", when: "2026-10-23 22:00–2026-10-24 00:00"),
    .init(input: "Training von 18 bis 19:30", title: "Training", when: "2026-10-23 18:00–19:30"),
    .init(input: "Doppelstunde 8-10 abends", title: "Doppelstunde", when: "2026-10-23 20:00–22:00"),

    // Sommerzeit-Ende am 25.10.: Wanduhrzeiten bleiben stehen
    .init(input: "Nachtflug 25.10. 1:30-3:30", title: "Nachtflug", when: "2026-10-25 01:30–03:30"),

    // Dauer
    .init(input: "Mittagessen um 12:30 für 45 min", title: "Mittagessen", when: "2026-10-23 12:30–13:15"),
    .init(input: "Sport um 18 Uhr für 1,5 Std", title: "Sport", when: "2026-10-23 18:00–19:30"),
    .init(input: "Lernen 2h morgen 14:00", title: "Lernen", when: "2026-10-24 14:00–16:00"),
    .init(input: "Abstimmung 15 Uhr eine halbe Stunde", title: "Abstimmung", when: "2026-10-23 15:00–15:30"),
    .init(input: "Geschenk für Mila 17 Uhr", title: "Geschenk für Mila", when: "2026-10-23 17:00–18:00"),

    // Wiederholung
    .init(input: "Yoga montags und donnerstags 18 Uhr", title: "Yoga", when: "2026-10-26 18:00–19:00", rrule: "FREQ=WEEKLY;BYDAY=MO,TH"),
    .init(input: "Standup werktags 9:15", title: "Standup", when: "2026-10-26 09:15–10:15", rrule: "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
    .init(input: "Müll alle 2 Wochen Di", title: "Müll", when: "2026-10-27", rrule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU"),
    .init(input: "Miete monatlich am 1.", title: "Miete", when: "2026-11-01", rrule: "FREQ=MONTHLY"),
    .init(input: "Jahrestag 12.5. jährlich", title: "Jahrestag", when: "2027-05-12", rrule: "FREQ=YEARLY"),
    .init(input: "Laufen täglich 7 Uhr bis 31.10.", title: "Laufen", when: "2026-10-24 07:00–08:00", rrule: "FREQ=DAILY;UNTIL=20261031T225959Z"),
    .init(input: "Planung jeden Mo, Mi und Fr 8 Uhr", title: "Planung", when: "2026-10-26 08:00–09:00", rrule: "FREQ=WEEKLY;BYDAY=MO,WE,FR"),
    .init(input: "Wochenmarkt jede Woche am Samstag 9 Uhr", title: "Wochenmarkt", when: "2026-10-24 09:00–10:00", rrule: "FREQ=WEEKLY;BYDAY=SA"),
    .init(input: "Backup jeden 2. Tag", title: "Backup", when: "2026-10-23", rrule: "FREQ=DAILY;INTERVAL=2"),
    .init(input: "Putzen jede zweite Woche", title: "Putzen", when: "2026-10-23", rrule: "FREQ=WEEKLY;INTERVAL=2"),
    .init(input: "Pflanzen gießen alle drei Tage", title: "Pflanzen gießen", when: "2026-10-23", rrule: "FREQ=DAILY;INTERVAL=3"),
    .init(input: "Chor dienstags 19:30 bis zum 15.12.", title: "Chor", when: "2026-10-27 19:30–20:30", rrule: "FREQ=WEEKLY;BYDAY=TU;UNTIL=20261215T225959Z"),
    .init(input: "Sprechstunde jeden Werktag 11 Uhr", title: "Sprechstunde", when: "2026-10-23 11:00–12:00", rrule: "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),

    // Ort
    .init(input: "Treffen in Halle 3 um 14 Uhr", title: "Treffen", when: "2026-10-23 14:00–15:00", location: "Halle 3"),
    .init(input: "Mittagessen in der Kantine 12 Uhr", title: "Mittagessen", when: "2026-10-23 12:00–13:00", location: "Kantine"),
    .init(input: "Abstimmung @Büro Do 9:00", title: "Abstimmung", when: "2026-10-29 09:00–10:00", location: "Büro"),
    .init(input: "Probe in Raum 4 mit Band morgen", title: "Probe mit Band", when: "2026-10-24", location: "Raum 4"),

    // Kalender
    .init(input: "Präsentation /arbeit Do 10-11 Uhr", title: "Präsentation", when: "2026-10-29 10:00–11:00", calendarID: "cal-arbeit"),
    .init(input: "Zahnarzt Dienstag 14 Uhr /pri", title: "Zahnarzt", when: "2026-10-27 14:00–15:00", calendarID: "cal-privat"),
    .init(input: "Kino /pa Sa 20 Uhr", title: "Kino", when: "2026-10-24 20:00–21:00", calendarID: "cal-paar"),
    .init(input: "Kino /p Sa 20 Uhr", title: "Kino", when: "2026-10-24 20:00–21:00"),

    // Nichts zu erkennen
    .init(input: "Zahnarzt", title: "Zahnarzt", when: "–"),
    .init(input: "Kapitel 3-5 lesen", title: "Kapitel 3-5 lesen", when: "–"),
    .init(input: "Raum 2 buchen", title: "Raum 2 buchen", when: "–"),
    .init(input: "", title: "", when: "–"),
]

@Suite("QuickEntryParser")
struct QuickEntryParserTests {
    let parser = QuickEntryParser(timeZone: berlin, calendars: testCalendars)

    @Test(arguments: cases)
    func parses(_ c: ParserCase) {
        let e = parser.parse(c.input, now: fixedNow)
        #expect(e.title == c.title)
        #expect(summary(e) == c.when)
        #expect(e.recurrence?.rruleString(allDay: e.isAllDay, timeZone: berlin) == c.rrule)
        #expect(e.location == c.location)
        #expect(e.calendarID == c.calendarID)
    }

    @Test("Ein Wochentag ist heute nur, solange die Uhrzeit nicht vorbei ist")
    func weekdayLaterToday() {
        let tuesdayAfternoon = berlinDate(2026, 10, 27, 15, 0)
        #expect(summary(parser.parse("Zahnarzt Di 14 Uhr", now: tuesdayAfternoon)) == "2026-11-03 14:00–15:00")
        #expect(summary(parser.parse("Zahnarzt Di 16 Uhr", now: tuesdayAfternoon)) == "2026-10-27 16:00–17:00")
        #expect(summary(parser.parse("Zahnarzt Di", now: tuesdayAfternoon)) == "2026-10-27")
    }

    @Test("Die Standarddauer ist einstellbar")
    func defaultDuration() {
        let p = QuickEntryParser(timeZone: berlin, defaultDurationMinutes: 30)
        #expect(summary(p.parse("Call 15 Uhr", now: fixedNow)) == "2026-10-23 15:00–15:30")
    }

    @Test("Hervorhebungen zeigen auf die erkannten Wörter")
    func highlights() {
        let e = parser.parse("Team-Sync morgen 9–9:30 in Raum 2 /arbeit", now: fixedNow)
        #expect(e.highlights == [
            Highlight(kind: .date, range: 10..<16),
            Highlight(kind: .time, range: 17..<23),
            Highlight(kind: .location, range: 24..<33),
            Highlight(kind: .calendar, range: 34..<41),
        ])
    }

    @Test("Der Parser ist deterministisch")
    func deterministic() {
        for c in cases {
            #expect(parser.parse(c.input, now: fixedNow) == parser.parse(c.input, now: fixedNow))
        }
    }
}

@Suite("QuickEntryPreview")
struct QuickEntryPreviewTests {
    let parser = QuickEntryParser(timeZone: berlin, calendars: testCalendars)

    func preview(_ text: String) -> QuickEntryPreview {
        QuickEntryPreview(parser.parse(text, now: fixedNow), now: fixedNow, timeZone: berlin, calendars: testCalendars)
    }

    @Test func timed() {
        let p = preview("Zahnarzt Di 14 Uhr /privat")
        #expect(p.title == "Zahnarzt")
        #expect(p.when == "Di, 27. Okt. · 14:00–15:00")
        #expect(p.calendarName == "Privat")
        #expect(p.recurrence == nil)
    }

    @Test func todayAndTomorrow() {
        #expect(preview("Kino heute Abend").when == "Heute, Fr, 23. Okt. · 19:00–20:00")
        #expect(preview("Team-Sync morgen 9–9:30").when == "Morgen, Sa, 24. Okt. · 09:00–09:30")
    }

    @Test func allDayRange() {
        #expect(preview("Urlaub 3.–7.11.").when == "Di, 3. Nov. – Sa, 7. Nov. · ganztägig")
        #expect(preview("Geburtstag Jona 3.1.").when == "So, 3. Jan. 2027 · ganztägig")
    }

    @Test func overnight() {
        #expect(preview("Nachtschicht 22-6 Uhr").when == "Heute, Fr, 23. Okt. 22:00 – Morgen, Sa, 24. Okt. 06:00")
        #expect(preview("Lesen 22-24 Uhr").when == "Heute, Fr, 23. Okt. · 22:00–00:00")
    }

    @Test func recurrence() {
        #expect(preview("jeden Montag 7:30 Review").recurrence == "jeden Montag")
        #expect(preview("Yoga montags und donnerstags 18 Uhr").recurrence == "jeden Montag und Donnerstag")
        #expect(preview("Standup werktags 9:15").recurrence == "werktags")
        #expect(preview("Müll alle 2 Wochen Di").recurrence == "alle 2 Wochen am Dienstag")
        #expect(preview("Laufen täglich 7 Uhr bis 31.10.").recurrence == "täglich bis 31. Okt. 2026")
    }

    @Test func unknownCalendarAndEmptyTitle() {
        let p = preview("/urlaub morgen")
        #expect(p.title == "Neuer Termin")
        #expect(p.calendarName == "„urlaub“ (unbekannt)")
        #expect(preview("Zahnarzt").when == "Tag offen")
    }
}
