import Foundation
@testable import LichtungKit
import Testing

/// LIC-22: Ziehen im Raster. Spaltenhöhe 1440 Punkte, also ein Punkt je Minute.
@Suite("Zeitraster: anlegen, verschieben, Länge ändern")
struct GridInteractionTests {
    typealias L = LayoutFixtures
    typealias G = GridInteraction
    let h = 1440.0

    func at(_ d: LocalDate, _ hour: Int, _ minute: Int = 0) -> Date { d.date(hour: hour, minute: minute, in: L.cal) }

    func berlin(_ i: DateInterval) -> String {
        "\(LocalDateTime(i.start, calendar: L.cal))…\(LocalDateTime(i.end, calendar: L.cal))"
    }

    @Test func pointsToMinutes() {
        #expect(G.minutes(atY: 720, columnHeight: 1152) == 900)
        #expect(G.minutes(atY: -5, columnHeight: h) == 0)
        #expect(G.minutes(atY: 2000, columnHeight: h) == 1440)
        #expect(G.minutes(forDistance: -48, columnHeight: 1152) == -60)
        #expect(G.snapped(37) == 30 && G.snapped(38) == 45 && G.snapped(-20) == -15)
    }

    @Test func dayOffsetStaysInVisibleColumns() {
        #expect(G.dayOffset(forDistance: 130, columnWidth: 100, column: 2, columns: 7) == 1)
        #expect(G.dayOffset(forDistance: 940, columnWidth: 100, column: 2, columns: 7) == 4)
        #expect(G.dayOffset(forDistance: -340, columnWidth: 100, column: 2, columns: 7) == -2)
        #expect(G.dayOffset(forDistance: 400, columnWidth: 100, column: 0, columns: 1) == 0)
    }

    @Test func createSnapsOutwardInBothDirections() {
        let d = L.day(23)
        let down = G.creation(day: d, fromY: 545, toY: 610, columnHeight: h, calendar: L.cal)
        #expect(berlin(down) == "2026-10-23T09:00:00…2026-10-23T10:15:00")
        let up = G.creation(day: d, fromY: 610, toY: 545, columnHeight: h, calendar: L.cal)
        #expect(up == down)
        // Kaum bewegt: mindestens 15 Minuten.
        let tap = G.creation(day: d, fromY: 545, toY: 546, columnHeight: h, calendar: L.cal)
        #expect(berlin(tap) == "2026-10-23T09:00:00…2026-10-23T09:15:00")
        // Am Tagesende: endet um Mitternacht, nicht danach.
        let late = G.creation(day: d, fromY: 1436, toY: 1439, columnHeight: h, calendar: L.cal)
        #expect(berlin(late) == "2026-10-23T23:45:00…2026-10-24T00:00:00")
    }

    @Test func createOnChangeoverDays() {
        let autumn = G.creation(day: L.day(25), fromY: 720, toY: 780, columnHeight: h, calendar: L.cal)
        #expect(berlin(autumn) == "2026-10-25T12:00:00…2026-10-25T13:00:00")
        #expect(autumn.duration == 3600)
        // In der fehlenden Stunde: erster gültiger Zeitpunkt, 15 Minuten lang.
        let spring = LocalDate(year: 2027, month: 3, day: 28)
        let gap = G.creation(day: spring, fromY: 125, toY: 130, columnHeight: h, calendar: L.cal)
        #expect(berlin(gap) == "2027-03-28T03:00:00…2027-03-28T03:15:00")
    }

    @Test func moveKeepsDurationAndSnapsDelta() {
        let d = L.day(23)
        let s = at(d, 9), e = at(d, 10)
        #expect(berlin(G.moved(start: s, end: e, deltaMinutes: 37, dayOffset: 0, calendar: L.cal)) == "2026-10-23T09:30:00…2026-10-23T10:30:00")
        #expect(berlin(G.moved(start: s, end: e, deltaMinutes: -20, dayOffset: 0, calendar: L.cal)) == "2026-10-23T08:45:00…2026-10-23T09:45:00")
        // Weniger als ein halber Schritt: bleibt.
        #expect(G.moved(start: s, end: e, deltaMinutes: 7, dayOffset: 0, calendar: L.cal) == DateInterval(start: s, end: e))
        // Ein Termin um 9:05 bleibt um 5 Minuten neben dem Raster.
        let odd = G.moved(start: at(d, 9, 5), end: at(d, 9, 50), deltaMinutes: 15, dayOffset: 0, calendar: L.cal)
        #expect(berlin(odd) == "2026-10-23T09:20:00…2026-10-23T10:05:00")
    }

