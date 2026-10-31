// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import Combine
import LichtungKit
import SwiftUI
import UIKit

/// iPhone: Liste (A) als Start, dazu Raster (B) und Monat (D) (Spec §5).
struct PhoneRootView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase

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
                        Button("Heute") { model.goToToday() }
                    }
                    // Erst nach „Kopieren“ zu sehen (LIC-50).
                    ToolbarItem(placement: .topBarLeading) { pasteMenu }
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
                .invitationsButton() // Terminanfragen (LIC-30)
                .safeAreaInset(edge: .bottom) {
                    QuickEntryBar().background(.bar)
                }
        }
        .sheet(item: $model.selectedOccurrence, onDismiss: { model.detailDismissed() }) { occ in
            EventDetailView(occurrence: occ)
        }
        .onShake { if model.undoHistory.canUndo || model.undoHistory.canRedo { showsUndoAlert = true } }
        // Wer in einer anderen App etwas kopiert hat, sieht „Einfügen“ nicht mehr (LIC-54).
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { model.expireClipboard(pasteboardText: Pasteboard.string) }
        }
        // Auch während die App offen ist, z. B. Text im Editor kopiert (LIC-58).
        // Erst nach dem laufenden Durchlauf prüfen: Kopiert Lichtung selbst,
        // kommt die Mitteilung schon beim Setzen der Zwischenablage, bevor
        // `Pasteboard.write` den neuen Stand gemerkt hat; sofort geprüft,
        // verfiele die eigene, gerade gemachte Kopie.
        .onReceive(NotificationCenter.default.publisher(for: UIPasteboard.changedNotification)) { _ in
            let appModel: AppModel = model
            Task { @MainActor in appModel.expireClipboard(pasteboardText: Pasteboard.string) }
        }
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

    /// Nach „Kopieren“ (Kontextmenü, Detail): Antippen fügt die Kopie am
    /// ausgewählten Tag ein, lange drücken bietet auch „Kopie verwerfen“.
    /// Die Logik kommt aus dem Kern (`EventClipboard`, LIC-50). Hat sich die
    /// Zwischenablage inzwischen geändert, verfällt die Kopie beim
    /// Aktivwerden der App (`PasteboardMark`, LIC-54).
    @ViewBuilder
    var pasteMenu: some View {
        if let clip = model.clipboard {
            let title: String = clip.pasteTitle(on: model.selectedDay)
            Menu {
                Button(title, systemImage: "doc.on.clipboard") { paste() }
                Button("Kopie verwerfen", systemImage: "xmark", role: .destructive) { model.clipboard = nil }
            } label: {
                Image(systemName: "doc.on.clipboard")
            } primaryAction: {
                paste()
            }
            .accessibilityLabel(title)
        }
    }

    func paste() {
        let text = Pasteboard.string
        Task { await model.paste(pasteboardText: text) }
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
