// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

// Terminanfragen (Spec §5, LIC-30): Knopf mit Zahl in der Toolbar, Liste
// aller offenen Anfragen, Antworten in Liste, Detail und Kontextmenü. Die
// Regeln kommen aus dem Kern (`InvitationList`, `InvitationActions`).

extension AppModel {
    /// Offene Anfragen ab jetzt (alle nicht ausgeblendeten Kalender, nicht nur die Auswahl).
    var invitations: InvitationList { query.invitations(now: Date()) }

    func canRespond(_ occ: Occurrence) -> Bool {
        InvitationActions.canRespond(occ, calendars: state.calendars)
    }

    /// Eigener Status samt noch nicht gesendeter Antworten (Detail-Blatt bleibt aktuell).
    func responseStatus(_ occ: Occurrence) -> ParticipationStatus? {
        InvitationActions.currentStatus(of: occ, in: query.events)
    }

    /// Antwort am Termin: bei Serien nur dieses Vorkommen (`.this`) oder alle (`.all`).
    func respond(_ occ: Occurrence, _ response: InvitationResponse, scope: EditScope = .all) async {
        await engine.perform(InvitationActions.intent(occ, response: response, scope: scope), label: response.undoLabel)
    }

    /// Antwort aus der Liste (Serie: alle Vorkommen, geändertes Vorkommen: nur dieses).
    func respond(to invitation: Invitation, _ response: InvitationResponse) async {
        await engine.perform(invitation.intent(response), label: response.undoLabel)
    }
}

extension InvitationResponse {
    var systemImage: String {
        switch self {
        case .accepted: "checkmark.circle"
        case .tentative: "questionmark.circle"
        case .declined: "xmark.circle"
        }
    }

    var tint: Color {
        switch self {
        case .accepted: .green
        case .tentative: .orange
        case .declined: .red
        }
    }
}

/// Toolbar-Knopf mit Zahl der offenen Anfragen. iPhone: Blatt mit der
/// Liste; Mac: Popover. Ein Tipp auf eine Anfrage schließt die Liste und
/// öffnet das Detail am Tag des Termins.
private struct InvitationsToolbarButton: ViewModifier {
    @Environment(AppModel.self) private var model
    @State private var showsList = false
    /// Termin, dessen Detail aufgeht, sobald die Liste zu ist (zwei Blätter
    /// gleichzeitig zeigt SwiftUI nicht).
    @State private var pendingShow: Occurrence?