    @Test func moveAcrossDaysAndMidnight() {
        let d = L.day(23)
        // Zwei Spalten weiter ist der Umstellungstag: 9:00 bleibt 9:00, eine Stunde bleibt eine Stunde.
        let week = G.moved(start: at(d, 9), end: at(d, 10), deltaMinutes: 0, dayOffset: 2, calendar: L.cal)
        #expect(berlin(week) == "2026-10-25T09:00:00…2026-10-25T10:00:00")
        let night = G.moved(start: at(d, 23), end: at(d, 23, 45), deltaMinutes: 90, dayOffset: 0, calendar: L.cal)
        #expect(berlin(night) == "2026-10-24T00:30:00…2026-10-24T01:15:00")
        let back = G.moved(start: at(d, 0, 30), end: at(d, 1), deltaMinutes: -60, dayOffset: 0, calendar: L.cal)
        #expect(berlin(back) == "2026-10-22T23:30:00…2026-10-23T00:00:00")
        // Über die Umstellung im Frühjahr: 1 Stunde echte Zeit, auch wenn die Uhr springt.
        let sat = LocalDate(year: 2027, month: 3, day: 27)
        let spring = G.moved(start: at(sat, 1, 30), end: at(sat, 2, 30), deltaMinutes: 0, dayOffset: 1, calendar: L.cal)
        #expect(berlin(spring) == "2027-03-28T01:30:00…2027-03-28T03:30:00")
        #expect(spring.duration == 3600)
    }

    @Test func resizeKeepsMinimumAndStaysInDay() {
        let d = L.day(23)
        let s = at(d, 9), e = at(d, 10)
        #expect(berlin(G.resized(start: s, end: e, day: d, deltaMinutes: 50, calendar: L.cal)) == "2026-10-23T09:00:00…2026-10-23T10:45:00")
        #expect(berlin(G.resized(start: s, end: e, day: d, deltaMinutes: -100, calendar: L.cal)) == "2026-10-23T09:00:00…2026-10-23T09:15:00")
        #expect(berlin(G.resized(start: s, end: e, day: d, deltaMinutes: 2000, calendar: L.cal)) == "2026-10-23T09:00:00…2026-10-24T00:00:00")
        #expect(G.resized(start: s, end: e, day: d, deltaMinutes: 5, calendar: L.cal) == DateInterval(start: s, end: e))
        // Ende in die fehlende Stunde: erster gültiger Zeitpunkt danach.
        let spring = LocalDate(year: 2027, month: 3, day: 28)
        let r = G.resized(start: at(spring, 1), end: at(spring, 1, 30), day: spring, deltaMinutes: 60, calendar: L.cal)
        #expect(berlin(r) == "2027-03-28T01:00:00…2027-03-28T03:00:00")
    }

    @Test func onlyTimedEventsInWritableCalendarsAreDraggable() {
        let writable = CalendarInfo(id: "cal_1", accountId: "a", name: "Privat")
        let readOnly = CalendarInfo(id: "cal_1", accountId: "a", name: "Feiertage", writable: false)
        #expect(G.isDraggable(L.timed("A", 23, 9, to: 10), calendar: writable))
        #expect(!G.isDraggable(L.timed("A", 23, 9, to: 10), calendar: readOnly))
        #expect(!G.isDraggable(L.timed("A", 23, 9, to: 10), calendar: nil))
        #expect(!G.isDraggable(L.allDay("Urlaub", 23), calendar: writable))
    }

    @Test func previewFrameIsClippedPerDay() {
        let night = DateInterval(start: at(L.day(23), 22), end: at(L.day(24), 2))
        let first = G.frame(of: night, day: L.day(23), calendar: L.cal)
        #expect(abs((first?.top ?? 0) - 1320.0 / 1440) < 1e-9 && abs((first?.height ?? 0) - 120.0 / 1440) < 1e-9)
        #expect(G.frame(of: night, day: L.day(24), calendar: L.cal) == GridFrame(top: 0, height: 120.0 / 1440))
        #expect(G.frame(of: night, day: L.day(25), calendar: L.cal) == nil)
        #expect(G.timeText(night, calendar: L.cal) == "22:00–2:00")
    }

