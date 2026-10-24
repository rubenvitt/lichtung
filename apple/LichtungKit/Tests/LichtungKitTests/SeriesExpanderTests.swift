import Foundation
import LichtungKit
import Testing

/// Hilfen für die Serientests. Alle Daten fest und erfunden, kein `Date()`.
private enum SX {
    static let zone = TimeZone(identifier: "Europe/Berlin")!
    static let newYork = TimeZone(identifier: "America/New_York")!
    static let utc = TimeZone(identifier: "UTC")!
    static let expander = SeriesExpander(deviceTimeZone: zone)

    static func day(_ y: Int, _ m: Int, _ d: Int) -> LocalDate { LocalDate(year: y, month: m, day: d) }

    static func local(_ y: Int, _ m: Int, _ d: Int, _ h: Int, _ min: Int = 0) -> LocalDateTime {
        LocalDateTime(date: day(y, m, d), hour: h, minute: min)
    }

    static func berlinValue(_ y: Int, _ m: Int, _ d: Int, _ h: Int, _ min: Int = 0) -> DateValue {
        .dateTime(local(y, m, d, h, min), timeZone: "Europe/Berlin")
    }

    static func instant(_ y: Int, _ m: Int, _ d: Int, _ h: Int = 0, _ min: Int = 0, in tz: TimeZone = zone) -> Date {
        local(y, m, d, h, min).instant(in: .lichtung(timeZone: tz))
    }

    /// Fenster von Tag zu Tag (Mitternacht in `tz`, Ende exklusiv).
    static func window(_ from: LocalDate, _ to: LocalDate, in tz: TimeZone = zone) -> DateInterval {
        let cal = Calendar.lichtung(timeZone: tz)
        return DateInterval(start: from.startDate(in: cal), end: to.startDate(in: cal))
    }

    static func event(
        start: DateValue, end: DateValue, rrule: String?, rdates: [DateValue] = [], exdates: [DateValue] = [],
        overrides: [EventOverride] = [], status: EventStatus = .confirmed, incomplete: Bool? = nil
    ) -> Event {
        Event(
            id: "ev_test", calendarId: "cal_test",
            details: EventDetails(title: "Probe", start: start, end: end, status: status),
            recurrence: rrule.map { Recurrence(rrule: $0, rdates: rdates, exdates: exdates, incomplete: incomplete) },
            overrides: overrides
        )
    }

    /// Wöchentlich/beliebig, 30 Minuten ab der Wanduhrzeit in Berlin.
    static func timed(_ rrule: String?, _ y: Int, _ m: Int, _ d: Int, _ h: Int, _ min: Int = 0, minutes: Int = 30,
                      rdates: [DateValue] = [], exdates: [DateValue] = [], overrides: [EventOverride] = []) -> Event {
        let s = local(y, m, d, h, min)
        let e = LocalDateTime(s.instant(in: .lichtung(timeZone: zone)).addingTimeInterval(Double(minutes) * 60), calendar: .lichtung(timeZone: zone))
        return event(start: .dateTime(s, timeZone: "Europe/Berlin"), end: .dateTime(e, timeZone: "Europe/Berlin"),
                     rrule: rrule, rdates: rdates, exdates: exdates, overrides: overrides)
    }

    /// Beginn jedes Vorkommens als Berliner Wanduhrzeit, `2026-10-26 09:00`.
    static func stamps(_ occ: [Occurrence], in tz: TimeZone = zone) -> [String] {
        let cal = Calendar.lichtung(timeZone: tz)
        return occ.map {
            let c = cal.dateComponents([.year, .month, .day, .hour, .minute], from: $0.start)
            return String(format: "%04d-%02d-%02d %02d:%02d", c.year!, c.month!, c.day!, c.hour!, c.minute!)
        }
    }

    /// Nur die Tage, `2026-10-26`.
    static func days(_ occ: [Occurrence], in tz: TimeZone = zone) -> [String] {
        stamps(occ, in: tz).map { String($0.prefix(10)) }
    }

    static func utcHour(_ d: Date) -> Int { Calendar.lichtung(timeZone: utc).component(.hour, from: d) }
}

