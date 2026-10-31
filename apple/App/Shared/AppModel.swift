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

    // MARK: Tastatur und Kontextmenüs (LIC-24)

    /// Ansicht im Mac-Hauptfenster; ⌘1–⌘4 im Menü „Darstellung“.
    var viewMode: CalendarViewMode = .week
    /// Markierter Termin (Mac): Klick, Tab, Pfeile. Getrennt von
    /// `selectedOccurrence`, das das Detail-Blatt öffnet.
    var highlightedOccurrence: Occurrence?
    /// Ob das Kalenderfeld den Tastaturfokus hat (kein Textfeld). Setzt `MacRootView`.
    var calendarHasKeyboardFocus = false
    /// Bitte an `MacRootView`, den Fokus ins Kalenderfeld zu holen (Zähler wie `searchFocusRequests`).
    var calendarFocusRequests = 0
    /// Zählt „Heute“ mit: Ist heute schon gewählt, ändert sich `selectedDay` nicht,
    /// und das Tagesband muss trotzdem zurückkommen (LIC-67).
    var todayRequests = 0
    /// Serientermin, für den die Frage nach dem Geltungsbereich beim Löschen offen ist.
    var pendingDelete: Occurrence?
    /// Zuletzt kopierter Termin (⌘C, Kontextmenü „Kopieren“).
    var clipboard: EventClipboard?

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
            refreshHighlight() // LIC-24
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
        guard let calendarId = entry.calendarID ?? defaultCalendarId else { return }
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

    /// Kopie, optional um `days` Tage verschoben. Aus einem schreibgeschützten
    /// Kalender (Abo) landet sie im Standardkalender (LIC-24, LIC-53).
    func duplicate(_ occ: Occurrence, days: Int = 0) async {
        guard let calendarId = duplicateCalendarId(for: occ) else { return }
        await engine.perform(OccurrenceActions.duplicateIntent(occ, shiftedByDays: days, calendarId: calendarId), label: "Termin duplizieren")
    }

    func undo() async { await engine.undo() }
    func redo() async { await engine.redo() }

    func refresh() async {
        _ = try? await engine.sync()
    }

    // MARK: Raster (LIC-22, LIC-42)

    /// Zweite Zeitzone als schmale Spalte im Raster; `nil` blendet sie aus.
    /// Gewählt wird sie in den Einstellungen (LIC-43, `@AppStorage("secondaryTimeZone")`);
    /// `SessionRootView` reicht Änderungen weiter.
    var secondaryTimeZone: TimeZone? = TimeZoneCatalog.zone(fromStored: UserDefaults.standard.string(forKey: "secondaryTimeZone") ?? "")

    /// Ob sich der Termin im Raster ziehen lässt (nicht ganztägig, Kalender beschreibbar).
    func canDrag(_ occ: Occurrence) -> Bool {
        GridInteraction.isDraggable(occ, calendar: state.calendars[occ.calendarId])
    }

    /// Verschieben oder Länge ändern aus dem Raster. Bei Serien nach der
    /// Rückfrage `.this` oder `.thisAndFollowing` (LIC-47, `GridInteraction.moveScopes`).
    /// Über `perform`, damit Rückgängig geht. `false`, wenn sich nichts ändert.
    @discardableResult
    func move(_ occ: Occurrence, start: Date, end: Date, scope: EditScope = .this) async -> Bool {
        guard end >= start, start != occ.start || end != occ.end,
              let intent = GridInteraction.moveIntent(occ, to: DateInterval(start: start, end: end), scope: scope, deviceTimeZone: timeZone)
        else { return false }
        let sameLength = end.timeIntervalSince(start) == occ.end.timeIntervalSince(occ.start)
        await engine.perform(intent, label: sameLength ? "Termin verschieben" : "Termindauer ändern")
        return true
    }

    /// Kalender für neue Termine: Wahl aus den Einstellungen
    /// (`@AppStorage(DefaultCalendar.storageKey)`), sonst der erste beschreibbare (LIC-47).
    var defaultCalendarId: String? {
        DefaultCalendar.choose(stored: UserDefaults.standard.string(forKey: DefaultCalendar.storageKey), writable: writableCalendars)
    }

    /// Kalender für eine Kopie: der eigene oder, wenn der schreibgeschützt
    /// ist, der Standardkalender (LIC-53).
    func duplicateCalendarId(for occ: Occurrence) -> String? {
        OccurrenceActions.duplicateCalendar(
            for: occ, calendars: state.calendars, writable: writableCalendars,
            storedDefault: UserDefaults.standard.string(forKey: DefaultCalendar.storageKey)
        )
    }

    /// Standarddauer neuer Termine aus den Einstellungen
    /// (`@AppStorage(NewEventDuration.storageKey)`, LIC-53).
    var newEventMinutes: Int {
        NewEventDuration.minutes(stored: UserDefaults.standard.integer(forKey: NewEventDuration.storageKey))
    }

    /// Entwurf nach einem Zug in leerer Fläche (LIC-47). Das Raster öffnet
    /// damit den Editor (`editorDraft`), statt sofort anzulegen.
    func newEventDraft(spanning interval: DateInterval) -> EventDraft {
        EventDraft.new(spanning: interval, calendarId: defaultCalendarId, deviceTimeZone: timeZone)
    }

    /// Entwurf nach einem Doppelklick in leere Fläche (Mac, LIC-47): Beginn
    /// an der Stelle, Standarddauer aus den Einstellungen (LIC-53).
    func newEventDraft(startingAt start: LocalDateTime) -> EventDraft {
        EventDraft.new(startingAt: start, minutes: newEventMinutes, calendarId: defaultCalendarId, deviceTimeZone: timeZone)
    }

    // MARK: Suche und Rückgängig (LIC-42)

    /// Kopie des Rückgängig-Verlaufs für Knöpfe und Beschriftung
    /// („Rückgängig: Termin löschen“). Folgt `observeUndo()`.
    private(set) var undoHistory = UndoHistory()

    /// Mac: ⌘F setzt den Fokus ins Suchfeld der Toolbar. Die Ansicht
    /// beobachtet den Zähler, weil das Menü nicht an ihren Fokus kommt.
    var searchFocusRequests = 0

    /// Liest nach jedem neuen Stand den Verlauf nach. Er ändert sich nur
    /// zusammen mit einem Stand (Anlegen, Ändern, Rückgängig), und der Engine
    /// beendet `perform`/`undo` vollständig, bevor diese Abfrage drankommt.
    func observeUndo() async {
        for await _ in await engine.updates() {
            undoHistory = await engine.undoHistory
        }
    }

    /// Treffer der Terminsuche in den angezeigten Kalendern.
    func search(_ text: String) -> [EventSearch.Hit] {
        query.search(text, selection: calendarSelection, now: Date())
    }

    /// Springt auf heute; zählt `todayRequests` hoch, damit das Tagesband
    /// auch dann zurückkommt, wenn heute schon gewählt war (LIC-67).
    func goToToday() {
        selectedDay = today
        todayRequests += 1
    }

    /// Springt zum Tag eines Vorkommens und öffnet sein Detail.
    /// Am Mac bleibt er danach markiert (LIC-24).
    func show(_ occurrence: Occurrence) {
        selectedDay = LocalDate(occurrence.start, calendar: calendar)
        open(occurrence)
    }

    // MARK: Editor (LIC-20)

    /// Offener Termin-Editor (Sheet in `RootView`).
    var editorDraft: EventDraft?
    /// Editor, der aufgeht, sobald die Detailansicht geschlossen ist: Zwei
    /// Sheets gleichzeitig zeigt SwiftUI nicht.
    private var pendingEditorDraft: EventDraft?

    /// Neuer Termin am ausgewählten Tag im Standardkalender (LIC-47) mit der
    /// Standarddauer (LIC-53).
    func newEvent() {
        editorDraft = EventDraft.new(on: selectedDay, now: Date(), calendarId: defaultCalendarId, deviceTimeZone: timeZone)
            .lasting(minutes: newEventMinutes)
    }

    /// Editor für ein Vorkommen öffnen (aus der Detailansicht: nach deren Schließen).
    func edit(_ occ: Occurrence) {
        let event = state.visibleEvents(deviceTimeZone: timeZone)[occ.eventId]
        let draft = EventDraft(editing: occ, event: event, deviceTimeZone: timeZone)
        if selectedOccurrence != nil {
            pendingEditorDraft = draft
            selectedOccurrence = nil
        } else {
            editorDraft = draft
        }
    }

    /// `onDismiss` der Detailansicht.
    func detailDismissed() {
        guard let draft = pendingEditorDraft else { return }
        pendingEditorDraft = nil
        editorDraft = draft
    }

    /// Nur Termine in beschreibbaren Kalendern lassen sich bearbeiten.
    func canEdit(_ occ: Occurrence) -> Bool {
        state.calendars[occ.calendarId]?.writable == true
    }

    /// Kalender zur Auswahl im Editor: die beschreibbaren, dazu der eigene
    /// des Termins, falls er ausgeblendet ist.
    func editorCalendars(for draft: EventDraft) -> [CalendarInfo] {
        var list = writableCalendars
        for id in [draft.calendarId, draft.original?.calendarId].compactMap({ $0 }) where !list.contains(where: { $0.id == id }) {
            if let cal = state.calendars[id], cal.writable { list.append(cal) }
        }
        return list
    }

    func save(_ draft: EventDraft, scope: EditScope) async {
        guard let intent = draft.saveIntent(scope: scope) else { return }
        await engine.perform(intent, label: draft.saveLabel)
    }

    func delete(_ draft: EventDraft, scope: EditScope) async {
        guard let intent = draft.deleteIntent(scope: scope) else { return }
        await engine.perform(intent, label: "Termin löschen")
    }
}

