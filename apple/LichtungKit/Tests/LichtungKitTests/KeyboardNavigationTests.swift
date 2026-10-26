import Foundation
@testable import LichtungKit
import Testing

/// LIC-24: Tastaturbedienung und Kontextmenüs am Mac. Fr 23.10.2026,
/// Woche Mo 19.–So 25.10., Europe/Berlin.
@Suite("Tastatur: Ansichten und Blättern")
struct CalendarViewModeTests {
    typealias L = LayoutFixtures

    @Test func shortcutsAndLabels() {
        #expect(CalendarViewMode.allCases.map(\.shortcutDigit) == ["1", "2", "3", "4"])
        #expect(CalendarViewMode.allCases.map(\.label) == ["Tag", "Woche", "Monat", "Jahr"])
    }

    @Test func stepsByViewUnit() {
        let d = L.day(23)
        #expect(CalendarViewMode.day.stepped(d, by: 1) == L.day(24))
        #expect(CalendarViewMode.day.stepped(d, by: -1) == L.day(22))
        #expect(CalendarViewMode.week.stepped(d, by: 1) == L.day(30))
        #expect(CalendarViewMode.week.stepped(d, by: -2) == L.day(9))
        #expect(CalendarViewMode.month.stepped(d, by: 1) == L.day(1, month: 11))
        #expect(CalendarViewMode.month.stepped(L.day(31), by: -1) == L.day(1, month: 9))
        #expect(CalendarViewMode.year.stepped(d, by: 1) == LocalDate(year: 2027, month: 10, day: 1))
    }

    @Test func visibleDaysPerView() {
        let d = L.day(23)
        #expect(CalendarViewMode.day.visibleDays(around: d) == DaySpan(start: d, count: 1))
        #expect(CalendarViewMode.week.visibleDays(around: d) == DaySpan(start: L.day(19), count: 7))
        // Sonntag gehört zur Woche ab Montag davor.
        #expect(CalendarViewMode.week.visibleDays(around: L.day(25))?.start == L.day(19))
        // Oktober 2026 beginnt am Donnerstag: Raster ab Mo 28.9., sechs Wochen.
        #expect(CalendarViewMode.month.visibleDays(around: d) == DaySpan(start: L.day(28, month: 9), count: 42))
        #expect(CalendarViewMode.year.visibleDays(around: d) == nil)
    }

    @Test func daySpanContainsAndInterval() {
        let span = DaySpan(start: L.day(19), count: 7)
        #expect(span.contains(L.day(19)) && span.contains(L.day(25)))
        #expect(!span.contains(L.day(26)) && !span.contains(L.day(18)))
        #expect(span.interval(in: L.cal).duration == 7 * 86_400 + 3600) // Umstellung am 25.10.
    }
}

/// LIC-50: ←/→ wandern Tag für Tag, ⌘←/⌘→ blättern über das Menü.
@Suite("Tastatur: Tag für Tag")
struct DaySelectionTests {
    typealias L = LayoutFixtures
    typealias N = OccurrenceNavigation

    @Test func arrowsMoveOneDayInEveryView() {
        for mode in CalendarViewMode.allCases {
            #expect(mode.movedSelection(L.day(23), forward: true) == L.day(24))
            #expect(mode.movedSelection(L.day(23), forward: false) == L.day(22))
        }
        #expect(CalendarViewMode.month.movedSelection(L.day(31), forward: true) == L.day(1, month: 11))
    }

    @Test func weekPagesOnlyAtItsEdges() {
        let week = CalendarViewMode.week
        // Fr → Sa → So bleibt in der Woche Mo 19.–So 25.10.
        var day = L.day(23)
        for _ in 0..<2 {
            let next = week.movedSelection(day, forward: true)
            #expect(!week.pages(from: day, to: next))
            #expect(week.visibleDays(around: next)?.start == L.day(19))
            day = next
        }
        // So → Mo blättert in die nächste Woche.
        let monday = week.movedSelection(day, forward: true)
        #expect(monday == L.day(26))
        #expect(week.pages(from: day, to: monday))
        #expect(week.visibleDays(around: monday)?.start == L.day(26))
        // Mo ← blättert zurück.
        #expect(week.pages(from: L.day(19), to: week.movedSelection(L.day(19), forward: false)))
    }