@Suite("RRule")
struct RRuleParsingTests {
    @Test func liestAlleTeile() throws {
        let r = try #require(RRule("RRULE:FREQ=MONTHLY;INTERVAL=2;COUNT=10;BYDAY=2MO,-1FR,TU;BYMONTHDAY=-1,15;BYMONTH=1,6;BYSETPOS=-1;WKST=SU;X-FOO=bar"))
        #expect(r.frequency == .monthly)
        #expect(r.interval == 2)
        #expect(r.count == 10)
        #expect(r.byDay == [.init(.monday, ordinal: 2), .init(.friday, ordinal: -1), .init(.tuesday)])
        #expect(r.byMonthDay == [-1, 15])
        #expect(r.byMonth == [1, 6])
        #expect(r.bySetPos == [-1])
        #expect(r.weekStart == .sunday)
    }

    @Test func liestUntil() throws {
        #expect(RRule("FREQ=DAILY;UNTIL=20261231")?.until == .date(SX.day(2026, 12, 31)))
        #expect(RRule("FREQ=DAILY;UNTIL=20261231T225959")?.until == .local(SX.local(2026, 12, 31, 22, 59).withSecond(59)))
        let utc = try #require(RRule("FREQ=DAILY;UNTIL=20261231T225959Z")?.until)
        #expect(utc == .utc(SX.instant(2026, 12, 31, 22, 59, in: SX.utc).addingTimeInterval(59)))
    }

    @Test(arguments: [
        "", "FREQ=SOMETIMES", "BYDAY=MO", "FREQ=WEEKLY;INTERVAL=0", "FREQ=WEEKLY;BYDAY=XX",
        "FREQ=MONTHLY;BYMONTHDAY=32", "FREQ=DAILY;UNTIL=2026-12-31", "FREQ=DAILY;COUNT=drei", "FREQ=WEEKLY;BYDAY=0MO",
    ])
    func kaputteRegelErgibtNil(_ text: String) {
        #expect(RRule(text) == nil)
    }

    @Test func nichtUnterstuetzt() {
        #expect(RRule("FREQ=HOURLY")?.isExpandable == false)
        #expect(RRule("FREQ=YEARLY;BYWEEKNO=20")?.isExpandable == false)
        #expect(RRule("FREQ=yearly")?.isExpandable == true)
    }
}

private extension LocalDateTime {
    func withSecond(_ s: Int) -> LocalDateTime { LocalDateTime(date: date, hour: hour, minute: minute, second: s) }
}

/// Regel → erwartete Vorkommen (Berliner Wanduhrzeit) in einem Fenster.
struct SeriesCase: CustomTestStringConvertible, Sendable {
    let name: String
    let rrule: String
    /// DTSTART in Berlin.
    let start: LocalDateTime
    let window: (LocalDate, LocalDate)
    let expected: [String]

    var testDescription: String { name }
}

