import Foundation
@testable import LichtungKit
import Testing

/// Kern-Aufrufe, die Liste, Detail und Suche der Apps nutzen (LIC-42).
@Suite("Suche in den Apps")
struct AppSearchTests {
    typealias F = SyncFixtures
    let cal = Calendar.lichtung(timeZone: SyncFixtures.tz)

    func state() -> SyncState {
        var s = SyncState()
        s.calendars["cal_1"] = F.calendar("cal_1")
        s.calendars["cal_2"] = F.calendar("cal_2")
        s.calendars["cal_aus"] = F.calendar("cal_aus", hidden: true)
        s.calendarSets["set_arbeit"] = CalendarSet(id: "set_arbeit", name: "Arbeit", calendarIds: ["cal_2"], position: 0)
        s.events["privat"] = F.event("privat", title: "Review privat", day: 3)
        s.events["arbeit"] = F.event("arbeit", title: "Review Arbeit", day: 4, calendar: "cal_2")
        s.events["versteckt"] = F.event("versteckt", title: "Review geheim", day: 5, calendar: "cal_aus")
        return s
    }

    func query(_ s: SyncState) -> CalendarQuery { CalendarQuery(state: s, deviceTimeZone: F.tz) }

    @Test func followsCalendarSelection() {
        let q = query(state())
        #expect(q.search("review", now: F.now).map(\.id) == ["privat", "arbeit"])
        #expect(q.search("review", selection: .set("set_arbeit"), now: F.now).map(\.id) == ["arbeit"])
    }

    @Test func dropsSeriesWithoutOccurrenceInReach() {
        var s = state()
        // Zwei Montage im Januar 2020: weit außerhalb der zwei Jahre Reichweite.
        let start = LocalDate(year: 2020, month: 1, day: 6)
        s.events["alt"] = Event(
            id: "alt", calendarId: "cal_1",
            details: EventDetails(
                title: "Review alt",
                start: .dateTime(LocalDateTime(date: start, hour: 9, minute: 0), timeZone: "Europe/Berlin"),
                end: .dateTime(LocalDateTime(date: start, hour: 10, minute: 0), timeZone: "Europe/Berlin")
            ),
            recurrence: Recurrence(rrule: "FREQ=WEEKLY;COUNT=2")
        )
        #expect(!query(s).search("review", now: F.now).map(\.id).contains("alt"))
    }

    @Test func respectsLimit() {
        #expect(query(state()).search("review", now: F.now, limit: 1).map(\.id) == ["privat"])
    }

    @Test func dayAndDateText() throws {
        var s = state()
        s.events["heute"] = Event(id: "heute", calendarId: "cal_1", details: EventDetails(
            title: "Messe", start: .date(LocalDate(year: 2026, month: 10, day: 23)), end: .date(LocalDate(year: 2026, month: 10, day: 24))
        ))
        s.events["nacht"] = Event(id: "nacht", calendarId: "cal_1", details: EventDetails(
            title: "Nachtzug", start: F.at(6, 22), end: F.at(7, 6)
        ))
        let q = query(s)
        let today = LocalDate(F.now, calendar: cal)

        let privat = try #require(q.search("privat", now: F.now).first)
        #expect(privat.day(calendar: cal) == LocalDate(year: 2026, month: 11, day: 3))
        #expect(privat.dateText(today: today, calendar: cal) == "Di 3. Nov. · 09:00–10:00")

        let messe = try #require(q.search("messe", now: F.now).first)
        #expect(messe.dateText(today: today, calendar: cal) == "Heute · Fr 23. · ganztägig")

        let zug = try #require(q.search("nachtzug", now: F.now).first)
        #expect(zug.dateText(today: today, calendar: cal) == "Fr 6. Nov. · ab 22:00")
    }
}

@Suite("Detailtexte")
struct DetailTextTests {
    typealias F = SyncFixtures
    let cal = Calendar.lichtung(timeZone: SyncFixtures.tz)

    func occurrence(travel: Int?, allDay: Bool = false) -> Occurrence {
        let details: EventDetails = allDay
            ? EventDetails(title: "Ausflug", start: .date(LocalDate(year: 2026, month: 11, day: 3)), end: .date(LocalDate(year: 2026, month: 11, day: 4)), travelTimeMinutes: travel)
            : EventDetails(title: "Arzt", start: F.at(3, 14), end: F.at(3, 15), travelTimeMinutes: travel)
        let start = details.start.instant(deviceTimeZone: F.tz)
        let end = details.end.instant(deviceTimeZone: F.tz)
        return Occurrence(eventId: "e", calendarId: "cal_1", recurrenceId: nil, details: details, start: start, end: end, isModified: false)
    }

    @Test func travelTimeWithDeparture() {
        #expect(TravelTime.detailText(for: occurrence(travel: 30), calendar: cal) == "30 Min. Reisezeit · Aufbruch 13:30")
        #expect(TravelTime.detailText(for: occurrence(travel: 90), calendar: cal) == "1,5 Std. Reisezeit · Aufbruch 12:30")
    }

    @Test func noTravelTimeText() {
        #expect(TravelTime.detailText(for: occurrence(travel: nil), calendar: cal) == nil)
        #expect(TravelTime.detailText(for: occurrence(travel: 0), calendar: cal) == nil)
        #expect(TravelTime.detailText(for: occurrence(travel: 30, allDay: true), calendar: cal) == nil)
    }
}

@Suite("Rückgängig-Beschriftung")
struct UndoTitleTests {
    typealias F = SyncFixtures

    @Test func titlesNameTheLastAction() async {
        var s = SyncState()
        s.events["ev_1"] = F.event("ev_1", title: "Zahnarzt")
        let e = EngineFixtures.engine(ScriptedTransport([]), store: InMemoryCacheStore(s))
        #expect(await e.undoHistory.undoTitle == "Rückgängig")
        #expect(await e.undoHistory.redoTitle == "Wiederholen")

        await e.perform(.delete(eventId: "ev_1"), label: "Termin löschen")
        #expect(await e.undoHistory.undoTitle == "Rückgängig: Termin löschen")

        await e.undo()
        #expect(await e.undoHistory.undoTitle == "Rückgängig")
        #expect(await e.undoHistory.redoTitle == "Wiederholen: Termin löschen")
    }

    /// Die App liest den Verlauf nach jedem gemeldeten Stand nach. Das trägt
    /// nur, wenn er dann schon zur Änderung passt.
    @Test func historyIsCurrentWhenUpdateArrives() async {
        var s = SyncState()
        s.events["ev_1"] = F.event("ev_1", title: "Zahnarzt")
        let e = EngineFixtures.engine(ScriptedTransport([]), store: InMemoryCacheStore(s))
        let updates = await e.updates()
        // Liest wie `AppModel.observeUndo` und läuft parallel zu `perform`.
        let reader = Task { () -> String? in
            for await snapshot in updates where !snapshot.outbox.isEmpty {
                return await e.undoHistory.undoLabel
            }
            return nil
        }
        await e.perform(.delete(eventId: "ev_1"), label: "Termin löschen")
        #expect(await reader.value == "Termin löschen")
    }
}
