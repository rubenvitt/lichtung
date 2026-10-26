import Foundation
@testable import LichtungKit
import Testing

/// LIC-47: Feinschliff im Raster (Editor nach dem Anlegen, Doppelklick,
/// Standardkalender, Serien-Rückfrage, Bedienhilfen, automatisch scrollen,
/// zweite Zeitzone pro Tag). Spaltenhöhe 1440 Punkte, ein Punkt je Minute.
@Suite("Zeitraster: Feinschliff")
struct GridPolishTests {
    typealias L = LayoutFixtures
    typealias G = GridInteraction
    typealias F = SyncFixtures
    let h = 1440.0

    func at(_ d: LocalDate, _ hour: Int, _ minute: Int = 0) -> Date { d.date(hour: hour, minute: minute, in: L.cal) }

    func berlin(_ i: DateInterval) -> String {
        "\(LocalDateTime(i.start, calendar: L.cal))…\(LocalDateTime(i.end, calendar: L.cal))"
    }

    // MARK: Editor nach dem Anlegen

    @Test func dragCreationBecomesDraft() {
        let interval = G.creation(day: L.day(23), fromY: 545, toY: 610, columnHeight: h, calendar: L.cal)
        let d = EventDraft.new(spanning: interval, calendarId: "cal_2", deviceTimeZone: L.tz, id: "neu")
        #expect(d.isNew && d.id == "neu" && d.calendarId == "cal_2")
        #expect(d.start == LocalDateTime(date: L.day(23), hour: 9, minute: 0))
        #expect(d.end == LocalDateTime(date: L.day(23), hour: 10, minute: 15))
        #expect(d.timeZoneID == "Europe/Berlin" && d.title.isEmpty)
        guard case let .create(cal, input)? = d.saveIntent() else {
            Issue.record("kein Anlegen")
            return
        }
        #expect(cal == "cal_2" && input.title == EventDraft.defaultTitle)
        #expect(input.end == .dateTime(LocalDateTime(date: L.day(23), hour: 10, minute: 15), timeZone: "Europe/Berlin"))
    }

    @Test func dragCreationOverChangeoverKeepsWallClock() {
        // 25.10.: Die Zeilen 1–4 Uhr sind vier echte Stunden lang.
        let interval = G.creation(day: L.day(25), fromY: 60, toY: 240, columnHeight: h, calendar: L.cal)
        let d = EventDraft.new(spanning: interval, calendarId: "cal_1", deviceTimeZone: L.tz)
        #expect(d.start == LocalDateTime(date: L.day(25), hour: 1, minute: 0))
        #expect(d.end == LocalDateTime(date: L.day(25), hour: 4, minute: 0))
        #expect(d.canSave)
    }

    @Test func doubleClickDraftAtRoundedTime() {
        // Doppelklick um 14:37: Beginn 14:30, eine Stunde lang.
        let start = G.slotStart(day: L.day(23), atY: 14 * 60 + 37, columnHeight: h)
        let d = EventDraft.new(startingAt: start, calendarId: "cal_1", deviceTimeZone: L.tz)
        #expect(d.start == LocalDateTime(date: L.day(23), hour: 14, minute: 30))
        #expect(d.end == LocalDateTime(date: L.day(23), hour: 15, minute: 30))
        // Spät abends passt die Stunde noch in den Tag.
        let late = G.slotStart(day: L.day(23), atY: 23 * 60 + 50, columnHeight: h)
        #expect(late == LocalDateTime(date: L.day(23), hour: 23, minute: 0))
    }

    // MARK: Standardkalender