private let seriesTable: [SeriesCase] = [
    .init(name: "wöchentlich über Sommerzeit-Ende", rrule: "FREQ=WEEKLY;BYDAY=MO",
          start: SX.local(2026, 10, 19, 9), window: (SX.day(2026, 10, 19), SX.day(2026, 11, 3)),
          expected: ["2026-10-19 09:00", "2026-10-26 09:00", "2026-11-02 09:00"]),
    .init(name: "wöchentlich über Sommerzeit-Beginn", rrule: "FREQ=WEEKLY",
          start: SX.local(2027, 3, 22, 9), window: (SX.day(2027, 3, 20), SX.day(2027, 4, 6)),
          expected: ["2027-03-22 09:00", "2027-03-29 09:00", "2027-04-05 09:00"]),
    .init(name: "täglich mit COUNT", rrule: "FREQ=DAILY;COUNT=5",
          start: SX.local(2026, 11, 1, 10), window: (SX.day(2026, 10, 1), SX.day(2027, 1, 1)),
          expected: ["2026-11-01 10:00", "2026-11-02 10:00", "2026-11-03 10:00", "2026-11-04 10:00", "2026-11-05 10:00"]),
    .init(name: "COUNT, Fenster erst ab dem dritten", rrule: "FREQ=DAILY;COUNT=5",
          start: SX.local(2026, 11, 1, 10), window: (SX.day(2026, 11, 3), SX.day(2027, 1, 1)),
          expected: ["2026-11-03 10:00", "2026-11-04 10:00", "2026-11-05 10:00"]),
    .init(name: "UNTIL als Datum", rrule: "FREQ=DAILY;UNTIL=20261103",
          start: SX.local(2026, 11, 1, 10), window: (SX.day(2026, 10, 1), SX.day(2027, 1, 1)),
          expected: ["2026-11-01 10:00", "2026-11-02 10:00", "2026-11-03 10:00"]),
    .init(name: "UNTIL als UTC-Zeitpunkt (inklusive)", rrule: "FREQ=DAILY;UNTIL=20261103T090000Z",
          start: SX.local(2026, 11, 1, 10), window: (SX.day(2026, 10, 1), SX.day(2027, 1, 1)),
          expected: ["2026-11-01 10:00", "2026-11-02 10:00", "2026-11-03 10:00"]),
    .init(name: "UNTIL als UTC-Zeitpunkt knapp davor", rrule: "FREQ=DAILY;UNTIL=20261103T085959Z",
          start: SX.local(2026, 11, 1, 10), window: (SX.day(2026, 10, 1), SX.day(2027, 1, 1)),
          expected: ["2026-11-01 10:00", "2026-11-02 10:00"]),
    .init(name: "UNTIL floating", rrule: "FREQ=DAILY;UNTIL=20261102T100000",
          start: SX.local(2026, 11, 1, 10), window: (SX.day(2026, 10, 1), SX.day(2027, 1, 1)),
          expected: ["2026-11-01 10:00", "2026-11-02 10:00"]),
    .init(name: "monatlich am 31. (kürzere Monate fallen aus)", rrule: "FREQ=MONTHLY",
          start: SX.local(2026, 10, 31, 8), window: (SX.day(2026, 10, 1), SX.day(2027, 4, 1)),
          expected: ["2026-10-31 08:00", "2026-12-31 08:00", "2027-01-31 08:00", "2027-03-31 08:00"]),
    .init(name: "monatlich BYMONTHDAY=31", rrule: "FREQ=MONTHLY;BYMONTHDAY=31",
          start: SX.local(2026, 10, 31, 8), window: (SX.day(2026, 10, 1), SX.day(2027, 4, 1)),
          expected: ["2026-10-31 08:00", "2026-12-31 08:00", "2027-01-31 08:00", "2027-03-31 08:00"]),
    .init(name: "monatlich am Letzten (BYMONTHDAY=-1)", rrule: "FREQ=MONTHLY;BYMONTHDAY=-1",
          start: SX.local(2026, 10, 31, 8), window: (SX.day(2026, 10, 1), SX.day(2027, 3, 1)),
          expected: ["2026-10-31 08:00", "2026-11-30 08:00", "2026-12-31 08:00", "2027-01-31 08:00", "2027-02-28 08:00"]),
    .init(name: "monatlich letzter Freitag", rrule: "FREQ=MONTHLY;BYDAY=-1FR",
          start: SX.local(2026, 10, 30, 17), window: (SX.day(2026, 10, 1), SX.day(2027, 2, 1)),
          expected: ["2026-10-30 17:00", "2026-11-27 17:00", "2026-12-25 17:00", "2027-01-29 17:00"]),
    .init(name: "monatlich zweiter Montag", rrule: "FREQ=MONTHLY;BYDAY=2MO;COUNT=3",
          start: SX.local(2026, 11, 9, 18), window: (SX.day(2026, 10, 1), SX.day(2027, 6, 1)),
          expected: ["2026-11-09 18:00", "2026-12-14 18:00", "2027-01-11 18:00"]),
    .init(name: "Mo/Mi/Fr alle zwei Wochen", rrule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE,FR;WKST=MO",
          start: SX.local(2026, 11, 2, 7, 30), window: (SX.day(2026, 11, 1), SX.day(2026, 11, 22)),
          expected: ["2026-11-02 07:30", "2026-11-04 07:30", "2026-11-06 07:30",
                     "2026-11-16 07:30", "2026-11-18 07:30", "2026-11-20 07:30"]),
    .init(name: "WKST=MO (RFC-Beispiel)", rrule: "FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=MO",
          start: SX.local(2026, 11, 3, 9), window: (SX.day(2026, 11, 1), SX.day(2027, 1, 1)),
          expected: ["2026-11-03 09:00", "2026-11-08 09:00", "2026-11-17 09:00", "2026-11-22 09:00"]),
    .init(name: "WKST=SU (RFC-Beispiel)", rrule: "FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=SU",
          start: SX.local(2026, 11, 3, 9), window: (SX.day(2026, 11, 1), SX.day(2027, 1, 1)),
          expected: ["2026-11-03 09:00", "2026-11-15 09:00", "2026-11-17 09:00", "2026-11-29 09:00"]),
    .init(name: "jährlich am 29. Februar", rrule: "FREQ=YEARLY",
          start: SX.local(2028, 2, 29, 12), window: (SX.day(2028, 1, 1), SX.day(2037, 1, 1)),
          expected: ["2028-02-29 12:00", "2032-02-29 12:00", "2036-02-29 12:00"]),
    .init(name: "jährlich vierter Donnerstag im November", rrule: "FREQ=YEARLY;BYMONTH=11;BYDAY=4TH",
          start: SX.local(2026, 11, 26, 18), window: (SX.day(2026, 1, 1), SX.day(2029, 1, 1)),
          expected: ["2026-11-26 18:00", "2027-11-25 18:00", "2028-11-23 18:00"]),
    .init(name: "letzter Werktag im Monat", rrule: "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
          start: SX.local(2026, 10, 30, 16), window: (SX.day(2026, 10, 1), SX.day(2027, 3, 1)),
          expected: ["2026-10-30 16:00", "2026-11-30 16:00", "2026-12-31 16:00", "2027-01-29 16:00", "2027-02-26 16:00"]),
    .init(name: "DTSTART passt nicht zur Regel, zählt trotzdem", rrule: "FREQ=WEEKLY;BYDAY=FR;COUNT=3",
          start: SX.local(2026, 11, 3, 9), window: (SX.day(2026, 11, 1), SX.day(2027, 1, 1)),
          expected: ["2026-11-03 09:00", "2026-11-06 09:00", "2026-11-13 09:00"]),
    .init(name: "BYHOUR erweitert den Tag", rrule: "FREQ=DAILY;COUNT=4;BYHOUR=9,15",
          start: SX.local(2026, 11, 2, 9), window: (SX.day(2026, 11, 1), SX.day(2027, 1, 1)),
          expected: ["2026-11-02 09:00", "2026-11-02 15:00", "2026-11-03 09:00", "2026-11-03 15:00"]),
    .init(name: "Werktage täglich gefiltert", rrule: "FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR",
          start: SX.local(2026, 10, 23, 8), window: (SX.day(2026, 10, 23), SX.day(2026, 10, 28)),
          expected: ["2026-10-23 08:00", "2026-10-26 08:00", "2026-10-27 08:00"]),
]

