import Foundation
@testable import LichtungKit
import Testing

/// Vorkommen für Layout-Tests, Oktober/November 2026 in Berlin (erfunden).
enum LayoutFixtures {
    static let tz = TimeZone(identifier: "Europe/Berlin")!
    static let cal = Calendar.lichtung(timeZone: tz)

    static func day(_ d: Int, month: Int = 10) -> LocalDate { LocalDate(year: 2026, month: month, day: d) }

    static func timed(_ id: String, _ d: Int, _ h: Int, _ m: Int = 0, to eh: Int, _ em: Int = 0, endDay: Int? = nil, month: Int = 10, calendar: String = "cal_1") -> Occurrence {
        let s = DateValue.dateTime(LocalDateTime(date: day(d, month: month), hour: h, minute: m), timeZone: "Europe/Berlin")
        let e = DateValue.dateTime(LocalDateTime(date: day(endDay ?? d, month: month), hour: eh, minute: em), timeZone: "Europe/Berlin")
        let details = EventDetails(title: id, start: s, end: e)
        return Occurrence(eventId: id, calendarId: calendar, recurrenceId: nil, details: details,
                          start: s.instant(deviceTimeZone: tz), end: e.instant(deviceTimeZone: tz), isModified: false)
    }

    static func allDay(_ id: String, _ d: Int, through last: Int? = nil, month: Int = 10, calendar: String = "cal_1") -> Occurrence {
        let s = DateValue.date(day(d, month: month))
        let e = DateValue.date(day(last ?? d, month: month).adding(days: 1))
        let details = EventDetails(title: id, start: s, end: e)
        return Occurrence(eventId: id, calendarId: calendar, recurrenceId: nil, details: details,
                          start: s.instant(deviceTimeZone: tz), end: e.instant(deviceTimeZone: tz), isModified: false)
    }
}

@Suite("Ansicht A: Liste und Tagesleiste")
struct AgendaTests {
    typealias L = LayoutFixtures
    let today = LayoutFixtures.day(23)

    @Test func groupsByDayAllDayFirst() {
        let occ = [
            L.timed("Sport", 23, 11, to: 12), L.timed("Team-Sync", 23, 9, to: 9, 30), L.allDay("Bereitschaft", 24),
            L.timed("Workshop", 24, 10, to: 11),
        ]
        let days = AgendaBuilder.days(occ, from: today, days: 4, today: today, calendar: L.cal)
        #expect(days.map(\.heading) == ["Heute · Fr 23.", "Morgen · Sa 24."])
        #expect(days[0].items.map(\.occurrence.title) == ["Team-Sync", "Sport"])
        #expect(days[0].items.map(\.timeText) == ["09:00–09:30", "11:00–12:00"])
        #expect(days[1].items.map(\.timeText) == ["ganztägig", "10:00–11:00"])
    }

    @Test func todayStaysWhenEmpty() {
        let days = AgendaBuilder.days([L.timed("Zahnarzt", 27, 14, to: 15)], from: today, days: 7, today: today, calendar: L.cal)
        #expect(days.map(\.heading) == ["Heute · Fr 23.", "Di 27. Okt."])
        #expect(days[0].items.isEmpty)
    }

    @Test func multiDayTimedSplitsAcrossDays() {
        let shift = L.timed("Nachtschicht", 23, 22, to: 6, endDay: 24)
        let trip = L.timed("Reise", 23, 18, to: 12, endDay: 25)
        let days = AgendaBuilder.days([shift, trip], from: today, days: 3, today: today, calendar: L.cal)
        #expect(days[0].items.map(\.timeText) == ["ab 18:00", "ab 22:00"])
        #expect(days[1].items.map(\.timeText) == ["ganztägig", "bis 06:00"])
        #expect(days[1].items.map(\.span) == [.middle, .last])
        #expect(days[2].items.map(\.timeText) == ["bis 12:00"])
    }

    @Test func multiDayAllDayAppearsEachDay() {
        let days = AgendaBuilder.days([L.allDay("Urlaub", 3, through: 7, month: 11)], from: L.day(1, month: 11), days: 10,
                                      today: today, calendar: L.cal)
        #expect(days.map(\.date.day) == [3, 4, 5, 6, 7])
        #expect(days.first?.heading == "Di 3. Nov.")
    }

    @Test func headingOtherYearAndYesterday() {
        #expect(AgendaBuilder.heading(LocalDate(year: 2027, month: 1, day: 3), today: today) == "So 3. Jan. 2027")
        #expect(AgendaBuilder.heading(L.day(22), today: today) == "Gestern · Do 22.")
    }