    @Test func otherViewsPage() {
        // Tag: jeder Schritt ist eine neue Seite.
        #expect(CalendarViewMode.day.pages(from: L.day(23), to: L.day(24)))
        // Monat: erst am Monatswechsel, auch wenn der Tag schon im Raster stand.
        #expect(!CalendarViewMode.month.pages(from: L.day(23), to: L.day(24)))
        #expect(CalendarViewMode.month.pages(from: L.day(31), to: L.day(1, month: 11)))
        // Jahr: erst am Jahreswechsel.
        #expect(!CalendarViewMode.year.pages(from: L.day(23), to: L.day(24)))
        #expect(CalendarViewMode.year.pages(from: LocalDate(year: 2026, month: 12, day: 31), to: LocalDate(year: 2027, month: 1, day: 1)))
    }

    @Test func stepTitles() {
        #expect(CalendarViewMode.allCases.map { $0.stepTitle(forward: true) } == ["Nächster Tag", "Nächste Woche", "Nächster Monat", "Nächstes Jahr"])
        #expect(CalendarViewMode.allCases.map { $0.stepTitle(forward: false) } == ["Vorheriger Tag", "Vorherige Woche", "Vorheriger Monat", "Vorheriges Jahr"])
    }

    let week = [
        L.allDay("Bereitschaft", 20),
        L.timed("Frühstück", 21, 8, to: 9),
        L.timed("Team-Sync", 21, 9, to: 9, 30),
        L.timed("Mittag", 21, 12, to: 13),
        L.allDay("Messe", 22, through: 23),
        L.timed("Workshop", 23, 10, to: 11),
        L.timed("Nachtschicht", 23, 22, to: 6, endDay: 24),
    ]

    @Test func highlightMovesSidewaysToNearestTime() {
        // Di 20. ganztägig → Mi 21.: kein ganztägiger, also der erste Termin.
        #expect(N.counterpart(of: week[0], on: L.day(21), in: week, calendar: L.cal)?.title == "Frühstück")
        // Workshop Fr 10 Uhr → Mi 21.: Team-Sync (9:00) liegt näher als Mittag (12:00).
        #expect(N.counterpart(of: week[5], on: L.day(21), in: week, calendar: L.cal)?.title == "Team-Sync")
        // Gleicher Abstand (Team-Sync 9:00 → 8:00 und 10:00 am Do): der frühere.
        let tie = week + [L.timed("Früh", 22, 8, to: 8, 30), L.timed("Spät", 22, 10, to: 10, 30)]
        #expect(N.counterpart(of: week[2], on: L.day(22), in: tie, calendar: L.cal)?.title == "Früh")
        // Ganztägig → ganztägig.
        #expect(N.counterpart(of: week[0], on: L.day(22), in: week, calendar: L.cal)?.title == "Messe")
        // Nur ganztägige am Ziel: der erste davon.
        #expect(N.counterpart(of: week[3], on: L.day(22), in: week, calendar: L.cal)?.title == "Messe")
    }

    @Test func highlightStaysOnMultiDayAndDropsOnEmptyDay() {
        // Messe Do–Fr bleibt markiert, wenn der Tag auf Fr wechselt.
        #expect(N.counterpart(of: week[4], on: L.day(23), in: week, calendar: L.cal)?.title == "Messe")
        // Nachtschicht reicht in den Sa 24. hinein.
        #expect(N.counterpart(of: week[6], on: L.day(24), in: week, calendar: L.cal)?.title == "Nachtschicht")
        // Am So 25. liegt nichts.
        #expect(N.counterpart(of: week[5], on: L.day(25), in: week, calendar: L.cal) == nil)
        // Vom Vortag hereinreichend zählt ab Mitternacht: Workshop 10 Uhr → Sa 24.
        let saturday = week + [L.timed("Brunch", 24, 11, to: 12)]
        #expect(N.counterpart(of: week[5], on: L.day(24), in: saturday, calendar: L.cal)?.title == "Brunch")
        #expect(N.counterpart(of: L.timed("Nachts", 22, 1, to: 2), on: L.day(24), in: saturday, calendar: L.cal)?.title == "Nachtschicht")
    }

