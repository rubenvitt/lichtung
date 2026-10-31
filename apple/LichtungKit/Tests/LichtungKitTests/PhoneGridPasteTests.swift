import Foundation
@testable import LichtungKit
import Testing

/// LIC-58: Halten auf leerer Fläche im iPhone-Raster bietet „Hier einfügen“
/// zur gehaltenen Uhrzeit, ohne das Anlegen per Ziehen zu ändern.
/// Spaltenhöhe 1440 Punkte, ein Punkt je Minute.
@Suite("iPhone: Einfügen im Raster")
struct PhoneGridPasteTests {
    typealias L = LayoutFixtures
    typealias G = GridInteraction
    let h = 1440.0

    @Test func withoutCopyHoldStaysCreation() {
        // Ohne Kopie wie bisher: Zug legt an, auch ohne Bewegung.
        #expect(G.slotHoldRelease(translation: (0, 0), canPaste: false) == .create)
        #expect(G.slotHoldRelease(translation: (0, 120), canPaste: false) == .create)
        // Hat der Zug nie begonnen, passiert nichts.
        #expect(G.slotHoldRelease(translation: nil, canPaste: false) == .nothing)
        #expect(G.slotHoldRelease(translation: nil, canPaste: true) == .nothing)
    }

    @Test func withCopyStillHoldOpensMenuAndDragStillCreates() {
        #expect(G.slotHoldRelease(translation: (0, 0), canPaste: true) == .menu)
        // Zittern des Fingers bleibt Menü.
        #expect(G.slotHoldRelease(translation: (3, -4), canPaste: true) == .menu)
        #expect(G.slotHoldRelease(translation: (0, 9.9), canPaste: true) == .menu)
        // Ab der Schwelle ist es ein Zug, in jede Richtung.
        #expect(G.slotHoldRelease(translation: (0, 10), canPaste: true) == .create)
        #expect(G.slotHoldRelease(translation: (-8, -8), canPaste: true) == .create)
        #expect(G.slotHoldRelease(translation: (0, 90), canPaste: true) == .create)
    }

    @Test func pasteStartRoundsLikeCreation() {
        let d = L.day(23)
        // Gehalten um 14:37: Beginn 14:30, wie der Beginn beim Aufziehen.
        let start = G.pasteStart(day: d, atY: 14 * 60 + 37, columnHeight: h, calendar: L.cal)
        #expect(LocalDateTime(start, calendar: L.cal) == LocalDateTime(date: d, hour: 14, minute: 30))
        #expect(start == G.creation(day: d, fromY: 14 * 60 + 37, toY: 14 * 60 + 37, columnHeight: h, calendar: L.cal).start)
        // Ganz unten bleibt der Beginn im Tag.
        let late = G.pasteStart(day: d, atY: 1439, columnHeight: h, calendar: L.cal)
        #expect(LocalDateTime(late, calendar: L.cal) == LocalDateTime(date: d, hour: 23, minute: 45))
        // Am 25.10. (Umstellung) zählt die Wanduhr der Zeile.
        let autumn = G.pasteStart(day: L.day(25), atY: 3 * 60 + 10, columnHeight: h, calendar: L.cal)
        #expect(LocalDateTime(autumn, calendar: L.cal) == LocalDateTime(date: L.day(25), hour: 3, minute: 0))
    }

    @Test func pasteAtTimeKeepsDurationAndZone() {
        let clip = EventClipboard(L.timed("Team-Sync", 23, 9, to: 9, 30), calendar: L.cal)
        let start = L.day(27).date(hour: 14, minute: 30, in: L.cal)
        guard case let .create(cal, input) = clip.intent(pastingAt: start, calendarId: "cal_2", deviceTimeZone: L.tz) else {
            Issue.record("keine Anlage")
            return
        }
        #expect(cal == "cal_2")
        #expect(input.start == .dateTime(LocalDateTime(date: L.day(27), hour: 14, minute: 30), timeZone: "Europe/Berlin"))
        #expect(input.end.normalized == .dateTime(LocalDateTime(date: L.day(27), hour: 15, minute: 0), timeZone: "Europe/Berlin"))
        #expect(input.recurrence == nil)
        #expect(clip.pasteTitle(at: start, calendar: L.cal) == "„Team-Sync“ einfügen um 14:30")
    }

    @Test func pasteAtTimeInOtherZoneKeepsThatZone() {
        // Termin in New York 9:00–10:00, eingefügt um 20:00 Berliner Zeit = 15:00 in New York.
        var occ = L.timed("Call", 23, 9, to: 10)
        occ.details.start = .dateTime(LocalDateTime(date: L.day(23), hour: 9, minute: 0), timeZone: "America/New_York")
        occ.details.end = .dateTime(LocalDateTime(date: L.day(23), hour: 10, minute: 0), timeZone: "America/New_York")
        let clip = EventClipboard(occ, calendar: L.cal)
        let start = L.day(27).date(hour: 20, minute: 0, in: L.cal)
        guard case let .create(_, input) = clip.intent(pastingAt: start, calendarId: "cal_1", deviceTimeZone: L.tz) else {
            Issue.record("keine Anlage")
            return
        }
        #expect(input.start == .dateTime(LocalDateTime(date: L.day(27), hour: 15, minute: 0), timeZone: "America/New_York"))
        #expect(input.end.normalized == .dateTime(LocalDateTime(date: L.day(27), hour: 16, minute: 0), timeZone: "America/New_York"))
    }

    @Test func pasteAllDayAtTimeStaysAllDay() {
        let clip = EventClipboard(L.allDay("Urlaub", 23, through: 24), calendar: L.cal)
        let start = L.day(2, month: 11).date(hour: 10, minute: 0, in: L.cal)
        guard case let .create(_, input) = clip.intent(pastingAt: start, calendarId: "cal_1", deviceTimeZone: L.tz) else {
            Issue.record("keine Anlage")
            return
        }
        #expect(input.start == .date(L.day(2, month: 11)))
        #expect(input.end == .date(L.day(4, month: 11)))
        #expect(clip.pasteTitle(at: start, calendar: L.cal) == "„Urlaub“ einfügen am Mo 2.11.")
    }
}
