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

    /// Ansichten im Umschalter oben. Raster-Varianten sind eigene Einträge,
    /// damit jede Ansicht einen Tipp entfernt ist.
    enum Mode: String, CaseIterable, Identifiable {
        case list, day, threeDays, week, month
        var id: String { rawValue }

        var label: String {
            switch self {
            case .list: "Liste"
            case .day: "Tag"
            case .threeDays: "3 Tage"
            case .week: "Woche"
            case .month: "Monat"
            }
        }

        /// Tage im Raster; `nil` für Liste und Monat.
        var gridDays: Int? {
            switch self {
            case .day: 1
            case .threeDays: 3
            case .week: 7
            case .list, .month: nil
            }
        }
    }

    /// Zuletzt gewählte Ansicht, bleibt über Neustarts erhalten.
    @AppStorage("phoneViewMode") private var mode: Mode = .list
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
                    ToolbarItem(placement: .principal) {
                        MonthTitle(year: model.selectedDay.year, month: model.selectedDay.month, style: .headline)
                    }
                    // Erst nach „Kopieren“ zu sehen (LIC-50).
                    ToolbarItem(placement: .topBarLeading) { pasteMenu }
                    ToolbarItem(placement: .topBarTrailing) { undoMenu }
                }
                .phoneSettingsButton() // Einstellungen (LIC-43)
                .invitationsButton() // Terminanfragen (LIC-30)
                .syncProblemsButton() // Konflikte, abgelehnte Änderungen (LIC-70)
                .safeAreaInset(edge: .top, spacing: 0) {
                    if searchText.trimmingCharacters(in: .whitespaces).isEmpty {
                        modeBar
                    }
                }
                .safeAreaInset(edge: .bottom, spacing: 0) {
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

    /// Umschalter der Ansicht und „Heute“, direkt unter der Navigationsleiste.
    private var modeBar: some View {
        HStack(spacing: 10) {
            Picker("Ansicht", selection: $mode.animation(.snappy)) {
                ForEach(Mode.allCases) { Text($0.label).tag($0) }
            }
            .pickerStyle(.segmented)
            Button {
                model.goToToday()
            } label: {
                Text("Heute")
                    .font(.subheadline.weight(.semibold))
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(Capsule().fill(Theme.line))
                    .foregroundStyle(isShowingToday ? Theme.text2 : Theme.text1)
            }
            .buttonStyle(.plain)
            .accessibilityHint("Springt zum heutigen Tag")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Theme.line).frame(height: 0.5)
        }
    }

    /// Heute ist schon ausgewählt: „Heute“ tritt etwas zurück.
    private var isShowingToday: Bool { model.selectedDay == model.today }

    @ViewBuilder
    private var content: some View {
        switch mode {
        case .list:
            AgendaView()
                .refreshable { await model.refresh() }
        case .day, .threeDays, .week:
            let days = mode.gridDays ?? 1
            let start = days == 7
                ? model.selectedDay.adding(days: -(model.selectedDay.weekday.rawValue - 1))
                : model.selectedDay
            DayGridView(start: start, dayCount: days)
                // Nur ein klar waagerechter Wisch blättert; senkrecht scrollt (LIC-65).
                .gesture(DragGesture(minimumDistance: 40).onEnded { value in
                    guard let step = SwipeStep.horizontal(dx: value.translation.width, dy: value.translation.height) else { return }
                    model.page(days: step * days)
                })
        case .month:
            MonthView(year: model.selectedDay.year, month: model.selectedDay.month, visibleLanes: 2)
                .gesture(DragGesture(minimumDistance: 40).onEnded { value in
                    guard let step = SwipeStep.vertical(dx: value.translation.width, dy: value.translation.height) else { return }
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
    @ViewBuilder
    var undoMenu: some View {
        if model.undoHistory.canUndo || model.undoHistory.canRedo {
            undoMenuButton
        }
    }

    private var undoMenuButton: some View {
        Menu {
            Button(model.undoHistory.undoTitle) { Task { await model.undo() } }
                .disabled(!model.undoHistory.canUndo)
            Button(model.undoHistory.redoTitle) { Task { await model.redo() } }
                .disabled(!model.undoHistory.canRedo)
        } label: {
            Label("Rückgängig", systemImage: "arrow.uturn.backward") // Text fürs ⋯-Menü (LIC-66)
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
                Label(title, systemImage: "doc.on.clipboard") // Text fürs ⋯-Menü (LIC-66)
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