    @Test func selectedDayFollowsHighlight() {
        let week = CalendarViewMode.week
        // Markierung wandert auf den Workshop am Fr: Tag springt von Di auf Fr.
        #expect(week.selection(following: self.week[5], from: L.day(20), calendar: L.cal) == L.day(23))
        // Beginnt vor der Woche (So 18.): erster sichtbarer Tag.
        let early = L.allDay("Reise", 18, through: 21)
        #expect(week.selection(following: early, from: L.day(23), calendar: L.cal) == L.day(19))
        // Monat: Tag aus dem September im Oktober-Raster würde blättern, bleibt also.
        #expect(CalendarViewMode.month.selection(following: L.timed("Herbst", 29, 9, to: 10, month: 9), from: L.day(23), calendar: L.cal) == L.day(23))
        #expect(CalendarViewMode.month.selection(following: self.week[1], from: L.day(23), calendar: L.cal) == L.day(21))
        // Jahr: keine Termine, Tag bleibt.
        #expect(CalendarViewMode.year.selection(following: self.week[1], from: L.day(23), calendar: L.cal) == L.day(23))
    }

    @Test func entryWithoutHighlightStartsAtSelectedDay() {
        #expect(N.entry(in: week, forward: true, day: L.day(21), calendar: L.cal)?.title == "Frühstück")
        #expect(N.entry(in: week, forward: false, day: L.day(21), calendar: L.cal)?.title == "Mittag")
        // Messe (Do–Fr) liegt am Fr noch an.
        #expect(N.entry(in: week, forward: true, day: L.day(23), calendar: L.cal)?.title == "Messe")
        // Nach dem letzten Termin bzw. vor dem ersten: das jeweilige Ende.
        #expect(N.entry(in: week, forward: true, day: L.day(25), calendar: L.cal)?.title == "Nachtschicht")
        #expect(N.entry(in: week, forward: false, day: L.day(19), calendar: L.cal)?.title == "Bereitschaft")
        #expect(N.entry(in: [], forward: true, day: L.day(19), calendar: L.cal) == nil)
    }
}

@Suite("Tastatur: Markierung wandern")
struct OccurrenceNavigationTests {
    typealias L = LayoutFixtures
    typealias N = OccurrenceNavigation

    let list = [
        L.timed("Sport", 23, 11, to: 12),
        L.timed("Team-Sync", 23, 9, to: 9, 30),
        L.allDay("Bereitschaft", 23),
        L.timed("Workshop", 24, 10, to: 11),
    ]

    @Test func orderAllDayFirstThenByStart() {
        #expect(N.ordered(list).map(\.title) == ["Bereitschaft", "Team-Sync", "Sport", "Workshop"])
        // Gleicher Beginn: kürzere zuerst, dann Titel.
        let same = [L.timed("B", 23, 9, to: 11), L.timed("C", 23, 9, to: 10), L.timed("A", 23, 9, to: 10)]
        #expect(N.ordered(same).map(\.title) == ["A", "C", "B"])
    }

    @Test func withoutHighlightStartsAtEnds() {
        #expect(N.neighbor(of: nil, in: list, forward: true)?.title == "Bereitschaft")
        #expect(N.neighbor(of: nil, in: list, forward: false)?.title == "Workshop")
        #expect(N.neighbor(of: nil, in: [], forward: true) == nil)
    }

    @Test func walksAndStopsAtEnds() {
        let sport = list[0]
        #expect(N.neighbor(of: sport, in: list, forward: true)?.title == "Workshop")
        #expect(N.neighbor(of: sport, in: list, forward: false)?.title == "Team-Sync")
        let workshop = list[3]
        #expect(N.neighbor(of: workshop, in: list, forward: true)?.title == "Workshop")
        let first = list[2]
        #expect(N.neighbor(of: first, in: list, forward: false)?.title == "Bereitschaft")
    }