// MARK: Tastatur und Kontextmenüs (LIC-24)

extension AppModel {
    /// Sichtbarer Bereich der Mac-Ansicht (`nil` im Jahr).
    var visibleSpan: DaySpan? { viewMode.visibleDays(around: selectedDay) }

    /// Termine der Mac-Ansicht in der Reihenfolge, in der die Markierung wandert.
    var visibleOccurrences: [Occurrence] {
        guard let span = visibleSpan else { return [] }
        return OccurrenceNavigation.ordered(occurrences(from: span.start, days: span.count))
    }

    /// Ziel für das Menü „Termin“ und die Tasten: die Markierung, solange das
    /// Kalenderfeld den Fokus hat und kein Blatt offen ist. Mit Fokus in einem
    /// Textfeld sind die Menüpunkte aus, damit ⌘⌫ dort weiter Text löscht.
    var commandTarget: Occurrence? {
        guard calendarCommandsEnabled else { return nil }
        return highlightedOccurrence
    }

    /// Ob Menüpunkte mit Kürzeln, die auch Textfelder kennen (⌘←/⌘→, ⌘⌫),
    /// fürs Kalenderfeld gelten: Fokus dort und kein Blatt offen (LIC-50).
    var calendarCommandsEnabled: Bool {
        calendarHasKeyboardFocus && selectedOccurrence == nil && editorDraft == nil
    }