@Suite("SeriesExpander")
struct SeriesExpanderTests {
    @Test(arguments: seriesTable)
    func expandiert(_ c: SeriesCase) {
        let ev = SX.timed(c.rrule, c.start.date.year, c.start.date.month, c.start.date.day, c.start.hour, c.start.minute)
        let occ = SX.expander.occurrences(of: ev, in: SX.window(c.window.0, c.window.1))
        #expect(SX.stamps(occ) == c.expected)
    }

    @Test("Sommerzeit: Wanduhrzeit bleibt, UTC verschiebt sich")
    func sommerzeit() {
        let ev = SX.timed("FREQ=WEEKLY;BYDAY=MO", 2026, 10, 19, 9)
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 10, 19), SX.day(2026, 10, 27)))
        #expect(occ.map { SX.utcHour($0.start) } == [7, 8])
        #expect(occ.map { $0.end.timeIntervalSince($0.start) } == [1800, 1800])
        let spring = SX.expander.occurrences(of: SX.timed("FREQ=WEEKLY", 2027, 3, 22, 9), in: SX.window(SX.day(2027, 3, 22), SX.day(2027, 3, 30)))
        #expect(spring.map { SX.utcHour($0.start) } == [8, 7])
    }

    @Test("Dauer über die Umstellung bleibt absolut")
    func dauerAbsolut() {
        // Sa 22:00 bis So 06:00 = 8 Stunden; in der Nacht der Umstellung endet es um 05:00.
        let ev = SX.timed("FREQ=WEEKLY", 2026, 10, 17, 22, minutes: 8 * 60)
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 10, 24), SX.day(2026, 10, 26)))
        #expect(occ.count == 1)
        #expect(occ[0].end.timeIntervalSince(occ[0].start) == 8 * 3600)
        #expect(occ[0].details.end == SX.berlinValue(2026, 10, 25, 5))
    }

    @Test func recurrenceIdHatFormVonDtstart() {
        let ev = SX.timed("FREQ=WEEKLY", 2026, 11, 2, 9)
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 9), SX.day(2026, 11, 10)))
        #expect(occ.count == 1)
        #expect(occ[0].recurrenceId == SX.berlinValue(2026, 11, 9, 9))
        #expect(occ[0].details.start == SX.berlinValue(2026, 11, 9, 9))
        #expect(occ[0].details.end == SX.berlinValue(2026, 11, 9, 9, 30))
        #expect(occ[0].isModified == false)
        #expect(occ[0].id == "ev_test/2026-11-09T09:00:00/Europe/Berlin")
    }

    @Test func exdate() {
        let ev = SX.timed("FREQ=WEEKLY", 2026, 11, 2, 9, exdates: [
            .dateTime(SX.local(2026, 11, 9, 9), timeZone: "Europe/Berlin", utc: SX.instant(2026, 11, 9, 9)),
            // dieselbe Serie, aber in UTC angegeben
            .dateTime(SX.local(2026, 11, 16, 8), timeZone: "UTC"),
            // passt auf keine Wanduhrzeit: bleibt wirkungslos
            .dateTime(SX.local(2026, 11, 23, 10), timeZone: "Europe/Berlin"),
        ])
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 1), SX.day(2026, 11, 30)))
        #expect(SX.days(occ) == ["2026-11-02", "2026-11-23"])
    }

    @Test("EXDATE als Datum nimmt den ganzen Tag weg, auch DTSTART")
    func exdateDatum() {
        let ev = SX.timed("FREQ=DAILY;COUNT=3", 2026, 11, 2, 9, exdates: [.date(SX.day(2026, 11, 2)), .date(SX.day(2026, 11, 4))])
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 1), SX.day(2026, 11, 30)))
        #expect(SX.days(occ) == ["2026-11-03"])
    }

    @Test func rdate() {
        let ev = SX.timed("FREQ=WEEKLY;COUNT=2", 2026, 11, 2, 9, minutes: 45, rdates: [
            SX.berlinValue(2026, 11, 4, 15),
            SX.berlinValue(2026, 11, 2, 9), // schon in der Regel: kein Duplikat
        ], exdates: [])
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 1), SX.day(2026, 11, 30)))
        #expect(SX.stamps(occ) == ["2026-11-02 09:00", "2026-11-04 15:00", "2026-11-09 09:00"])
        #expect(occ[1].end.timeIntervalSince(occ[1].start) == 45 * 60)
        #expect(occ[1].recurrenceId == SX.berlinValue(2026, 11, 4, 15))
    }

    private func moved(_ rid: DateValue, to start: DateValue, _ end: DateValue, status: EventStatus = .confirmed) -> EventOverride {
        EventOverride(recurrenceId: rid, details: EventDetails(title: "Probe (verschoben)", start: start, end: end, status: status))
    }

    @Test func overrideErsetztVorkommen() {
        let ev = SX.timed("FREQ=WEEKLY", 2026, 11, 2, 9, overrides: [
            moved(.dateTime(SX.local(2026, 11, 9, 9), timeZone: "Europe/Berlin", utc: SX.instant(2026, 11, 9, 9)),
                  to: SX.berlinValue(2026, 11, 9, 10), SX.berlinValue(2026, 11, 9, 11)),
        ])
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 1), SX.day(2026, 11, 17)))
        #expect(SX.stamps(occ) == ["2026-11-02 09:00", "2026-11-09 10:00", "2026-11-16 09:00"])
        #expect(occ[1].isModified)
        #expect(occ[1].title == "Probe (verschoben)")
        #expect(occ[1].recurrenceId == SX.berlinValue(2026, 11, 9, 9))
        #expect(occ[1].end.timeIntervalSince(occ[1].start) == 3600)
    }

    @Test("Override in das Fenster hinein und aus ihm heraus")
    func overrideVerschobenUeberFenstergrenze() {
        // Mo 16.11. 9:00 wird auf Sa 14.11. 11:00 vorgezogen.
        let ev = SX.timed("FREQ=WEEKLY", 2026, 11, 2, 9, overrides: [
            moved(SX.berlinValue(2026, 11, 16, 9), to: SX.berlinValue(2026, 11, 14, 11), SX.berlinValue(2026, 11, 14, 12)),
        ])
        let weekend = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 14), SX.day(2026, 11, 16)))
        #expect(SX.stamps(weekend) == ["2026-11-14 11:00"])
        #expect(weekend.first?.recurrenceId == SX.berlinValue(2026, 11, 16, 9))
        let monday = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 16), SX.day(2026, 11, 17)))
        #expect(monday.isEmpty)
    }

    @Test func overrideAbgesagt() {
        let ev = SX.timed("FREQ=WEEKLY", 2026, 11, 2, 9, overrides: [
            moved(SX.berlinValue(2026, 11, 9, 9), to: SX.berlinValue(2026, 11, 9, 9), SX.berlinValue(2026, 11, 9, 9, 30), status: .cancelled),
        ])
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 1), SX.day(2026, 11, 17)))
        #expect(SX.days(occ) == ["2026-11-02", "2026-11-16"])
    }

    @Test func ganztaegigeSerie() {
        let ev = SX.event(start: .date(SX.day(2026, 10, 24)), end: .date(SX.day(2026, 10, 25)), rrule: "FREQ=DAILY;COUNT=3")
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 10, 1), SX.day(2026, 11, 1)))
        #expect(SX.stamps(occ) == ["2026-10-24 00:00", "2026-10-25 00:00", "2026-10-26 00:00"])
        // Der Tag der Umstellung hat 25 Stunden, Ende ist exklusiv die nächste Mitternacht.
        #expect(occ.map { $0.end.timeIntervalSince($0.start) / 3600 } == [24, 25, 24])
        #expect(occ[1].recurrenceId == .date(SX.day(2026, 10, 25)))
        #expect(occ[1].details.start == .date(SX.day(2026, 10, 25)))
        #expect(occ[1].details.end == .date(SX.day(2026, 10, 26)))
        #expect(occ.allSatisfy { $0.isAllDay })
    }

    @Test("Ganztägig gilt in der Gerätezone")
    func ganztaegigInNewYork() {
        let ev = SX.event(start: .date(SX.day(2026, 11, 3)), end: .date(SX.day(2026, 11, 4)), rrule: "FREQ=WEEKLY;UNTIL=20261110")
        let ny = SeriesExpander(deviceTimeZone: SX.newYork)
        let occ = ny.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 1), SX.day(2026, 12, 1), in: SX.newYork))
        #expect(SX.stamps(occ, in: SX.newYork) == ["2026-11-03 00:00", "2026-11-10 00:00"])
    }

    @Test func floatingSerieInAndererGeraetezone() {
        let ev = SX.event(start: .dateTime(SX.local(2026, 11, 2, 8), timeZone: nil), end: .dateTime(SX.local(2026, 11, 2, 9), timeZone: nil),
                          rrule: "FREQ=DAILY;COUNT=2")
        let ny = SeriesExpander(deviceTimeZone: SX.newYork)
        let occ = ny.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 1), SX.day(2026, 11, 10), in: SX.newYork))
        #expect(SX.stamps(occ, in: SX.newYork) == ["2026-11-02 08:00", "2026-11-03 08:00"])
        #expect(occ[0].start == SX.instant(2026, 11, 2, 8, in: SX.newYork))
        #expect(occ[0].recurrenceId == .dateTime(SX.local(2026, 11, 2, 8), timeZone: nil))
    }

    @Test("Mehrtägiges Vorkommen ragt ins Fenster")
    func mehrtaegig() {
        // Fr 18:00 bis Mo 9:00, wöchentlich. Fenster: nur Sonntag.
        let ev = SX.timed("FREQ=WEEKLY", 2026, 10, 30, 18, minutes: (2 * 24 + 15) * 60)
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 8), SX.day(2026, 11, 9)))
        #expect(SX.stamps(occ) == ["2026-11-06 18:00"])
        let allDay = SX.event(start: .date(SX.day(2026, 11, 2)), end: .date(SX.day(2026, 11, 7)), rrule: "FREQ=MONTHLY")
        let occ2 = SX.expander.occurrences(of: allDay, in: SX.window(SX.day(2026, 12, 5), SX.day(2026, 12, 6)))
        #expect(SX.days(occ2) == ["2026-12-02"])
        #expect(SX.expander.occurrences(of: allDay, in: SX.window(SX.day(2026, 12, 7), SX.day(2026, 12, 8))).isEmpty)
    }

    @Test(arguments: ["FREQ=SOMETIMES", "FREQ=WEEKLY;INTERVAL=0", "", "BYDAY=MO", "FREQ=HOURLY;COUNT=5", "FREQ=YEARLY;BYWEEKNO=1"])
    func kaputteRegelLiefertNurDtstart(_ rrule: String) {
        let ev = SX.timed(rrule, 2026, 11, 2, 9, rdates: [SX.berlinValue(2026, 11, 5, 9)])
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 10, 1), SX.day(2027, 1, 1)))
        #expect(SX.stamps(occ) == ["2026-11-02 09:00", "2026-11-05 09:00"])
    }

    @Test func abgesagteSerieUndEinzeltermin() {
        let cancelled = SX.event(start: SX.berlinValue(2026, 11, 2, 9), end: SX.berlinValue(2026, 11, 2, 10), rrule: "FREQ=DAILY", status: .cancelled)
        #expect(SX.expander.occurrences(of: cancelled, in: SX.window(SX.day(2026, 11, 1), SX.day(2026, 11, 10))).isEmpty)

        let single = SX.timed(nil, 2026, 11, 2, 9)
        let occ = SX.expander.occurrences(of: single, in: SX.window(SX.day(2026, 11, 2), SX.day(2026, 11, 3)))
        #expect(occ.count == 1)
        #expect(occ[0].recurrenceId == nil)
        #expect(SX.expander.occurrences(of: single, in: SX.window(SX.day(2026, 11, 3), SX.day(2026, 11, 4))).isEmpty)
    }

    @Test("Termin ohne Dauer zählt nur mit Beginn im Fenster")
    func ohneDauer() {
        let ev = SX.timed("FREQ=DAILY", 2026, 11, 2, 0, minutes: 0)
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 11, 3), SX.day(2026, 11, 4)))
        #expect(SX.stamps(occ) == ["2026-11-03 00:00"])
    }

    @Test func incompleteAendertNichts() {
        let a = SX.event(start: SX.berlinValue(2026, 11, 2, 9), end: SX.berlinValue(2026, 11, 2, 10), rrule: "FREQ=WEEKLY;COUNT=3")
        let b = SX.event(start: SX.berlinValue(2026, 11, 2, 9), end: SX.berlinValue(2026, 11, 2, 10), rrule: "FREQ=WEEKLY;COUNT=3", incomplete: true)
        let w = SX.window(SX.day(2026, 11, 1), SX.day(2026, 12, 1))
        #expect(SX.expander.occurrences(of: a, in: w) == SX.expander.occurrences(of: b, in: w))
        #expect(SX.expander.occurrences(of: b, in: w).count == 3)
    }

    @Test("Lange tägliche Serie: kurzes Fenster weit hinten")
    func langeSerie() {
        let ev = SX.timed("FREQ=DAILY", 2026, 1, 1, 7)
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2035, 6, 4), SX.day(2035, 6, 11)))
        #expect(SX.days(occ) == ["2035-06-04", "2035-06-05", "2035-06-06", "2035-06-07", "2035-06-08", "2035-06-09", "2035-06-10"])
        let counted = SX.timed("FREQ=DAILY;COUNT=3650", 2026, 1, 1, 7)
        let last = SX.expander.occurrences(of: counted, in: SX.window(SX.day(2035, 12, 25), SX.day(2036, 1, 10)))
        #expect(SX.days(last).last == "2035-12-29") // 3650. Tag ab 1.1.2026
    }

    @Test("Regel ohne je ein weiteres Vorkommen hängt nicht")
    func unmoeglicheRegel() {
        let ev = SX.timed("FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30", 2026, 11, 2, 9)
        let occ = SX.expander.occurrences(of: ev, in: SX.window(SX.day(2026, 1, 1), SX.day(2100, 1, 1)))
        #expect(SX.days(occ) == ["2026-11-02"])
    }
}