    @Test func outsideListUsesPositionInOrder() {
        // In der Seitenleiste markiert, im Raster nicht sichtbar (z. B. 23.10. 10 Uhr).
        let elsewhere = L.timed("Anderswo", 23, 10, to: 10, 30)
        #expect(N.neighbor(of: elsewhere, in: list, forward: true)?.title == "Sport")
        #expect(N.neighbor(of: elsewhere, in: list, forward: false)?.title == "Team-Sync")
        let later = L.timed("Später", 30, 9, to: 10)
        #expect(N.neighbor(of: later, in: list, forward: true)?.title == "Workshop")
        #expect(N.neighbor(of: later, in: list, forward: false)?.title == "Workshop")
    }

    @Test func refreshFindsSameIdOrNil() {
        var moved = list[0]
        moved.start = moved.start.addingTimeInterval(3600)
        #expect(N.refreshed(list[0], in: [moved])?.start == moved.start)
        #expect(N.refreshed(list[0], in: [list[1]]) == nil)
    }
}

@Suite("Tastatur: Belegung")
struct CalendarKeyCommandTests {
    @Test func mapping() {
        #expect(CalendarKeyCommand(key: .down) == .highlightNext)
        #expect(CalendarKeyCommand(key: .tab) == .highlightNext)
        #expect(CalendarKeyCommand(key: .up) == .highlightPrevious)
        #expect(CalendarKeyCommand(key: .tab, shift: true) == .highlightPrevious)
        #expect(CalendarKeyCommand(key: .backTab) == .highlightPrevious)
        #expect(CalendarKeyCommand(key: .left) == .previousDay)
        #expect(CalendarKeyCommand(key: .right) == .nextDay)
        #expect(CalendarKeyCommand(key: .enter) == .open)
        #expect(CalendarKeyCommand(key: .space) == .open)
        #expect(CalendarKeyCommand(key: .delete) == .delete)
        #expect(CalendarKeyCommand(key: .escape) == .clearHighlight)
    }
}

@Suite("Kontextmenü: Aktionen am Termin")
struct OccurrenceActionsTests {
    typealias D = DraftFixtures
    typealias F = SyncFixtures
    typealias A = OccurrenceActions

    let calendars: [String: CalendarInfo] = [
        "cal_1": CalendarInfo(id: "cal_1", accountId: "acc_1", name: "Privat", position: 0),
        "cal_2": CalendarInfo(id: "cal_2", accountId: "acc_1", name: "Arbeit", position: 1),
        "cal_3": CalendarInfo(id: "cal_3", accountId: "acc_2", name: "Ausgeblendet", hidden: true, position: 2),
        "cal_ics": CalendarInfo(id: "cal_ics", accountId: "acc_3", name: "Feiertage", writable: false, position: 3),
        "cal_0": CalendarInfo(id: "cal_0", accountId: "acc_2", name: "Familie", position: 0),
    ]

    @Test func singleEventDeletesWithoutAsking() {
        let (occ, e) = D.single()
        #expect(A.deleteScopes(occ).isEmpty)
        #expect(A.deleteIntent(occ) == .delete(eventId: "ev_1"))
        #expect(A.deleteIntent(occ, scope: .this) == .delete(eventId: "ev_1"))
        #expect(A.deleteIntent(occ) == D.draft(occ, e).deleteIntent(scope: .all))
    }

    @Test func seriesAsksForScopeLikeEditor() throws {
        let e = D.series()
        let occ = try #require(D.occurrence(of: e).dropFirst().first) // Mo 9.11.
        #expect(A.deleteScopes(occ) == D.draft(occ, e).deleteScopes)
        for scope in [EditScope.this, .thisAndFollowing, .all] {
            #expect(A.deleteIntent(occ, scope: scope) == D.draft(occ, e).deleteIntent(scope: scope))
        }
        #expect(A.deleteIntent(occ, scope: .this) == .delete(eventId: "ev_s", scope: .this, occurrence: occ.recurrenceId))
    }