    @Test func defaultCalendarFromSettingsWithFallback() {
        let writable = [F.calendar("arbeit"), F.calendar("privat")]
        #expect(DefaultCalendar.choose(stored: "privat", writable: writable) == "privat")
        #expect(DefaultCalendar.choose(stored: "", writable: writable) == "arbeit")
        #expect(DefaultCalendar.choose(stored: nil, writable: writable) == "arbeit")
        // Gewählt, aber nicht mehr beschreibbar oder ausgeblendet: erster beschreibbarer.
        #expect(DefaultCalendar.choose(stored: "abo", writable: writable) == "arbeit")
        #expect(DefaultCalendar.choose(stored: "privat", writable: []) == nil)
        #expect(DefaultCalendar.fallbackNote(stored: "privat", writable: writable) == nil)
        #expect(DefaultCalendar.fallbackNote(stored: "", writable: writable) == nil)
        #expect(DefaultCalendar.fallbackNote(stored: "abo", writable: writable)
            == "Der gewählte Kalender ist nicht mehr beschreibbar. Neue Termine landen in „arbeit“.")
    }

    // MARK: Serien-Rückfrage

    func seriesOccurrence() -> Occurrence {
        let day = LocalDate(year: 2026, month: 11, day: 9)
        return Occurrence(eventId: "ev_s", calendarId: "cal_1", recurrenceId: F.at(9, 9),
                          details: EventDetails(title: "Termin", start: F.at(9, 9), end: F.at(9, 10)),
                          start: at(day, 9), end: at(day, 10), isModified: false)
    }

    @Test func seriesMoveAsksForScope() {
        let occ = seriesOccurrence()
        let single = L.timed("Einzeln", 23, 9, to: 10)
        #expect(G.moveScopes(occ) == [.this, .thisAndFollowing])
        #expect(G.moveScopes(single).isEmpty)
        let target = G.moved(start: occ.start, end: occ.end, deltaMinutes: 60, dayOffset: 0, calendar: L.cal)
        let changes = EventChanges(start: F.at(9, 10), end: F.at(9, 11))
        #expect(G.moveIntent(occ, to: target, scope: .thisAndFollowing, deviceTimeZone: L.tz)
            == .update(eventId: "ev_s", scope: .thisAndFollowing, occurrence: F.at(9, 9), changes: changes))
        #expect(G.moveIntent(occ, to: target, scope: .this, deviceTimeZone: L.tz)
            == .update(eventId: "ev_s", scope: .this, occurrence: F.at(9, 9), changes: changes))
        // „Alle“ bietet das Ziehen nicht an.
        #expect(G.moveIntent(occ, to: target, scope: .all, deviceTimeZone: L.tz) == nil)
        // Einzeltermine gelten ganz, egal was gewählt ist.
        if case let .update(_, scope, occurrence, _)? = G.moveIntent(single, to: target, scope: .thisAndFollowing, deviceTimeZone: L.tz) {
            #expect(scope == .all && occurrence == nil)
        } else {
            Issue.record("keine Änderung")
        }
    }

    @Test func thisAndFollowingFromGridSplitsLocally() async {
        var s = SyncState()
        s.events["ev_s"] = F.event("ev_s", day: 2, rrule: "FREQ=WEEKLY;BYDAY=MO")
        let engine = EngineFixtures.engine(ScriptedTransport([]), store: InMemoryCacheStore(s))
        let occ = seriesOccurrence()
        let target = G.moved(start: occ.start, end: occ.end, deltaMinutes: 60, dayOffset: 0, calendar: L.cal)
        await engine.perform(G.moveIntent(occ, to: target, scope: .thisAndFollowing, deviceTimeZone: L.tz)!, label: "Termin verschieben")
        let events = await engine.visibleEvents()
        #expect(events["ev_s"]?.details.start == F.at(2, 9), "alte Serie bleibt bis davor")
        #expect(events.values.contains { $0.id != "ev_s" && $0.details.start == F.at(9, 10) }, "neue Serie ab 10 Uhr")
    }

    // MARK: Bedienhilfen

    @Test func nudgeByFifteenMinutes() {
        let d = L.day(23)
        let s = at(d, 9), e = at(d, 10)
        #expect(G.nudgeActions.map(\.name) == ["15 Minuten früher", "15 Minuten später"])
        #expect(G.nudgeActions.map(\.minutes) == [-15, 15])
        #expect(berlin(G.nudged(start: s, end: e, minutes: -15, calendar: L.cal)) == "2026-10-23T08:45:00…2026-10-23T09:45:00")
        #expect(berlin(G.nudged(start: s, end: e, minutes: 15, calendar: L.cal)) == "2026-10-23T09:15:00…2026-10-23T10:15:00")
        // Über Mitternacht in den Vortag.
        let early = G.nudged(start: at(d, 0, 10), end: at(d, 0, 40), minutes: -15, calendar: L.cal)
        #expect(berlin(early) == "2026-10-22T23:55:00…2026-10-23T00:25:00")
        // Über die Umstellung: Die Dauer bleibt eine Stunde.
        let autumnStart = at(L.day(25), 1, 45)
        let autumn = G.nudged(start: autumnStart, end: autumnStart.addingTimeInterval(3600), minutes: 15, calendar: L.cal)
        #expect(autumn.duration == 3600)
    }

    // MARK: Automatisch scrollen

    @Test func autoScrollOnlyNearEdges() {
        // Ausschnitt 600 Punkte, Zone 44 Punkte.
        #expect(G.autoScrollStep(pointerY: 300, viewportHeight: 600) == 0)
        #expect(G.autoScrollStep(pointerY: 44, viewportHeight: 600) == 0)
        #expect(G.autoScrollStep(pointerY: 556, viewportHeight: 600) == 0)
        #expect(G.autoScrollStep(pointerY: 22, viewportHeight: 600) == -8)
        #expect(G.autoScrollStep(pointerY: 0, viewportHeight: 600) == -16)
        #expect(G.autoScrollStep(pointerY: 578, viewportHeight: 600) == 8)
        // Über den Rand hinaus: nicht schneller als der größte Schritt.
        #expect(G.autoScrollStep(pointerY: -200, viewportHeight: 600) == -16)
        #expect(G.autoScrollStep(pointerY: 900, viewportHeight: 600) == 16)
        // Kleiner Ausschnitt: Zone höchstens ein Viertel.
        #expect(G.autoScrollStep(pointerY: 50, viewportHeight: 120) == 0)
        #expect(G.autoScrollStep(pointerY: 0, viewportHeight: 120) == -16)
        #expect(G.autoScrollStep(pointerY: 10, viewportHeight: 0) == 0)
    }

    @Test func autoScrollStaysInsideContent() {
        // Inhalt 24 × 48 = 1152 Punkte, Ausschnitt 600.
        #expect(G.autoScrolledOffset(100, step: -16, contentHeight: 1152, viewportHeight: 600) == 84)
        #expect(G.autoScrolledOffset(8, step: -16, contentHeight: 1152, viewportHeight: 600) == 0)
        #expect(G.autoScrolledOffset(545, step: 16, contentHeight: 1152, viewportHeight: 600) == 552)
        #expect(G.autoScrolledOffset(0, step: 16, contentHeight: 400, viewportHeight: 600) == 0)
    }

    // MARK: Zweite Zeitzone pro Tag

    @Test func secondZonePerDayInChangeoverWeek() {
        let ny = TimeZone(identifier: "America/New_York")!
        // Woche 26.10.–1.11.2026: Berlin schon Winterzeit, New York stellt am
        // 1.11. um 2:00 Ortszeit (7:00 Berlin) um.
        let days = (0..<7).map { L.day(26).adding(days: $0) }
        let labels = TimelineLayout.secondaryHourLabels(for: ny, home: L.tz, days: days)
        #expect(labels.gutter[7] == "02" && labels.gutter[12] == "07")
        #expect(labels.perDay.count == 7)
        #expect(labels.perDay[0..<6].allSatisfy { $0 == nil })
        let sunday = labels.perDay[6]
        #expect(sunday?[6] == nil, "bis 7 Uhr Berlin wie links")
        #expect(sunday?[7] == "01" && sunday?[12] == "06")
        // Woche davor: Berlin stellt am Sonntag 25.10. um, New York nicht.
        let before = TimelineLayout.secondaryHourLabels(for: ny, home: L.tz, days: (19...25).map { L.day($0) })
        #expect(before.gutter[12] == "06")
        #expect(before.perDay[0..<6].allSatisfy { $0 == nil })
        #expect(before.perDay[6]?[12] == "07")
        // Ohne Umstellung: nichts pro Tag.
        let normal = TimelineLayout.secondaryHourLabels(for: ny, home: L.tz, days: (9...15).map { L.day($0, month: 11) })
        #expect(normal.perDay.allSatisfy { $0 == nil })
        #expect(TimelineLayout.secondaryHourLabels(for: ny, home: L.tz, days: []).perDay.isEmpty)
    }
}