    @Test func tickerShowsCalendarsAndBusyShare() {
        let occ = [
            L.timed("A", 23, 8, to: 11, calendar: "arbeit"), L.timed("B", 23, 10, to: 14, calendar: "arbeit"),
            L.timed("C", 23, 18, to: 19, calendar: "sport"), L.allDay("D", 24, calendar: "privat"),
        ]
        let t = DayTicker.days(occ, from: L.day(19), count: 7, today: today, calendar: L.cal)
        #expect(t.map(\.weekdayShort) == ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"])
        #expect(t[4].isToday)
        #expect(t[4].calendarIds == ["arbeit", "sport"])
        // 8–14 und 18–19 Uhr von 12 Stunden: 7/12.
        #expect(abs(t[4].busyFraction - 7.0 / 12.0) < 0.0001)
        #expect(t[5].calendarIds == ["privat"])
        #expect(t[5].busyFraction == 0)
        #expect(t[5].isWeekend && t[6].isWeekend && !t[4].isWeekend)
    }
}

@Suite("Ansicht B/C: Zeitraster")
struct TimelineLayoutTests {
    typealias L = LayoutFixtures

    @Test func singleEventPosition() {
        let b = TimelineLayout.blocks([L.timed("Sync", 23, 9, to: 9, 30)], day: L.day(23), calendar: L.cal)
        #expect(b.count == 1)
        #expect(b[0].top == 9.0 / 24.0)
        #expect(b[0].height == 0.5 / 24.0)
        #expect(b[0].columns == 1 && b[0].span == 1)
    }

    @Test func overlappingGetColumns() {
        let occ = [L.timed("A", 23, 9, to: 11), L.timed("B", 23, 10, to: 12), L.timed("C", 23, 11, to: 13), L.timed("D", 23, 14, to: 15)]
        let b = Dictionary(uniqueKeysWithValues: TimelineLayout.blocks(occ, day: L.day(23), calendar: L.cal).map { ($0.occurrence.title, $0) })
        #expect(b["A"]?.column == 0)
        #expect(b["B"]?.column == 1)
        // C beginnt, wenn A endet: Spalte 0 ist wieder frei.
        #expect(b["C"]?.column == 0)
        #expect(b["A"]?.columns == 2 && b["C"]?.columns == 2)
        #expect(b["D"]?.columns == 1)
    }

    @Test func blockSpansFreeColumnsToTheRight() {
        // A lang, B und C kurz nacheinander rechts daneben, D unter B frei.
        let occ = [L.timed("A", 23, 9, to: 12), L.timed("B", 23, 9, to: 10), L.timed("C", 23, 9, 30, to: 10, 30), L.timed("D", 23, 11, to: 11, 30)]
        let b = Dictionary(uniqueKeysWithValues: TimelineLayout.blocks(occ, day: L.day(23), calendar: L.cal).map { ($0.occurrence.title, $0) })
        #expect(b["A"]?.columns == 3)
        #expect(b["D"]?.column == 1)
        #expect(b["D"]?.span == 2)
        #expect(b["B"]?.span == 1)
    }

    @Test func shortEventsGetMinimumHeight() {
        let occ = [L.timed("A", 23, 9, to: 9, 5), L.timed("B", 23, 9, 10, to: 9, 15)]
        let b = TimelineLayout.blocks(occ, day: L.day(23), calendar: L.cal, minimumMinutes: 15)
        // Mit 15 min Mindesthöhe überschneiden sie sich und stehen nebeneinander.
        #expect(b.map(\.columns) == [2, 2])
        #expect(abs(b[0].height - 15.0 / 1440.0) < 1e-9)
    }

    @Test func clipsAtDayBordersAndAllDayIsSkipped() {
        let occ = [L.timed("Nacht", 23, 22, to: 6, endDay: 24), L.allDay("Frei", 24)]
        let b = TimelineLayout.blocks(occ, day: L.day(24), calendar: L.cal)
        #expect(b.count == 1)
        #expect(b[0].top == 0)
        #expect(b[0].continuesBefore && !b[0].continuesAfter)
        // Der 24.10. ist ein normaler Tag mit 24 Stunden.
        #expect(abs(b[0].height - 6.0 / 24.0) < 1e-9)
    }