    @Test func moveIntentKeepsZoneAndUsesThisForSeries() {
        let new = DateInterval(start: at(L.day(23), 9, 30), end: at(L.day(23), 10, 30))
        let single = L.timed("ev_1", 23, 9, to: 10)
        let changes = EventChanges(
            start: .dateTime(LocalDateTime(date: L.day(23), hour: 9, minute: 30), timeZone: "Europe/Berlin"),
            end: .dateTime(LocalDateTime(date: L.day(23), hour: 10, minute: 30), timeZone: "Europe/Berlin")
        )
        #expect(G.moveIntent(single, to: new, deviceTimeZone: L.tz) == .update(eventId: "ev_1", scope: .all, changes: changes))

        var instance = single
        instance.recurrenceId = SyncFixtures.at(23, 9, month: 10)
        #expect(G.moveIntent(instance, to: new, deviceTimeZone: L.tz)
            == .update(eventId: "ev_1", scope: .this, occurrence: instance.recurrenceId, changes: changes))

        // Termin in New York: neue Zeit als Wanduhr in New York.
        var ny = single
        ny.details.start = .dateTime(LocalDateTime(date: L.day(23), hour: 3, minute: 0), timeZone: "America/New_York")
        ny.details.end = .dateTime(LocalDateTime(date: L.day(23), hour: 4, minute: 0), timeZone: "America/New_York")
        if case let .update(_, _, _, c)? = G.moveIntent(ny, to: new, deviceTimeZone: L.tz) {
            #expect(c.start == .dateTime(LocalDateTime(date: L.day(23), hour: 3, minute: 30), timeZone: "America/New_York"))
        } else {
            Issue.record("keine Änderung")
        }

        // Floating bleibt floating.
        var floating = single
        floating.details.start = .dateTime(LocalDateTime(date: L.day(23), hour: 9, minute: 0), timeZone: nil)
        if case let .update(_, _, _, c)? = G.moveIntent(floating, to: new, deviceTimeZone: L.tz) {
            #expect(c.start == .dateTime(LocalDateTime(date: L.day(23), hour: 9, minute: 30), timeZone: nil))
        } else {
            Issue.record("keine Änderung")
        }

        #expect(G.moveIntent(L.allDay("Urlaub", 23), to: new, deviceTimeZone: L.tz) == nil)
    }

    @Test func createIntentInDeviceZone() {
        let i = DateInterval(start: at(L.day(23), 9), end: at(L.day(23), 10))
        guard case let .create(cal, input) = G.createIntent(calendarId: "cal_1", interval: i, timeZone: L.tz) else {
            Issue.record("kein Anlegen")
            return
        }
        #expect(cal == "cal_1" && input.title == "Neuer Termin")
        #expect(input.start == .dateTime(LocalDateTime(date: L.day(23), hour: 9, minute: 0), timeZone: "Europe/Berlin"))
        #expect(input.end == .dateTime(LocalDateTime(date: L.day(23), hour: 10, minute: 0), timeZone: "Europe/Berlin"))
    }

    @Test func movingOneOccurrenceIsUndoable() async {
        typealias F = SyncFixtures
        var s = SyncState()
        s.events["ev_s"] = F.event("ev_s", day: 2, rrule: "FREQ=WEEKLY;BYDAY=MO")
        let engine = EngineFixtures.engine(ScriptedTransport([]), store: InMemoryCacheStore(s))
        let day = LocalDate(year: 2026, month: 11, day: 9)
        let occ = Occurrence(eventId: "ev_s", calendarId: "cal_1", recurrenceId: F.at(9, 9),
                             details: EventDetails(title: "Termin", start: F.at(9, 9), end: F.at(9, 10)),
                             start: at(day, 9), end: at(day, 10), isModified: false)
        let target = G.moved(start: occ.start, end: occ.end, deltaMinutes: 120, dayOffset: 0, calendar: L.cal)
        await engine.perform(G.moveIntent(occ, to: target, deviceTimeZone: L.tz)!, label: "Termin verschieben")
        let moved = await engine.visibleEvents()["ev_s"]
        #expect(moved?.overrides.first?.details.start == F.at(9, 11))
        #expect(moved?.details.start == F.at(2, 9)) // Serie bleibt
        await engine.undo()
        #expect(await engine.visibleEvents()["ev_s"]?.overrides.first?.details.start == F.at(9, 9))
    }
}