    @Test func moveTargetsAreOtherWritableVisibleCalendars() {
        let (occ, _) = D.single()
        #expect(A.moveTargets(for: occ, calendars: calendars).map(\.id) == ["cal_0", "cal_2"])
        // Aus einem Abo lässt sich nichts verschieben.
        var ics = occ
        ics.calendarId = "cal_ics"
        #expect(A.moveTargets(for: ics, calendars: calendars).isEmpty)
    }

    @Test func moveAlwaysForWholeEvent() throws {
        let e = D.series()
        let occ = try #require(D.occurrence(of: e).first)
        #expect(A.moveIntent(occ, toCalendar: "cal_2") == .update(eventId: "ev_s", scope: .all, changes: EventChanges(calendarId: "cal_2")))
        #expect(A.moveIntent(occ, toCalendar: occ.calendarId) == nil)
    }

    @Test func duplicateFallsBackToWritableCalendar() {
        let (occ, _) = D.single()
        let writable = [calendars["cal_0"]!, calendars["cal_1"]!]
        #expect(A.duplicateCalendar(for: occ, calendars: calendars, writable: writable, storedDefault: nil) == "cal_1")
        var ics = occ
        ics.calendarId = "cal_ics"
        #expect(A.duplicateCalendar(for: ics, calendars: calendars, writable: writable, storedDefault: nil) == "cal_0")
        #expect(A.duplicateCalendar(for: ics, calendars: calendars, writable: [], storedDefault: nil) == nil)
    }

    /// LIC-53: Aus einem Abo landet die Kopie im Standardkalender, nicht im
    /// ersten beschreibbaren; der eigene beschreibbare Kalender geht vor.
    @Test func duplicateFromReadOnlyUsesDefaultCalendar() {
        let (occ, _) = D.single()
        let writable = [calendars["cal_0"]!, calendars["cal_1"]!, calendars["cal_2"]!]
        var ics = occ
        ics.calendarId = "cal_ics"
        #expect(A.duplicateCalendar(for: ics, calendars: calendars, writable: writable, storedDefault: "cal_2") == "cal_2")
        #expect(A.duplicateCalendar(for: occ, calendars: calendars, writable: writable, storedDefault: "cal_2") == "cal_1")
        // Wahl nicht (mehr) beschreibbar oder leer: erster beschreibbarer.
        #expect(A.duplicateCalendar(for: ics, calendars: calendars, writable: writable, storedDefault: "cal_ics") == "cal_0")
        #expect(A.duplicateCalendar(for: ics, calendars: calendars, writable: writable, storedDefault: "") == "cal_0")
        // Kalender der Kopie unbekannt (gelöscht, LIC-50 Einfügen): wie Abo.
        var gone = occ
        gone.calendarId = "cal_weg"
        #expect(A.duplicateCalendar(for: gone, calendars: calendars, writable: writable, storedDefault: "cal_2") == "cal_2")
    }

    @Test func duplicateNextWeekKeepsWallClock() {
        let (occ, _) = D.single()
        guard case let .create(cal, input) = A.duplicateIntent(occ, shiftedByDays: 7, calendarId: "cal_2") else {
            Issue.record("keine Anlage")
            return
        }
        #expect(cal == "cal_2")
        #expect(input.start == F.at(10, 9))
        #expect(input.end == F.at(10, 10))
        #expect(input.recurrence == nil)
    }
}

@Suite("Kopieren und Einfügen")
struct EventClipboardTests {
    typealias L = LayoutFixtures

    @Test func textIsReadable() {
        var occ = L.timed("Team-Sync", 23, 9, to: 9, 30)
        occ.details.location = "Raum 2"
        #expect(EventClipboard(occ, calendar: L.cal).text == "Team-Sync\nFr 23.10.2026, 9:00–9:30\nRaum 2")
        #expect(EventClipboard(L.allDay("Urlaub", 23), calendar: L.cal).text == "Urlaub\nFr 23.10.2026, ganztägig")
        #expect(EventClipboard(L.allDay("Messe", 23, through: 25), calendar: L.cal).text == "Messe\nFr 23.10.2026, ganztägig bis 25.10.2026")
    }