    @Test func dstDayHas25Hours() {
        // Am 25.10.2026 endet die Sommerzeit: 12:00 liegt 13 echte Stunden nach Mitternacht.
        let b = TimelineLayout.blocks([L.timed("Mittag", 25, 12, to: 13)], day: L.day(25), calendar: L.cal)
        #expect(abs(b[0].top - 13.0 / 25.0) < 1e-9)
        #expect(abs(b[0].height - 1.0 / 25.0) < 1e-9)
    }

    @Test func nowLine() {
        let now = L.day(23).date(hour: 12, minute: 0, in: L.cal)
        #expect(TimelineLayout.nowLine(now, day: L.day(23), calendar: L.cal) == 0.5)
        #expect(TimelineLayout.nowLine(now, day: L.day(24), calendar: L.cal) == nil)
    }
}

@Suite("Bahnen und Monatsraster")
struct LaneAndMonthTests {
    typealias L = LayoutFixtures
    let week = (19...25).map { LayoutFixtures.day($0) }

    @Test func allDayLanesAcrossWeek() {
        let occ = [L.allDay("Urlaub", 21, through: 27), L.allDay("Messe", 19, through: 20), L.allDay("Feiertag", 21), L.allDay("Alt", 15, through: 19)]
        let bars = LaneLayout.bars(occ, days: week, calendar: L.cal)
        let byTitle = Dictionary(uniqueKeysWithValues: bars.map { ($0.occurrence.title, $0) })
        #expect(byTitle["Alt"].map { "\($0.firstColumn)-\($0.lastColumn) \($0.continuesBefore)" } == "0-0 true")
        #expect(byTitle["Messe"].map { "\($0.firstColumn)-\($0.lastColumn)" } == "0-1")
        #expect(byTitle["Urlaub"].map { "\($0.firstColumn)-\($0.lastColumn) \($0.continuesAfter)" } == "2-6 true")
        // Gleicher Beginn: die längere Messe oben. Urlaub nimmt die frei gewordene obere Bahn.
        #expect(byTitle["Messe"]?.lane == 0)
        #expect(byTitle["Alt"]?.lane == 1)
        #expect(byTitle["Urlaub"]?.lane == 0)
        #expect(byTitle["Feiertag"]?.lane == 1)
    }

    @Test func timedEventEndingAtMidnightStaysOnOneDay() {
        let bars = LaneLayout.bars([L.timed("Lesen", 23, 22, to: 0, endDay: 24)], days: week, calendar: L.cal)
        #expect(bars.map { [$0.firstColumn, $0.lastColumn] } == [[4, 4]])
    }

    @Test func overflowCounts() {
        let occ = (0..<4).map { L.timed("T\($0)", 23, 9 + $0, to: 10 + $0) }
        let bars = LaneLayout.bars(occ, days: week, calendar: L.cal)
        #expect(LaneLayout.overflow(bars, columns: 7, visibleLanes: 2) == [0, 0, 0, 0, 2, 0, 0])
    }

    @Test func monthWeeksStartMonday() {
        let weeks = MonthGrid.weeks(year: 2026, month: 10)
        #expect(weeks.count == 5)
        #expect(weeks[0][0] == LocalDate(year: 2026, month: 9, day: 28))
        #expect(weeks[4][6] == LocalDate(year: 2026, month: 11, day: 1))
        // Februar 2027 beginnt an einem Montag und hat genau 4 Wochen.
        #expect(MonthGrid.weeks(year: 2027, month: 2).count == 4)
        #expect(MonthGrid.weeks(year: 2027, month: 2, fixedRows: true).count == 6)
        #expect(MonthGrid.title(year: 2026, month: 10) == "Oktober 2026")
    }

    @Test func isoWeeks() {
        #expect(MonthGrid.isoWeek(LocalDate(year: 2026, month: 10, day: 23)) == 43)
        #expect(MonthGrid.isoWeek(LocalDate(year: 2027, month: 1, day: 1)) == 53)
        #expect(MonthGrid.isoWeek(LocalDate(year: 2026, month: 1, day: 1)) == 1)
        #expect(MonthGrid.isoWeek(LocalDate(year: 2024, month: 12, day: 30)) == 1)
    }

    @Test func countsPerDay() {
        let days = (1...7).map { L.day($0, month: 11) }
        let c = MonthGrid.counts([L.allDay("Urlaub", 3, through: 5, month: 11), L.timed("Arzt", 3, 9, to: 10, month: 11)], days: days, calendar: L.cal)
        #expect(c[L.day(3, month: 11)] == 2)
        #expect(c[L.day(5, month: 11)] == 1)
        #expect(c[L.day(6, month: 11)] == nil)
    }
}
