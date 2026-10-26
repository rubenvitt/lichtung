// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import AppKit
import LichtungKit
import SwiftUI
import UniformTypeIdentifiers

/// Mac-Hauptfenster (C): Seitenleiste mit Mini-Monat und nächsten Terminen,
/// rechts Tag, Woche, Monat oder Jahr (Spec §5).
///
/// Tastatur (LIC-24): Das Kalenderfeld rechts nimmt den Fokus, sobald ein
/// Termin markiert oder leere Fläche angeklickt wird. Dann gelten ↑/↓ und
/// Tab/⇧Tab (Markierung wandern), ←/→ (Tag für Tag, LIC-50), Return/Leertaste
/// (öffnen), ⌫ (löschen), Esc (Markierung weg), ⌘C/⌘V. In Textfeldern bleiben
/// die Tasten beim Text. Belegung in `CalendarKeyCommand` (LichtungKit).
struct MacRootView: View {
    @Environment(AppModel.self) private var model

    // Suche (LIC-42)
    @State private var searchText = ""
    @FocusState private var searchFocused: Bool
    // Tastatur im Kalenderfeld (LIC-24)
    @FocusState private var calendarFocused: Bool

    var body: some View {
        @Bindable var model = model
        NavigationSplitView {
            SidebarView(searchText: searchText)
                .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 360)
        } detail: {
            VStack(spacing: 0) {
                keyboardDetail
                Divider()
                QuickEntryBar()
            }
            .navigationTitle(title)
            .toolbar {
                ToolbarItemGroup(placement: .navigation) {
                    Button { model.step(-1) } label: { Image(systemName: "chevron.left") }
                        .keyboardShortcut(.leftArrow, modifiers: .command)
                        .help("Zurück (⌘←)")
                    Button("Heute") { model.selectedDay = model.today }
                        .keyboardShortcut("t", modifiers: .command)
                    Button { model.step(1) } label: { Image(systemName: "chevron.right") }
                        .keyboardShortcut(.rightArrow, modifiers: .command)
                        .help("Weiter (⌘→)")
                }
                ToolbarItem(placement: .principal) {
                    Picker("Ansicht", selection: $model.viewMode) {
                        ForEach(CalendarViewMode.allCases) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .help("Ansicht (⌘1–⌘4)")
                }
                // MARK: Editor (LIC-20)
                ToolbarItem(placement: .primaryAction) {
                    Button { model.newEvent() } label: { Image(systemName: "plus") }
                        .help("Neuer Termin (⌘N)")
                }
            }
        }
        .sheet(item: $model.selectedOccurrence, onDismiss: { model.detailDismissed() }) { occ in
            EventDetailView(occurrence: occ).frame(minWidth: 360, minHeight: 420)
        }
        // Suche in der Toolbar, ⌘F über das Menü (LIC-42)
        .searchable(text: $searchText, placement: .toolbar, prompt: "Termine suchen")
        .searchFocused($searchFocused)
        .onChange(of: model.searchFocusRequests) { searchFocused = true }
    }

    private var title: String {
        let d = model.selectedDay
        return model.viewMode == .year ? "\(d.year)" : MonthGrid.title(year: d.year, month: d.month)
    }

    private var weekStart: LocalDate {
        model.selectedDay.adding(days: -(model.selectedDay.weekday.rawValue - 1))
    }

    @ViewBuilder
    private var detail: some View {
        let d = model.selectedDay
        switch model.viewMode {
        case .day: DayGridView(start: d, dayCount: 1)
        case .week: DayGridView(start: weekStart, dayCount: 7)
        case .month: MonthView(year: d.year, month: d.month)
        case .year: YearView(year: d.year)
        }
    }

    // MARK: Tastatur (LIC-24)

    /// Ob gerade kein Blatt (Detail, Editor) offen ist.
    private var noSheetOpen: Bool {
        model.selectedOccurrence == nil && model.editorDraft == nil
    }

    /// Kalenderfeld mit Fokus, Tasten, Kopieren und Einfügen.
    private var keyboardDetail: some View {
        detail
            .focusable(interactions: .edit)
            .focusEffectDisabled()
            .focused($calendarFocused)
            .onKeyPress(phases: [.down, .repeat]) { press in
                handleKey(press)
            }
            .onCopyCommand(perform: copyAction)
            .onPasteCommand(of: [UTType.plainText], perform: pasteAction)
            .onAppear { calendarFocused = true }
            .onChange(of: calendarFocused, initial: true) { _, focused in
                model.calendarHasKeyboardFocus = focused
            }
            .onChange(of: model.calendarFocusRequests) { calendarFocused = true }
            // Nach Detail oder Editor zurück ins Kalenderfeld.
            .onChange(of: noSheetOpen) { _, closed in
                if closed { calendarFocused = true }
            }
    }

    /// ⌘C im Kalenderfeld: Markierung als Text in die Zwischenablage und als
    /// Vorlage für ⌘V. `nil` lässt „Kopieren“ aus.
    private var copyAction: (() -> [NSItemProvider])? {
        guard let target = model.commandTarget else { return nil }
        let model = model
        return {
            let text: String = MainActor.assumeIsolated { model.copy(target) }
            return [NSItemProvider(object: text as NSString)]
        }
    }

    /// ⌘V im Kalenderfeld: die Kopie am ausgewählten Tag einfügen, solange die
    /// Zwischenablage noch ihren Text hält (`EventClipboard.matches`).
    private var pasteAction: ([NSItemProvider]) -> Void {
        let model = model
        return { _ in
            Task { @MainActor in
                await model.paste(pasteboardText: Pasteboard.string)
            }
        }
    }

