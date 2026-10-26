// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Mac-Hauptfenster (C): Seitenleiste mit Mini-Monat und nächsten Terminen,
/// rechts Tag, Woche, Monat oder Jahr (Spec §5).
struct MacRootView: View {
    @Environment(AppModel.self) private var model

    enum Mode: String, CaseIterable, Identifiable {
        case day = "Tag", week = "Woche", month = "Monat", year = "Jahr"
        var id: String { rawValue }
    }

    @State private var mode: Mode = .week
    // Suche (LIC-42)
    @State private var searchText = ""
    @FocusState private var searchFocused: Bool

    var body: some View {
        @Bindable var model = model
        NavigationSplitView {
            SidebarView(searchText: searchText)
                .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 360)
        } detail: {
            VStack(spacing: 0) {
                detail
                Divider()
                QuickEntryBar()
            }
            .navigationTitle(title)
            .toolbar {
                ToolbarItemGroup(placement: .navigation) {
                    Button { step(-1) } label: { Image(systemName: "chevron.left") }
                        .keyboardShortcut(.leftArrow, modifiers: .command)
                        .help("Zurück")
                    Button("Heute") { model.selectedDay = model.today }
                        .keyboardShortcut("t", modifiers: .command)
                    Button { step(1) } label: { Image(systemName: "chevron.right") }
                        .keyboardShortcut(.rightArrow, modifiers: .command)
                        .help("Weiter")
                }
                ToolbarItem(placement: .principal) {
                    Picker("Ansicht", selection: $mode) {
                        ForEach(Mode.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
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
        return mode == .year ? "\(d.year)" : MonthGrid.title(year: d.year, month: d.month)
    }

    private var weekStart: LocalDate {
        model.selectedDay.adding(days: -(model.selectedDay.weekday.rawValue - 1))
    }

    @ViewBuilder
    private var detail: some View {
        let d = model.selectedDay
        switch mode {
        case .day: DayGridView(start: d, dayCount: 1)
        case .week: DayGridView(start: weekStart, dayCount: 7)
        case .month: MonthView(year: d.year, month: d.month)
        case .year: YearView(year: d.year)
        }
    }

    private func step(_ direction: Int) {
        let d = model.selectedDay
        switch mode {
        case .day: model.selectedDay = d.adding(days: direction)
        case .week: model.selectedDay = d.adding(days: 7 * direction)
        case .month: model.selectedDay = LocalDate(year: d.year, month: d.month, day: 1).adding(months: direction)
        case .year: model.selectedDay = LocalDate(year: d.year + direction, month: d.month, day: 1)
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

/// Menüs und Tastenkürzel (Teil von LIC-24).
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
    }
}
