import Foundation
@testable import LichtungKit
import Testing

@Suite("Tagesband: Index und Ausschnitt (LIC-67)")
struct DayTickerIndexTests {
    typealias L = LayoutFixtures
    let today = LayoutFixtures.day(23)

    @Test func epochAndKnownDays() {
        #expect(DayTicker.index(of: LocalDate(year: 1970, month: 1, day: 1)) == 0)
        #expect(DayTicker.index(of: LocalDate(year: 1969, month: 12, day: 31)) == -1)
        #expect(DayTicker.index(of: LocalDate(year: 2000, month: 3, day: 1)) == 11017)
        #expect(DayTicker.day(at: 0) == LocalDate(year: 1970, month: 1, day: 1))
        #expect(DayTicker.day(at: 11016) == LocalDate(year: 2000, month: 2, day: 29))
    }

    @Test func roundTripMatchesLocalDateOverCenturies() {
        // Gegen `LocalDate.adding(days:)` (Calendar) prüfen, inkl. Schaltjahre 1900/2000/2100.
        var day = LocalDate(year: 1890, month: 1, day: 1)
        var index = DayTicker.index(of: day)
        var mismatches = 0
        while day.year < 2140 {
            let next = day.adding(days: 1)
            if DayTicker.day(at: index) != day || DayTicker.index(of: next) != index + 1 { mismatches += 1 }
            day = next
            index += 1
        }
        #expect(mismatches == 0)
    }

    @Test func indexIgnoresDaylightSaving() {
        // 25.10.2026 hat in Berlin 25 Stunden, 29.03.2026 nur 23: trotzdem je ein Index weiter.
        let before = DayTicker.index(of: L.day(24))
        #expect(DayTicker.index(of: L.day(25)) == before + 1)
        #expect(DayTicker.index(of: L.day(26)) == before + 2)
        let march = DayTicker.index(of: LocalDate(year: 2026, month: 3, day: 29))
        #expect(DayTicker.day(at: march + 1) == LocalDate(year: 2026, month: 3, day: 30))
    }

    @Test func indexRangeCoversHundredYears() {
        let r = DayTicker.indexRange(around: today, years: 100)
        #expect(DayTicker.day(at: r.lowerBound) == LocalDate(year: 1926, month: 1, day: 1))
        #expect(DayTicker.day(at: r.upperBound - 1) == LocalDate(year: 2126, month: 12, day: 31))
        #expect(r.contains(DayTicker.index(of: today)))
    }

    @Test func windowStaysWhileVisibleInsideAndMovesAtEdge() {
        let w = DayTicker.window(current: 0..<0, visible: 100...108, step: 14)
        #expect(w == 70..<140)
        // Kleine Bewegung: gleicher Ausschnitt.
        #expect(DayTicker.window(current: w, visible: 105...113, step: 14) == w)
        // Nahe am Rand: neuer Ausschnitt um die sichtbaren Zellen.
        let moved = DayTicker.window(current: w, visible: 120...128, step: 14)
        #expect(moved != w && moved.lowerBound <= 120 - 14 && moved.upperBound > 128 + 14)
        // Weit weg (Sprung über Jahre), auch negative Indizes.
        let far = DayTicker.window(current: w, visible: -40_000 ... -39_990, step: 14)
        #expect(far.lowerBound <= -40_000 - 14 && far.upperBound > -39_990 + 14)
        #expect(DayTicker.window(current: far, visible: -40_000 ... -39_990, step: 14) == far)
    }

    @Test func summariesMatchDaysAndBucketMultiDay() {
        let occ = [
            L.timed("A", 23, 8, to: 11, calendar: "arbeit"), L.timed("B", 23, 10, to: 14, calendar: "arbeit"),
            L.timed("C", 23, 18, to: 19, calendar: "sport"), L.allDay("D", 24, calendar: "privat"),
            L.allDay("Urlaub", 26, through: 28, calendar: "privat"),
            // Über Mitternacht in den Tag der Zeitumstellung (25.10.) hinein.
            L.timed("Nacht", 24, 22, to: 2, endDay: 25, calendar: "sport"),
        ]
        let first = DayTicker.index(of: L.day(19))
        let s = DayTicker.summaries(occ, in: first..<(first + 14), today: today, calendar: L.cal)
        #expect(s.count == 14)
        let fixed = DayTicker.days(occ, from: L.day(19), count: 14, today: today, calendar: L.cal)
        #expect(fixed == (0..<14).map { s[first + $0]! })
        let t = s[DayTicker.index(of: L.day(23))]!
        #expect(t.isToday && t.calendarIds == ["arbeit", "sport"])
        #expect(abs(t.busyFraction - 7.0 / 12.0) < 0.0001)
        #expect(s[DayTicker.index(of: L.day(24))]!.calendarIds == ["privat", "sport"])
        #expect(s[DayTicker.index(of: L.day(25))]!.calendarIds == ["sport"])
        #expect((26...28).allSatisfy { s[DayTicker.index(of: L.day($0))]!.calendarIds == ["privat"] })
        #expect(s[DayTicker.index(of: L.day(29))]!.calendarIds.isEmpty)
    }

    @Test func summariesClipLongOccurrencesToRange() {
        let long = L.allDay("Sabbatical", 1, through: 31)
        let first = DayTicker.index(of: L.day(10))
        let s = DayTicker.summaries([long], in: first..<(first + 3), today: today, calendar: L.cal)
        #expect(s.keys.sorted() == [first, first + 1, first + 2])
        #expect(s.values.allSatisfy { $0.calendarIds == ["cal_1"] })
        #expect(DayTicker.summaries([long], in: first..<first, today: today, calendar: L.cal).isEmpty)
    }

    @Test func emptyDayHasLabelsButNoLoad() {
        let d = DayTicker.day(L.day(24), today: today, calendar: L.cal)
        #expect(d.weekdayShort == "Sa" && d.dayNumber == 24 && d.isWeekend && !d.isToday)
        #expect(d.calendarIds.isEmpty && d.busyFraction == 0)
    }
}