    /// Einen Schritt der Ansicht vor oder zurück (⌘←/⌘→ im Menü „Darstellung“).
    /// Eine Markierung außerhalb des neuen Bereichs fällt weg.
    func step(_ direction: Int) {
        selectedDay = viewMode.stepped(selectedDay, by: direction)
        if let h = highlightedOccurrence, OccurrenceNavigation.refreshed(h, in: visibleOccurrences) == nil {
            highlightedOccurrence = nil
        }
    }

    /// Klick auf einen Termin (Mac): markieren, Fokus ins Kalenderfeld.
    func highlight(_ occ: Occurrence) {
        highlightedOccurrence = occ
        calendarFocusRequests += 1
    }

    /// Klick in leere Fläche: Markierung weg. Am Mac wird der Tag gewählt
    /// (Ziel für Einfügen); am iPhone nicht, dort beginnt das Raster mit dem Tag.
    func tapEmptyArea(day: LocalDate) {
        highlightedOccurrence = nil
        #if os(macOS)
        if selectedDay != day { selectedDay = day }
        calendarFocusRequests += 1
        #endif
    }

    /// Detail öffnen. Am Mac bleibt der Termin danach markiert.
    func open(_ occ: Occurrence) {
        #if os(macOS)
        highlightedOccurrence = occ
        #endif
        selectedOccurrence = occ
    }

