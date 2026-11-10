import Combine
import LichtungKit
import SwiftUI
import UIKit

/// iPhone: Tag als Start, dazu Woche, Monat und Liste; Umschalter und „+“ unten am Daumen (Spec §5).
struct PhoneRootView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase

    /// Ansichten im Umschalter unten.
    enum Mode: String, CaseIterable, Identifiable {
        case day, week, month, list
        var id: String { rawValue }

        /// „3 Tage“ gibt es nicht mehr; ein gespeicherter Wert wird zur Woche (Spec §5).
        init?(rawValue: String) {
            switch rawValue {
            case "day": self = .day
            case "week", "threeDays": self = .week
            case "month": self = .month
            case "list": self = .list
            default: return nil
            }
        }

        var label: String {
            switch self {
            case .day: "Tag"
            case .week: "Woche"
            case .month: "Monat"
            case .list: "Liste"
            }
        }
    }

    /// Zuletzt gewählte Ansicht, bleibt über Neustarts erhalten.
    @AppStorage("phoneViewMode") private var mode: Mode = .day
    // Suche und Rückgängig (LIC-42)
    @State private var searchText = ""
    @State private var showsUndoAlert = false
    // Schnelleingabe als Blatt hinter „+“
    @State private var showsQuickEntry = false
    @State private var opensEditorAfterQuickEntry = false
    /// Großer Titel im Kopf; wächst mit der Schriftgröße.
    @ScaledMetric(relativeTo: .largeTitle) private var titleSize: CGFloat = 32

    private var isSearching: Bool { !searchText.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        @Bindable var model = model
        NavigationStack {
            searchableContent
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) { todayButton }
                    // Erst nach „Kopieren“ zu sehen (LIC-50).
                    ToolbarItem(placement: .topBarTrailing) { pasteMenu }
                    ToolbarItem(placement: .topBarTrailing) { undoMenu }
                }
                .phoneSettingsButton() // Einstellungen (LIC-43)
                .invitationsButton() // Terminanfragen (LIC-30)
                .syncProblemsButton() // Konflikte, abgelehnte Änderungen (LIC-70)
                .safeAreaInset(edge: .top, spacing: 0) {
                    if !isSearching { header }
                }
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    if !isSearching { bottomBar }
                }
        }
        .sheet(item: $model.selectedOccurrence, onDismiss: { model.detailDismissed() }) { occ in
            EventDetailView(occurrence: occ)
        }
        .sheet(isPresented: $showsQuickEntry, onDismiss: {
            guard opensEditorAfterQuickEntry else { return }
            opensEditorAfterQuickEntry = false
            model.newEvent()
        }) {
            QuickEntryBar { opensEditor in
                opensEditorAfterQuickEntry = opensEditor
                showsQuickEntry = false
            }
            .frame(maxHeight: .infinity, alignment: .top)
            .padding(.top, 12)
            .presentationDetents([.height(260)])
            .presentationDragIndicator(.visible)
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

    // MARK: Kopf

    /// Kopf je Ansicht: Tag „Dienstag 10“ mit dem Satz zum nächsten Termin,
    /// Woche „November“ mit „KW 46“, Monat und Liste „November 2026“.
    /// Der Wochenstreifen steht in Tag und Liste.
    private var header: some View {
        VStack(alignment: .leading, spacing: 2) {
            switch mode {
            case .day:
                let day = model.selectedDay
                title(QuickEntryPreview.weekdayLong[day.weekday.rawValue - 1], .light, "\(day.day)", .semibold)
                if day == model.today {
                    // Jede Minute neu, damit „beginnt in 20 Minuten“ mitzählt.
                    TimelineView(.everyMinute) { context in
                        Text(AgendaBuilder.nextSentence(model.occurrences(from: day, days: 1), now: context.date, calendar: model.calendar))
                    }
                    .font(.subheadline)
                    .foregroundStyle(Theme.text2)
                }
            case .week:
                let day = model.focusDay
                title(MonthTitle.monthName(day.month), .semibold)
                Text(verbatim: "KW \(MonthGrid.isoWeek(day))")
                    .font(.subheadline)
                    .foregroundStyle(Theme.text2)
            case .month, .list:
                let day = model.selectedDay
                title(MonthTitle.monthName(day.month), .semibold, "\(day.year)", .light)
            }
            if mode == .day || mode == .list {
                DayTickerView()
                    .padding(.top, 8)
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.shade)
    }

    /// Großer Titel aus zwei Teilen, je mit eigener Stärke („Dienstag“ leicht, „10“ halbfett).
    private func title(_ first: String, _ firstWeight: Font.Weight, _ second: String? = nil, _ secondWeight: Font.Weight = .regular) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(first).fontWeight(firstWeight)
            if let second {
                Text(second).fontWeight(secondWeight).monospacedDigit()
            }
        }
        .font(.system(size: titleSize))
        .foregroundStyle(Theme.text1)
        .lineLimit(1)
        .minimumScaleFactor(0.6)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }

    /// „Heute“ oben links, nur wenn heute nicht zu sehen ist.
    @ViewBuilder
    private var todayButton: some View {
        if !isShowingToday {
            Button("Heute") { model.goToToday() }
                .accessibilityHint("Springt zum heutigen Tag")
        }
    }

    private var isShowingToday: Bool {
        let today = model.today
        switch mode {
        case .day, .list: return model.selectedDay == today
        case .week: return weekStart.days(until: today) >= 0 && weekStart.days(until: today) < 7
        case .month: return model.selectedDay.year == today.year && model.selectedDay.month == today.month
        }
    }

    /// Montag der Woche des Fokus-Tags; sie ist die angezeigte Woche.
    private var weekStart: LocalDate {
        model.focusDay.adding(days: -(model.focusDay.weekday.rawValue - 1))
    }

    // MARK: Leiste unten

    /// Umschalter der Ansicht in Kapselform, rechts der runde Knopf „+“ für die Schnelleingabe.
    private var bottomBar: some View {
        HStack(spacing: 10) {
            Picker("Ansicht", selection: $mode.animation(.snappy)) {
                ForEach(Mode.allCases) { Text($0.label).tag($0) }
            }
            .pickerStyle(.segmented)
            .controlSize(.large)
            Button {
                showsQuickEntry = true
            } label: {
                Image(systemName: "plus")
                    .font(.title2.weight(.light))
                    .foregroundStyle(Theme.onLight)
                    .frame(width: 46, height: 46)
                    .background(Circle().fill(Theme.light))
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Neuer Termin")
            .accessibilityHint("Öffnet die Schnelleingabe")
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .padding(.bottom, 4)
        .background(Theme.shade)
    }

    // MARK: Inhalt

    @ViewBuilder
    private var content: some View {
        switch mode {
        case .list:
            AgendaView()
                .refreshable { await model.refresh() }
        case .day:
            DayGridView(start: model.selectedDay, dayCount: 1, showsHeader: false)
                .gesture(swipe(days: 1))
        case .week:
            DayGridView(start: weekStart, dayCount: 7)
                .gesture(swipe(days: 7))
        case .month:
            MonthView(year: model.selectedDay.year, month: model.selectedDay.month, visibleLanes: 2)
                .gesture(DragGesture(minimumDistance: 40).onEnded { value in
                    guard let step = SwipeStep.vertical(dx: value.translation.width, dy: value.translation.height) else { return }
                    model.selectedDay = LocalDate(year: model.selectedDay.year, month: model.selectedDay.month, day: 1).adding(months: step)
                })
        }
    }

    /// Nur ein klar waagerechter Wisch blättert; senkrecht scrollt (LIC-65).
    private func swipe(days: Int) -> some Gesture {
        DragGesture(minimumDistance: 40).onEnded { value in
            guard let step = SwipeStep.horizontal(dx: value.translation.width, dy: value.translation.height) else { return }
            model.page(days: step * days)
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
        // Oben unter der Navigationsleiste; unten steht die eigene Leiste.
        .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always), prompt: "Termine suchen")
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