    private func handleKey(_ press: KeyPress) -> KeyPress.Result {
        // Mit ⌘, ⌥ oder ⌃ gelten die Menüs (z. B. ⌘←).
        let menuModifiers: EventModifiers = [.command, .option, .control]
        guard press.modifiers.isDisjoint(with: menuModifiers) else { return .ignored }
        guard let key = Self.calendarKey(press.key),
              let command = CalendarKeyCommand(key: key, shift: press.modifiers.contains(.shift))
        else { return .ignored }
        // Gehaltene Taste: wandern und blättern ja, öffnen und löschen nur einmal.
        if press.phase == .repeat && (command == .open || command == .delete) {
            return .handled
        }
        return model.perform(command) ? .handled : .ignored
    }

    private static func calendarKey(_ key: KeyEquivalent) -> CalendarKey? {
        switch key {
        case .upArrow: return .up
        case .downArrow: return .down
        case .leftArrow: return .left
        case .rightArrow: return .right
        case .tab: return .tab
        case KeyEquivalent("\u{19}"): return .backTab // ⇧Tab kommt je nach Tastatur so an
        case .return: return .enter
        case .space: return .space
        case .escape: return .escape
        case .delete, .deleteForward: return .delete
        default: return nil
        }
    }
}

struct SidebarView: View {
    @Environment(AppModel.self) private var model
    /// Text im Suchfeld der Toolbar; nicht leer → Treffer statt nächster Termine (LIC-42).
    var searchText = ""

    var body: some View {
        @Bindable var model = model
        VStack(alignment: .leading, spacing: 8) {
            MiniMonthView(year: model.selectedDay.year, month: model.selectedDay.month)
                .padding(.horizontal, 10)
                .padding(.top, 8)
            Picker("Kalender", selection: $model.calendarSelection) {
                Text("Alle Kalender").tag(CalendarQuery.Selection.visible)
                ForEach(model.state.calendarSets.values.sorted { $0.position < $1.position }) { set in
                    Text(set.name).tag(CalendarQuery.Selection.set(set.id))
                }
            }
            .labelsHidden()
            .padding(.horizontal, 10)
            Divider()
            if searchText.trimmingCharacters(in: .whitespaces).isEmpty {
                // Ansicht A in der Seitenleiste: die nächsten Termine.
                AgendaView(days: 14, showsTicker: false)
            } else {
                // Treffer der Suche; rechts springt das Raster zum Tag (LIC-42).
                SearchResultsView(text: searchText) { occurrence in
                    model.show(occurrence)
                }
            }
        }
    }
}

/// Jahresansicht: zwölf kleine Monate.
struct YearView: View {
    let year: Int

    var body: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 180), spacing: 24)], spacing: 24) {
                ForEach(1...12, id: \.self) { month in
                    MiniMonthView(year: year, month: month)
                }
            }
            .padding(20)
        }
    }
}

/// Menüs und Tastenkürzel (LIC-24).
struct LichtungCommands: Commands {
    let model: AppModel

    var body: some Commands {
        // MARK: Editor (LIC-20)
        CommandGroup(replacing: .newItem) {
            Button("Neuer Termin") { Task { @MainActor in model.newEvent() } }
                .keyboardShortcut("n", modifiers: .command)
        }
        CommandGroup(replacing: .undoRedo) {
            Button("Widerrufen") { Task { await model.undo() } }
                .keyboardShortcut("z", modifiers: .command)
            Button("Wiederholen") { Task { await model.redo() } }
                .keyboardShortcut("z", modifiers: [.command, .shift])
        }
        // ⌘F: Suchfeld der Toolbar. Ersetzt das Text-Suchen im Menü
        // „Bearbeiten“, sonst gäbe es ⌘F zweimal (LIC-42).
        CommandGroup(replacing: .textEditing) {
            Button("Termine suchen …") { model.searchFocusRequests += 1 }
                .keyboardShortcut("f", modifiers: .command)
        }
        CommandMenu("Kalender") {
            Button("Aktualisieren") { Task { await model.refresh() } }
                .keyboardShortcut("r", modifiers: .command)
        }
        // Ansicht wechseln: ⌘1 Tag, ⌘2 Woche, ⌘3 Monat, ⌘4 Jahr (Menü „Darstellung“).
        CommandGroup(before: .toolbar) {
            ViewModeMenuItems(model: model)
        }
        // Aktionen am markierten Termin; ohne Markierung ausgegraut.
        CommandMenu("Termin") {
            EventMenuBarItems(model: model)
        }
    }
}

/// Einträge für ⌘1–⌘4. Als eigene Ansicht, damit sie dem Modell folgen.
struct ViewModeMenuItems: View {
    let model: AppModel

    var body: some View {
        ForEach(CalendarViewMode.allCases) { mode in
            Button(mode.label) { model.viewMode = mode }
                .keyboardShortcut(KeyEquivalent(mode.shortcutDigit), modifiers: .command)
        }
        Divider()
    }
}

/// Menü „Termin“: dieselben Einträge wie das Kontextmenü, mit Tastenkürzeln.
/// Ziel ist die Markierung, solange das Kalenderfeld den Fokus hat.
struct EventMenuBarItems: View {
    let model: AppModel

    var body: some View {
        OccurrenceMenuItems(model: model, occurrence: model.commandTarget, showsShortcuts: true)
        Divider()
        Button("Nächster Termin") { model.perform(.highlightNext) }
        Button("Vorheriger Termin") { model.perform(.highlightPrevious) }
        Button("Markierung aufheben") { model.perform(.clearHighlight) }
            .disabled(model.highlightedOccurrence == nil)
    }
}
