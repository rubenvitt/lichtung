import Foundation
@testable import LichtungKit
import Testing

/// LIC-53: Standarddauer neuer Termine (Doppelklick, „Neuer Termin“,
/// Schnelleingabe ohne Dauer). Fr 23.10.2026, Europe/Berlin.
@Suite("Standarddauer neuer Termine")
struct NewEventDurationTests {
    typealias D = DraftFixtures
    typealias F = SyncFixtures

    @Test func storedValueFallsBackToStandard() {
        #expect(NewEventDuration.choices == [30, 45, 60, 90])
        #expect(NewEventDuration.minutes(stored: 0) == 60)
        #expect(NewEventDuration.minutes(stored: 45) == 45)
        #expect(NewEventDuration.minutes(stored: 90) == 90)
        // Nur die angebotenen Werte; alles andere ist der Standard.
        #expect(NewEventDuration.minutes(stored: 17) == 60)
        #expect(NewEventDuration.minutes(stored: -30) == 60)
    }

    @Test func labels() {
        #expect(NewEventDuration.choices.map(NewEventDuration.label) == ["30 Minuten", "45 Minuten", "1 Stunde", "1½ Stunden"])
        #expect(NewEventDuration.label(120) == "2 Stunden")
    }

    /// „Neuer Termin“: Beginn wie bisher, Ende nach der gewählten Dauer.
    @Test func newDraftLastsChosenMinutes() {
        let d = EventDraft.new(on: D.day(3), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz, id: "d")
        let short = d.lasting(minutes: 45)
        #expect(short.start == D.time(3, 9))
        #expect(short.end == D.time(3, 9, 45))
        #expect(short.id == "d" && short.calendarId == "cal_1" && short.isNew)
        #expect(d.lasting(minutes: 90).end == D.time(3, 10, 30))
        // Über Mitternacht.
        let late = EventDraft.new(startingAt: D.time(3, 23, 30), calendarId: nil, deviceTimeZone: D.tz)
        #expect(late.lasting(minutes: 30).end == D.time(4, 0))
    }

    @Test func allDayDraftIgnoresDuration() {
        var d = EventDraft.new(on: D.day(3), now: F.now, calendarId: "cal_1", deviceTimeZone: D.tz)
        d.setAllDay(true)
        #expect(d.lasting(minutes: 30) == d)
    }

    /// Schnelleingabe: ohne Ende oder Dauer gilt die Einstellung, sonst der Satz.
    @Test func quickEntryUsesDurationOnlyWhenSentenceHasNone() {
        let tz = TimeZone(identifier: "Europe/Berlin")!
        let cal = Calendar.lichtung(timeZone: tz)
        let parser = QuickEntryParser(timeZone: tz, defaultDurationMinutes: NewEventDuration.minutes(stored: 45))
        func minutes(_ text: String) -> Int? {
            guard case let .timed(start, end) = parser.parse(text, now: F.now).timing else { return nil }
            return Int(end.timeIntervalSince(start) / 60)
        }
        #expect(minutes("Zahnarzt morgen 14 Uhr") == 45)
        #expect(minutes("Lernen 2h morgen 14:00") == 120)
        #expect(minutes("Team-Sync morgen 9–9:30") == 30)
        guard case let .timed(start, _) = parser.parse("Zahnarzt morgen 14 Uhr", now: F.now).timing else {
            Issue.record("keine Uhrzeit")
            return
        }
        #expect(LocalDateTime(start, calendar: cal) == LocalDateTime(date: LocalDate(year: 2026, month: 10, day: 24), hour: 14, minute: 0))
    }
}