    /// Markierung aus der Tastatur wandern lassen. `false`, wenn nichts zu markieren ist.
    /// Ohne Markierung beginnt sie am ausgewählten Tag; der Tag folgt ihr, ohne
    /// zu blättern (LIC-50).
    @discardableResult
    func highlightNeighbor(forward: Bool) -> Bool {
        let list = visibleOccurrences
        let next: Occurrence?
        if let current = highlightedOccurrence {
            next = OccurrenceNavigation.neighbor(of: current, in: list, forward: forward)
        } else {
            next = OccurrenceNavigation.entry(in: list, forward: forward, day: selectedDay, calendar: calendar)
        }
        guard let next else { return false }
        highlightedOccurrence = next
        let day = viewMode.selection(following: next, from: selectedDay, calendar: calendar)
        if day != selectedDay { selectedDay = day }
        return true
    }

    /// ←/→ im Kalenderfeld (LIC-50): der ausgewählte Tag wandert einen Tag,
    /// in der Woche also von Spalte zu Spalte. Am Rand der Woche (des Monats)
    /// blättert die Ansicht mit, weil sie dem Tag folgt. Eine Markierung
    /// wandert seitwärts zum Termin dieses Tags mit der nächsten Uhrzeit.
    func moveSelectedDay(forward: Bool) {
        let day = viewMode.movedSelection(selectedDay, forward: forward)
        selectedDay = day
        if let current = highlightedOccurrence {
            highlightedOccurrence = OccurrenceNavigation.counterpart(of: current, on: day, in: visibleOccurrences, calendar: calendar)
        }
    }

    /// Taste im Kalenderfeld. `false` heißt: nicht verbraucht (weiterreichen).
    @discardableResult
    func perform(_ command: CalendarKeyCommand) -> Bool {
        switch command {
        case .highlightNext:
            return highlightNeighbor(forward: true)
        case .highlightPrevious:
            return highlightNeighbor(forward: false)
        case .open:
            guard let occ = highlightedOccurrence else { return false }
            open(occ)
            return true
        case .delete:
            guard let occ = highlightedOccurrence, canEdit(occ) else { return false }
            requestDelete(occ)
            return true
        case .clearHighlight:
            guard highlightedOccurrence != nil else { return false }
            highlightedOccurrence = nil
            return true
        case .previousDay:
            moveSelectedDay(forward: false)
            return true
        case .nextDay:
            moveSelectedDay(forward: true)
            return true
        }
    }

    /// Nach jedem neuen Stand: Markierung auf den frischen Termin setzen
    /// (z. B. nach dem Verschieben), gelöschte fallen weg.
    func refreshHighlight() {
        guard let h = highlightedOccurrence else { return }
        let day = LocalDate(h.start, calendar: calendar).adding(days: -1)
        let around = occurrences(from: day, days: 3)
        highlightedOccurrence = OccurrenceNavigation.refreshed(h, in: visibleOccurrences)
            ?? OccurrenceNavigation.refreshed(h, in: around)
    }

