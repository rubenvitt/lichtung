// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

// Konflikte und abgelehnte Änderungen (LIC-70, Spec §4 „Nichts verschwindet
// still“): Warn-Knopf mit Zahl in der Toolbar, nur solange es welche gibt.
// Die Liste erklärt, was nicht angekommen ist, öffnet den Termin und klickt
// Meldungen weg (Konflikte für alle Geräte über den Server).

extension AppModel {
    var syncProblems: [SyncProblem] { state.problems(deviceTimeZone: timeZone) }

    /// Öffnet ein Event am nächsten Vorkommen ab heute (sonst dem letzten davor).
    func showEvent(id eventId: String) {
        guard let event = query.events[eventId] else { return }
        let now = Date()
        let year: TimeInterval = 366 * 24 * 3600
        let window = DateInterval(start: now.addingTimeInterval(-year), end: now.addingTimeInterval(year))
        let all = query.expander.occurrences(of: event, in: window).sorted { $0.start < $1.start }
        guard let occ = all.first(where: { $0.end >= now }) ?? all.last else { return }
        show(occ)
    }
}

private struct SyncProblemsToolbarButton: ViewModifier {
    @Environment(AppModel.self) private var model
    @State private var showsList = false
    /// Event, das aufgeht, sobald die Liste zu ist.
    @State private var pendingEvent: String?

    @ViewBuilder
    func body(content: Content) -> some View {
        let count = model.syncProblems.count
        #if os(macOS)
        content
            .toolbar {
                if count > 0 {
                    ToolbarItem(placement: .primaryAction) {
                        Button { showsList.toggle() } label: { SyncProblemsIcon(count: count) }
                            .help(Self.title(count))
                            .accessibilityLabel(Self.title(count))
                            .popover(isPresented: $showsList, arrowEdge: .bottom) {
                                VStack(alignment: .leading, spacing: 0) {
                                    Text("Nicht übernommene Änderungen")
                                        .font(.headline)
                                        .padding(12)
                                    Divider()
                                    SyncProblemsList { select($0) }
                                }
                                .frame(width: 400, height: 420)
                                .environment(model)
                            }
                    }
                }
            }
            .onChange(of: showsList) { _, shown in
                if !shown { showPending() }
            }
        #else
        content
            .toolbar {
                if count > 0 {
                    ToolbarItem(placement: .topBarLeading) {
                        Button { showsList = true } label: {
                            Label { Text(Self.title(count)) } icon: { SyncProblemsIcon(count: count) }
                        }
                        .accessibilityLabel(Self.title(count))
                    }
                }
            }
            .sheet(isPresented: $showsList, onDismiss: { showPending() }) {
                NavigationStack {
                    SyncProblemsList { select($0) }
                        .navigationTitle("Nicht übernommen")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar {
                            ToolbarItem(placement: .confirmationAction) {
                                Button("Fertig") { showsList = false }
                            }
                        }
                }
                .environment(model)
            }
        #endif
    }

    private func select(_ eventId: String) {
        pendingEvent = eventId
        showsList = false
    }

    private func showPending() {
        guard let id = pendingEvent else { return }
        pendingEvent = nil
        model.showEvent(id: id)
    }

    static func title(_ count: Int) -> String {
        count == 1 ? "1 Änderung nicht übernommen" : "\(count) Änderungen nicht übernommen"
    }
}

extension View {
    /// Warn-Knopf für Konflikte und abgelehnte Änderungen (LIC-70).
    func syncProblemsButton() -> some View {
        modifier(SyncProblemsToolbarButton())
    }
}

struct SyncProblemsIcon: View {
    let count: Int

    var body: some View {
        Image(systemName: "exclamationmark.arrow.triangle.2.circlepath")
            .symbolRenderingMode(.hierarchical)
            .foregroundStyle(.orange)
            .overlay(alignment: .topTrailing) {
                Text(count > 99 ? "99+" : "\(count)")
                    .font(.system(size: 10, weight: .bold).monospacedDigit())
                    .foregroundStyle(.white)
                    .padding(.horizontal, 4)
                    .frame(minWidth: 16, minHeight: 16)
                    .background(Capsule().fill(Color.orange))
                    .offset(x: 9, y: -7)
                    .accessibilityHidden(true)
            }
    }
}

struct SyncProblemsList: View {
    @Environment(AppModel.self) private var model
    @Environment(AppSession.self) private var session
    let onOpen: @MainActor (String) -> Void
    @State private var busy: Set<String> = []
    @State private var error: String?

    var body: some View {
        let problems = model.syncProblems
        if problems.isEmpty {
            ContentUnavailableView(
                "Alles übernommen",
                systemImage: "checkmark.circle",
                description: Text("Änderungen, die beim Anbieter nicht angekommen sind, erscheinen hier.")
            )
        } else {
            List {
                if let error {
                    Label(error, systemImage: "wifi.exclamationmark").foregroundStyle(.red)
                }
                Section {
                    ForEach(problems) { problem in
                        row(problem)
                    }
                } footer: {
                    Text("Hatte der Anbieter inzwischen eine neuere Fassung, gilt diese. Deine Änderung steht hier, bis du sie wegklickst.")
                }
            }
            #if os(macOS)
            .listStyle(.inset)
            #else
            .listStyle(.insetGrouped)
            #endif
        }
    }

    private func row(_ p: SyncProblem) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(p.title).font(.headline)
            Text(p.message)
            if let detail = p.detail {
                Text(detail).font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                if let id = p.eventId {
                    Button("Termin öffnen") { onOpen(id) }
                }
                Spacer()
                if busy.contains(p.id) {
                    ProgressView().controlSize(.small)
                } else {
                    Button("Wegklicken") { Task { await dismiss(p) } }
                }
            }
            .buttonStyle(.borderless)
            .padding(.top, 2)
        }
        .padding(.vertical, 4)
        #if os(iOS)
        .swipeActions {
            Button("Wegklicken") { Task { await dismiss(p) } }
        }
        #endif
    }

    private func dismiss(_ p: SyncProblem) async {
        error = nil
        switch p.kind {
        case .rejected:
            await model.engine.dismissNotice(p.id)
        case .conflict:
            guard let management = session.management else {
                error = "Ohne Server lässt sich der Konflikt nicht wegklicken."
                return
            }
            busy.insert(p.id)
            defer { busy.remove(p.id) }
            do {
                try await management.dismissConflict(p.id)
                await model.refresh()
            } catch {
                self.error = "Wegklicken hat nicht geklappt. Bitte später noch einmal."
            }
        }
    }
}
