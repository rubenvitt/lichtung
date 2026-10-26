// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// iPhone: Liste (A) als Start, dazu Raster (B) und Monat (D) (Spec §5).
struct PhoneRootView: View {
    @Environment(AppModel.self) private var model

    enum Mode: String, CaseIterable, Identifiable {
        case list = "Liste", grid = "Raster", month = "Monat"
        var id: String { rawValue }
    }

    @State private var mode: Mode = .list
    @State private var gridDays = 1
    // Suche und Rückgängig (LIC-42)
    @State private var searchText = ""
    @State private var showsUndoAlert = false

    var body: some View {
        @Bindable var model = model
        NavigationStack {
            searchableContent
                .navigationTitle(MonthGrid.title(year: model.selectedDay.year, month: model.selectedDay.month))
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Heute") { model.selectedDay = model.today }
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        Menu {
                            Picker("Ansicht", selection: $mode) {
                                ForEach(Mode.allCases) { Text($0.rawValue).tag($0) }
                            }
                            if mode == .grid {
                                Picker("Tage", selection: $gridDays) {
                                    Text("Tag").tag(1)
                                    Text("3 Tage").tag(3)
                                    Text("Woche").tag(7)
                                }
                            }
                        } label: {
                            Image(systemName: "calendar")
                        }
                        .accessibilityLabel("Ansicht wählen")
                    }
                    // MARK: Editor (LIC-20)
                    ToolbarItem(placement: .topBarTrailing) {
                        Button { model.newEvent() } label: { Image(systemName: "plus") }
                            .accessibilityLabel("Neuer Termin")
                    }
                }
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) { undoMenu }
                }
                .phoneSettingsButton() // Einstellungen (LIC-43)
                .safeAreaInset(edge: .bottom) {
                    QuickEntryBar().background(.bar)
                }
        }
        .sheet(item: $model.selectedOccurrence, onDismiss: { model.detailDismissed() }) { occ in
            EventDetailView(occurrence: occ)
        }
        .onShake { if model.undoHistory.canUndo || model.undoHistory.canRedo { showsUndoAlert = true } }
        .alert(undoAlertTitle, isPresented: $showsUndoAlert) { undoAlertActions }
    }

    @ViewBuilder
    private var content: some View {
        switch mode {
        case .list:
            AgendaView()
                .refreshable { await model.refresh() }
        case .grid:
            let start = gridDays == 7
                ? model.selectedDay.adding(days: -(model.selectedDay.weekday.rawValue - 1))
                : model.selectedDay
            DayGridView(start: start, dayCount: gridDays)
                .gesture(DragGesture(minimumDistance: 40).onEnded { value in
                    let step = value.translation.width < 0 ? gridDays : -gridDays
                    model.selectedDay = model.selectedDay.adding(days: step)
                })
        case .month:
            MonthView(year: model.selectedDay.year, month: model.selectedDay.month, visibleLanes: 2)
                .gesture(DragGesture(minimumDistance: 40).onEnded { value in
                    let step = value.translation.height < 0 ? 1 : -1
                    model.selectedDay = LocalDate(year: model.selectedDay.year, month: model.selectedDay.month, day: 1).adding(months: step)
                })
        }
    }
}

// MARK: Suche und Rückgängig (LIC-42)

extension PhoneRootView {
    /// Solange im Suchfeld etwas steht, zeigt die Seite Treffer statt der
    /// Ansicht. Ein Treffer springt zum Tag und öffnet das Detail; die Suche
    /// bleibt offen, „Abbrechen“ führt zur Ansicht an diesem Tag.
    @ViewBuilder
    var searchableContent: some View {
        Group {
            if searchText.trimmingCharacters(in: .whitespaces).isEmpty {
                content
            } else {
                SearchResultsView(text: searchText) { occurrence in
                    model.show(occurrence)
                }
            }
        }
        .searchable(text: $searchText, prompt: "Termine suchen")
    }

    /// Antippen: Rückgängig. Lange drücken: Rückgängig oder Wiederholen mit
    /// Beschriftung der Aktion.
    var undoMenu: some View {
        Menu {
            Button(model.undoHistory.undoTitle) { Task { await model.undo() } }
                .disabled(!model.undoHistory.canUndo)
            Button(model.undoHistory.redoTitle) { Task { await model.redo() } }
                .disabled(!model.undoHistory.canRedo)
        } label: {
            Image(systemName: "arrow.uturn.backward")
        } primaryAction: {
            Task { await model.undo() }
        }
        .disabled(!model.undoHistory.canUndo && !model.undoHistory.canRedo)
        .accessibilityLabel(model.undoHistory.undoTitle)
    }

    /// Nach dem Schütteln: „Rückgängig: Termin löschen“, sonst Wiederholen.
    var undoAlertTitle: String {
        model.undoHistory.canUndo ? model.undoHistory.undoTitle : model.undoHistory.redoTitle
    }

    @ViewBuilder
    var undoAlertActions: some View {
        if model.undoHistory.canUndo {
            Button("Rückgängig") { Task { await model.undo() } }
        }
        if model.undoHistory.canRedo {
            Button("Wiederholen") { Task { await model.redo() } }
        }
        Button("Abbrechen", role: .cancel) {}
    }
}
