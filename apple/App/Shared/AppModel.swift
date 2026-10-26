// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import Foundation
import LichtungKit
import Observation

/// Zustand der App für alle Ansichten. Hält den `SyncEngine` und eine Kopie
/// des Stands auf dem Main Actor.
@MainActor
@Observable
final class AppModel {
    let engine: SyncEngine
    let timeZone: TimeZone
    let calendar: Calendar
    private(set) var state = SyncState()
    /// Ausgewählter Tag (Tagesleiste, Mini-Monat, Raster).
    var selectedDay: LocalDate
    var selectedOccurrence: Occurrence?
    var calendarSelection: CalendarQuery.Selection = .visible

    init(engine: SyncEngine, timeZone: TimeZone = .current) {
        self.engine = engine
        self.timeZone = timeZone
        self.calendar = .lichtung(timeZone: timeZone)
        self.selectedDay = LocalDate(Date(), calendar: .lichtung(timeZone: timeZone))
    }

    /// Folgt dem Stand des Engines, solange die Ansicht lebt (`.task`).
    func observe() async {
        for await snapshot in await engine.updates() {
            state = snapshot
        }
    }

    /// App ohne Server: erfundene Termine, Änderungen bleiben lokal in der Outbox.
    static func demo() -> AppModel {
        let tz = TimeZone.current
        let store = InMemoryCacheStore(DemoData.state(around: Date(), timeZone: tz))
        let api = SyncAPI(baseURL: URL(string: "https://lichtung.invalid")!, tokens: StaticToken(nil))
        return AppModel(engine: SyncEngine(api: api, store: store, deviceTimeZone: tz), timeZone: tz)
    }

    var today: LocalDate { LocalDate(Date(), calendar: calendar) }

    var query: CalendarQuery { CalendarQuery(state: state, deviceTimeZone: timeZone) }

    func occurrences(from start: LocalDate, days: Int) -> [Occurrence] {
        let window = DateInterval(start: start.startDate(in: calendar), end: start.adding(days: days).startDate(in: calendar))
        return query.occurrences(in: window, selection: calendarSelection)
    }

    func colorHex(for calendarId: String) -> String? {
        state.calendars[calendarId]?.displayColor
    }

    var writableCalendars: [CalendarInfo] {
        state.calendars.values.filter { $0.writable && !$0.hidden }.sorted { $0.position < $1.position }
    }

    // MARK: Ändern

    /// Termin aus der Schnelleingabe anlegen.
    func create(from entry: QuickEntry) async {
        guard let calendarId = entry.calendarID ?? writableCalendars.first?.id else { return }
        let tzID = timeZone.identifier
        let (start, end): (DateValue, DateValue)
        switch entry.timing {
        case .unspecified:
            start = .date(selectedDay)
            end = .date(selectedDay.adding(days: 1))
        case let .allDay(first, last):
            start = .date(first)
            end = .date(last.adding(days: 1))
        case let .timed(s, e):
            start = .dateTime(LocalDateTime(s, calendar: calendar), timeZone: tzID)
            end = .dateTime(LocalDateTime(e, calendar: calendar), timeZone: tzID)
        }
        let recurrence = entry.recurrence.map { Recurrence(rrule: $0.rruleString(allDay: start.isAllDay, timeZone: timeZone)) }
        let input = EventInput(title: entry.title.isEmpty ? "Neuer Termin" : entry.title, location: entry.location,
                               start: start, end: end, recurrence: recurrence)
        await engine.perform(.create(calendarId: calendarId, event: input), label: "Termin anlegen")
    }

    func delete(_ occ: Occurrence) async {
        let scope: EditScope = occ.recurrenceId == nil ? .all : .this
        await engine.perform(.delete(eventId: occ.eventId, scope: scope, occurrence: occ.recurrenceId), label: "Termin löschen")
    }

    func duplicate(_ occ: Occurrence, days: Int = 0) async {
        await engine.perform(.create(calendarId: occ.calendarId, event: EventDuplicate.input(from: occ, shiftedByDays: days)), label: "Termin duplizieren")
    }

    func undo() async { await engine.undo() }
    func redo() async { await engine.redo() }

    func refresh() async {
        _ = try? await engine.sync()
    }

    // MARK: Raster (LIC-22, LIC-42)

    /// Zweite Zeitzone als schmale Spalte im Raster; `nil` blendet sie aus.
    /// Gesetzt wird sie in den Einstellungen (LIC-43).
    var secondaryTimeZone: TimeZone?

    /// Ob sich der Termin im Raster ziehen lässt (nicht ganztägig, Kalender beschreibbar).
    func canDrag(_ occ: Occurrence) -> Bool {
        GridInteraction.isDraggable(occ, calendar: state.calendars[occ.calendarId])
    }

    /// Verschieben oder Länge ändern aus dem Raster. Bei Serien nur dieses
    /// Vorkommen (`scope .this`). Über `perform`, damit Rückgängig geht.
    func move(_ occ: Occurrence, start: Date, end: Date) async {
        guard end >= start, start != occ.start || end != occ.end,
              let intent = GridInteraction.moveIntent(occ, to: DateInterval(start: start, end: end), deviceTimeZone: timeZone)
        else { return }
        let sameLength = end.timeIntervalSince(start) == occ.end.timeIntervalSince(occ.start)
        await engine.perform(intent, label: sameLength ? "Termin verschieben" : "Termindauer ändern")
    }

    /// Termin aus einem Zug in leerer Fläche anlegen, im ersten beschreibbaren Kalender.
    func createTimed(start: Date, end: Date) async {
        guard end > start, let calendarId = writableCalendars.first?.id else { return }
        let intent = GridInteraction.createIntent(calendarId: calendarId, interval: DateInterval(start: start, end: end), timeZone: timeZone)
        await engine.perform(intent, label: "Termin anlegen")
    }
}