    // MARK: Löschen

    /// Löschen aus Kontextmenü, Menü oder ⌫: Einzeltermine sofort (Rückgängig
    /// geht), bei Serien erst nach der Frage nach dem Geltungsbereich.
    func requestDelete(_ occ: Occurrence) {
        guard canEdit(occ) else { return }
        if OccurrenceActions.deleteScopes(occ).isEmpty {
            Task { await delete(occ, scope: .all) }
        } else {
            pendingDelete = occ
        }
    }

    func delete(_ occ: Occurrence, scope: EditScope) async {
        if highlightedOccurrence?.id == occ.id { highlightedOccurrence = nil }
        await engine.perform(OccurrenceActions.deleteIntent(occ, scope: scope), label: "Termin löschen")
    }

    /// Für `confirmationDialog(isPresented:)`.
    var isConfirmingDelete: Bool {
        get { pendingDelete != nil }
        set { if !newValue { pendingDelete = nil } }
    }

    // MARK: Kalender wechseln

    func moveTargets(for occ: Occurrence) -> [CalendarInfo] {
        OccurrenceActions.moveTargets(for: occ, calendars: state.calendars)
    }

    func move(_ occ: Occurrence, toCalendar calendarId: String) async {
        guard let intent = OccurrenceActions.moveIntent(occ, toCalendar: calendarId) else { return }
        // Beim Wechsel vergibt der Server eine neue ID; die Markierung fällt weg.
        if highlightedOccurrence?.id == occ.id { highlightedOccurrence = nil }
        await engine.perform(intent, label: "Kalender wechseln")
    }

    // MARK: Neuer Termin hier

    /// Editor für einen neuen Termin am Tag `day`, mit Beginn `start` (Raster)
    /// oder der Vorgabe des Editors (Monat).
    func newEvent(on day: LocalDate, at start: LocalDateTime?) {
        let calendarId = defaultCalendarId
        if let start {
            editorDraft = EventDraft.new(startingAt: start, minutes: newEventMinutes, calendarId: calendarId, deviceTimeZone: timeZone)
        } else {
            editorDraft = EventDraft.new(on: day, now: Date(), calendarId: calendarId, deviceTimeZone: timeZone)
                .lasting(minutes: newEventMinutes)
        }
    }

    // MARK: Kopieren und Einfügen

    /// Merkt die Kopie und gibt den Text für die Zwischenablage zurück.
    func copy(_ occ: Occurrence) -> String {
        let clip = EventClipboard(occ, calendar: calendar)
        clipboard = clip
        return clip.text
    }

    /// Einfügen am Tag `day` (Standard: ausgewählter Tag), wenn die
    /// Zwischenablage noch die Kopie hält (`pasteboardText`). Hält sie etwas
    /// anderes, gilt die Kopie nicht mehr; „Einfügen“ verschwindet (LIC-50).
    func paste(on day: LocalDate? = nil, pasteboardText: String?) async {
        guard let clip = clipboard else { return }
        guard clip.matches(pasteboardText) else {
            clipboard = nil
            return
        }
        guard let calendarId = duplicateCalendarId(for: clip.occurrence) else { return }
        let intent = clip.intent(pastingOn: day ?? selectedDay, calendarId: calendarId, calendar: calendar)
        await engine.perform(intent, label: "Termin einfügen")
    }

    /// Kopie verwerfen, wenn die Zwischenablage inzwischen etwas anderes
    /// hält, z. B. beim Aktivwerden der App (iPhone, LIC-54). So zeigt die
    /// Toolbar „Einfügen“ nur, solange es noch geht.
    func expireClipboard(pasteboardText: String?) {
        if let clip = clipboard, !clip.matches(pasteboardText) { clipboard = nil }
    }
}