    @ViewBuilder
    func body(content: Content) -> some View {
        let count = model.invitations.badgeCount
        #if os(macOS)
        content
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button { showsList.toggle() } label: { InvitationsIcon(count: count) }
                        .help(Self.title(count))
                        .accessibilityLabel(Self.title(count))
                        .popover(isPresented: $showsList, arrowEdge: .bottom) {
                            VStack(alignment: .leading, spacing: 0) {
                                Text("Terminanfragen")
                                    .font(.headline)
                                    .padding(12)
                                Divider()
                                InvitationsList { select($0) }
                            }
                            .frame(width: 380, height: 480)
                            .environment(model)
                        }
                }
            }
            .onChange(of: showsList) { _, shown in
                if !shown { showPending() }
            }
        #else
        content
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button { showsList = true } label: { InvitationsIcon(count: count) }
                        .accessibilityLabel(Self.title(count))
                }
            }
            .sheet(isPresented: $showsList, onDismiss: { showPending() }) {
                NavigationStack {
                    InvitationsList { select($0) }
                        .navigationTitle("Terminanfragen")
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

    private func select(_ occ: Occurrence) {
        pendingShow = occ
        showsList = false
    }

    private func showPending() {
        guard let occ = pendingShow else { return }
        pendingShow = nil
        model.show(occ)
    }

    static func title(_ count: Int) -> String {
        switch count {
        case 0: "Terminanfragen"
        case 1: "Terminanfragen: 1 offen"
        default: "Terminanfragen: \(count) offen"
        }
    }
}

extension View {
    /// Knopf „Terminanfragen“ in der Toolbar (LIC-30).
    func invitationsButton() -> some View {
        modifier(InvitationsToolbarButton())
    }
}

/// Ablage-Symbol mit roter Zahl, solange Anfragen offen sind.
struct InvitationsIcon: View {
    let count: Int

    var body: some View {
        Image(systemName: count > 0 ? "tray.full" : "tray")
            .overlay(alignment: .topTrailing) {
                if count > 0 {
                    Text(count > 99 ? "99+" : "\(count)")
                        .font(.system(size: 10, weight: .bold).monospacedDigit())
                        .foregroundStyle(.white)
                        .padding(.horizontal, 4)
                        .frame(minWidth: 16, minHeight: 16)
                        .background(Capsule().fill(Color.red))
                        .offset(x: 9, y: -7)
                        .accessibilityHidden(true)
                }
            }
    }
}

/// Alle offenen Anfragen, nach Tagen gruppiert.
struct InvitationsList: View {
    @Environment(AppModel.self) private var model
    let onSelect: @MainActor (Occurrence) -> Void

    var body: some View {
        let list = model.invitations
        if list.isEmpty {
            ContentUnavailableView(
                "Keine offenen Anfragen",
                systemImage: "tray",
                description: Text("Einladungen, auf die du noch nicht geantwortet hast, erscheinen hier.")
            )
        } else {
            List {
                ForEach(list.days(today: model.today, calendar: model.calendar)) { day in
                    Section(day.heading) {
                        ForEach(day.items) { invitation in
                            InvitationRow(invitation: invitation, onSelect: onSelect)
                        }
                    }
                }
            }
            #if os(macOS)
            .listStyle(.inset)
            #else
            .listStyle(.insetGrouped)
            #endif
        }
    }
}

/// Eine Anfrage: Titel, Zeit, Organisator, Kalender, darunter die drei
/// Antworten. iPhone zusätzlich zum Wischen, beide mit Kontextmenü.
struct InvitationRow: View {
    @Environment(AppModel.self) private var model
    let invitation: Invitation
    let onSelect: @MainActor (Occurrence) -> Void

    var body: some View {
        let occ = invitation.occurrence
        VStack(alignment: .leading, spacing: 8) {
            Button {
                onSelect(occ)
            } label: {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    CalendarDot(color: model.color(for: occ.calendarId))
                    VStack(alignment: .leading, spacing: 2) {
                        Text(occ.title).font(.body.weight(.medium))
                        Text(invitation.dateText(today: model.today, calendar: model.calendar))
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                        if let source = invitation.sourceText {
                            Text(source).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                        if let series = invitation.seriesText {
                            Label(series, systemImage: "repeat").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if invitation.canRespond {
                InvitationResponseButtons(current: nil) { response in respond(response) }
            } else {
                Label(Invitation.readOnlyHint, systemImage: "lock")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 4)
        .swipeActions(edge: .leading) {
            if invitation.canRespond {
                Button(InvitationResponse.accepted.actionTitle) { respond(.accepted) }
                    .tint(.green)
            }
        }
        .swipeActions(edge: .trailing) {
            if invitation.canRespond {
                Button(InvitationResponse.declined.actionTitle) { respond(.declined) }
                    .tint(.red)
                Button(InvitationResponse.tentative.actionTitle) { respond(.tentative) }
                    .tint(.orange)
            }
        }
        .contextMenu {
            Button("Öffnen") { onSelect(occ) }
            if invitation.canRespond {
                Divider()
                ForEach(InvitationResponse.ordered, id: \.self) { response in
                    Button(response.actionTitle, systemImage: response.systemImage) { respond(response) }
                }
            }
        }
    }

    private func respond(_ response: InvitationResponse) {
        let invitation = invitation
        Task { await model.respond(to: invitation, response) }
    }
}

/// „Annehmen“, „Vielleicht“, „Ablehnen“ nebeneinander; die gewählte Antwort
/// (`current`) hervorgehoben.
struct InvitationResponseButtons: View {
    let current: InvitationResponse?
    let onRespond: @MainActor (InvitationResponse) -> Void

    var body: some View {
        HStack(spacing: 8) {
            ForEach(InvitationResponse.ordered, id: \.self) { response in
                if response == current {
                    Button { onRespond(response) } label: {
                        Label(response.actionTitle, systemImage: response.systemImage)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(response.tint)
                    .accessibilityAddTraits(.isSelected)
                } else {
                    Button { onRespond(response) } label: {
                        Label(response.actionTitle, systemImage: response.systemImage)
                    }
                    .buttonStyle(.bordered)
                    .tint(response.tint)
                }
            }
        }
        .controlSize(.small)
        .labelStyle(.titleAndIcon)
    }
}

/// Abschnitt „Teilnahme“ im Detail-Blatt: Organisator, eigener Status und
/// die drei Antworten. Bei Serien folgt die Frage „Nur dieser Termin“ oder
/// „Alle Termine“.
struct InvitationDetailSection: View {
    @Environment(AppModel.self) private var model
    let occurrence: Occurrence

    var body: some View {
        let status = model.responseStatus(occurrence) ?? occurrence.details.selfStatus
        Section("Teilnahme") {
            if let organizer = occurrence.details.organizer {
                LabeledContent("Organisiert von", value: organizerName(organizer))
            }
            LabeledContent("Deine Antwort", value: status?.responseText ?? "–")
            if model.canRespond(occurrence) {
                InvitationDetailButtons(occurrence: occurrence, current: InvitationResponse(status))
            } else if occurrence.details.status != .cancelled {
                Text(Invitation.readOnlyHint).font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private func organizerName(_ p: Person) -> String {
        let name = p.name?.trimmingCharacters(in: .whitespaces) ?? ""
        return name.isEmpty ? p.email : name
    }
}

/// Zeile mit den Antworten im Detail, mit der Frage nach dem Geltungsbereich
/// bei Serien (`InvitationActions.scopes`).
private struct InvitationDetailButtons: View {
    @Environment(AppModel.self) private var model
    let occurrence: Occurrence
    let current: InvitationResponse?
    @State private var pendingResponse: InvitationResponse?

    var body: some View {
        InvitationResponseButtons(current: current) { response in
            if InvitationActions.scopes(occurrence).isEmpty {
                respond(response, scope: .all)
            } else {
                pendingResponse = response
            }
        }
        .confirmationDialog(
            "Antwort für die Serie",
            isPresented: isAsking,
            titleVisibility: .visible,
            presenting: pendingResponse
        ) { response in
            ForEach(InvitationActions.scopes(occurrence), id: \.self) { scope in
                Button(scope.editorLabel) { respond(response, scope: scope) }
            }
            Button("Abbrechen", role: .cancel) {}
        } message: { response in
            Text("„\(occurrence.title)“ gehört zu einer Serie. Für welche Termine gilt „\(response.actionTitle)“?")
        }
    }

    private var isAsking: Binding<Bool> {
        Binding(get: { pendingResponse != nil }, set: { if !$0 { pendingResponse = nil } })
    }

    private func respond(_ response: InvitationResponse, scope: EditScope) {
        let occ = occurrence
        Task { await model.respond(occ, response, scope: scope) }
    }
}

/// Untermenü „Antworten“ im Kontextmenü und im Menü „Termin“ (Mac). Bei
/// Serien je ein Abschnitt für „Nur dieser Termin“ und „Alle Termine“.
struct InvitationMenu: View {
    let model: AppModel
    let occurrence: Occurrence

    var body: some View {
        let scopes = InvitationActions.scopes(occurrence)
        let current = InvitationResponse(model.responseStatus(occurrence))
        Menu("Antworten") {
            if scopes.isEmpty {
                buttons(scope: .all, current: current)
            } else {
                ForEach(scopes, id: \.self) { scope in
                    Section(scope.editorLabel) {
                        buttons(scope: scope, current: scope == .this ? current : nil)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func buttons(scope: EditScope, current: InvitationResponse?) -> some View {
        ForEach(InvitationResponse.ordered, id: \.self) { response in
            Button {
                let occ = occurrence
                Task { await model.respond(occ, response, scope: scope) }
            } label: {
                if response == current {
                    Label(response.actionTitle, systemImage: "checkmark")
                } else {
                    Text(response.actionTitle)
                }
            }
        }
    }
}