    @Test func pastesOnlyWhileClipboardHoldsTheCopy() {
        let clip = EventClipboard(L.timed("Team-Sync", 23, 9, to: 9, 30), calendar: L.cal)
        #expect(clip.matches(clip.text))
        #expect(!clip.matches("etwas anderes"))
        #expect(!clip.matches(nil))
    }

    @Test func pasteTitleNamesEventAndDay() {
        let clip = EventClipboard(L.timed("Team-Sync", 23, 9, to: 9, 30), calendar: L.cal)
        #expect(clip.pasteTitle(on: L.day(27)) == "„Team-Sync“ einfügen am Di 27.10.")
    }

    @Test func pasteKeepsTimeOnTargetDay() {
        let clip = EventClipboard(L.timed("Team-Sync", 23, 9, to: 9, 30), calendar: L.cal)
        #expect(clip.dayShift(to: L.day(27), calendar: L.cal) == 4)
        #expect(clip.dayShift(to: L.day(20), calendar: L.cal) == -3)
        guard case let .create(cal, input) = clip.intent(pastingOn: L.day(27), calendarId: "cal_1", calendar: L.cal) else {
            Issue.record("keine Anlage")
            return
        }
        #expect(cal == "cal_1")
        // Über die Umstellung am 25.10. hinweg bleibt die Wanduhrzeit.
        #expect(input.start == .dateTime(LocalDateTime(date: L.day(27), hour: 9, minute: 0), timeZone: "Europe/Berlin"))
        #expect(input.end == .dateTime(LocalDateTime(date: L.day(27), hour: 9, minute: 30), timeZone: "Europe/Berlin"))
    }

    @Test func pasteAllDay() {
        let clip = EventClipboard(L.allDay("Urlaub", 23, through: 24), calendar: L.cal)
        guard case let .create(_, input) = clip.intent(pastingOn: L.day(2, month: 11), calendarId: "cal_1", calendar: L.cal) else {
            Issue.record("keine Anlage")
            return
        }
        #expect(input.start == .date(L.day(2, month: 11)))
        #expect(input.end == .date(L.day(4, month: 11)))
    }
}

@Suite("Neuer Termin hier")
struct NewEventHereTests {
    typealias L = LayoutFixtures

    @Test func slotFromClickPosition() {
        let d = L.day(23)
        // Spalte 1440 hoch: ein Punkt je Minute.
        #expect(GridInteraction.slotStart(day: d, atY: 615, columnHeight: 1440) == LocalDateTime(date: d, hour: 10, minute: 15))
        #expect(GridInteraction.slotStart(day: d, atY: 0, columnHeight: 1440) == LocalDateTime(date: d, hour: 0, minute: 0))
        // Ganz unten: spätestens 23 Uhr.
        #expect(GridInteraction.slotStart(day: d, atY: 1439, columnHeight: 1440) == LocalDateTime(date: d, hour: 23, minute: 0))
        #expect(GridInteraction.slotStart(day: d, atY: nil, columnHeight: 1440) == LocalDateTime(date: d, hour: 9, minute: 0))
    }

    @Test func draftStartsAtSlot() {
        let start = LocalDateTime(date: L.day(23), hour: 14, minute: 30)
        let draft = EventDraft.new(startingAt: start, calendarId: "cal_1", deviceTimeZone: L.tz, id: "d")
        #expect(draft.start == start)
        #expect(draft.end == LocalDateTime(date: L.day(23), hour: 15, minute: 30))
        #expect(draft.isNew && draft.calendarId == "cal_1" && draft.timeZoneID == "Europe/Berlin")
        let late = EventDraft.new(startingAt: LocalDateTime(date: L.day(23), hour: 23, minute: 30), minutes: 90, calendarId: nil, deviceTimeZone: L.tz)
        #expect(late.end == LocalDateTime(date: L.day(24), hour: 1, minute: 0))
    }
}
