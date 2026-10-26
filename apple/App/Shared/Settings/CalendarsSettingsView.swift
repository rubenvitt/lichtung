// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Kalender je Konto: Reihenfolge (Ziehen), Farbe, Sichtbarkeit,
/// Schreibschutz für Claude, Openings (LIC-43).
struct CalendarsSettingsView: View {
    @Environment(AppSession.self) private var session
    @State private var editing: CalendarInfo?

    var body: some View {
        let settings = session.settings
        List {
            SettingsNotices()
            if settings.calendarSections.isEmpty {
                Text("Noch keine Kalender. Sie erscheinen, sobald ein Konto verbunden ist.")
                    .foregroundStyle(.secondary)
            }
            ForEach(settings.calendarSections) { section in
                Section {
                    ForEach(section.calendars) { calendar in
                        Button {
                            editing = calendar
                        } label: {
                            CalendarSettingsRow(calendar: calendar, account: section.account)
                        }
                        .buttonStyle(.plain)
                    }
                    .onMove { from, to in
                        let ids = section.calendars.map(\.id)
                        Task { await settings.moveCalendars(section: ids, from: from, to: to) }
                    }
                } header: {
                    Text(section.title)
                } footer: {
                    Text(Self.moveHint)
                }
            }
        }
        .navigationTitle("Kalender")
        #if os(iOS)
        .toolbar { EditButton() }
        #endif
        .sheet(item: $editing) { calendar in
            CalendarSettingsSheet(calendarId: calendar.id)
                .environment(session)
        }
    }

    #if os(macOS)
    private static let moveHint = "Zum Sortieren ziehen. Die Reihenfolge gilt in allen Ansichten."
    #else
    private static let moveHint = "Zum Sortieren „Bearbeiten“ tippen. Die Reihenfolge gilt in allen Ansichten."
    #endif
}

private struct CalendarSettingsRow: View {
    let calendar: CalendarInfo
    let account: Account?

    var body: some View {
        HStack(spacing: 10) {
            CalendarDot(color: Color(hex: calendar.displayColor), size: 12)
            VStack(alignment: .leading, spacing: 2) {
                Text(calendar.name)
                    .foregroundStyle(calendar.hidden ? Color.secondary : Color.primary)
                Text(badges)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if MCPWriteGuard.isWorkCalendar(calendar, account: account), !calendar.mcpWrite {
                Image(systemName: "lock.shield")
                    .foregroundStyle(.secondary)
                    .help("Claude darf hier nicht schreiben")
            }
            Image(systemName: "chevron.right")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .contentShape(Rectangle())
    }

    private var badges: String {
        var parts: [String] = []
        if calendar.hidden { parts.append("ausgeblendet") }
        if calendar.isWork == true { parts.append("Arbeitskalender") }
        if !calendar.writable { parts.append("nur lesen") }
        parts.append(calendar.mcpWrite ? "Claude darf schreiben" : "Claude nur lesend")
        return parts.joined(separator: " · ")
    }
}

/// Einstellungen eines Kalenders. Liest den Kalender immer frisch aus dem
/// Modell, damit Schalter den gesendeten Stand zeigen.
private struct CalendarSettingsSheet: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    let calendarId: String
    @State private var confirmsMCP = false

    var body: some View {
        let settings = session.settings
        NavigationStack {
            Group {
                if let calendar = settings.calendar(calendarId) {
                    form(calendar, settings: settings)
                } else {
                    Text("Diesen Kalender gibt es nicht mehr.")
                        .foregroundStyle(.secondary)
                }
            }
            .navigationTitle(settings.calendar(calendarId)?.name ?? "Kalender")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fertig") { dismiss() }
                }
            }
        }
        .settingsSheetFrame()
    }

    @ViewBuilder
    private func form(_ calendar: CalendarInfo, settings: SettingsModel) -> some View {
        let account = settings.account(calendar.accountId)
        let isWork = MCPWriteGuard.isWorkCalendar(calendar, account: account)
        Form {
            SettingsNotices()
            Section {
                LabeledContent("Konto", value: account?.name ?? "–")
                if !calendar.writable {
                    Text("Nur lesbar: Termine lassen sich hier nicht anlegen oder ändern.")
                        .foregroundStyle(.secondary)
                }
            }

            Section("Farbe") {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 32), spacing: 10)], spacing: 10) {
                    ForEach(CalendarPalette.swatches) { swatch in
                        Button {
                            update(CalendarSettings(color: .set(swatch.hex)))
                        } label: {
                            Circle()
                                .fill(Color(hex: swatch.hex))
                                .frame(width: 28, height: 28)
                                .overlay {
                                    if calendar.displayColor?.uppercased() == swatch.hex {
                                        Image(systemName: "checkmark")
                                            .font(.caption.weight(.bold))
                                            .foregroundStyle(.white)
                                    }
                                }
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(swatch.name)
                    }
                }
                .padding(.vertical, 4)
                Button("Farbe des Anbieters") { update(CalendarSettings(color: .clear)) }
                    .disabled(calendar.color == nil)
            }

            Section {
                Toggle("In Lichtung anzeigen", isOn: Binding(
                    get: { !calendar.hidden },
                    set: { update(CalendarSettings(hidden: !$0)) }
                ))
            } footer: {
                Text("Ausgeblendete Kalender erscheinen in keiner Ansicht und in keinem Set.")
            }

            Section {
                if MCPWriteGuard.canMark(calendar) {
                    Toggle(MCPWriteGuard.markTitle, isOn: Binding(
                        get: { calendar.isWork == true },
                        set: { update(CalendarSettings(isWork: $0)) }
                    ))
                    if MCPWriteGuard.suggestsMarking(calendar, account: account) {
                        Text(MCPWriteGuard.suggestion)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Toggle("Claude darf hier schreiben", isOn: Binding(
                    get: { calendar.mcpWrite },
                    set: { allow in
                        if MCPWriteGuard.confirmation(enabling: allow, calendar: calendar, account: account) != nil {
                            confirmsMCP = true
                        } else {
                            update(CalendarSettings(mcpWrite: allow))
                        }
                    }
                ))
                if isWork {
                    Label(
                        calendar.mcpWrite
                            ? "Arbeitskalender: Claude darf hier gerade schreiben."
                            : "Arbeitskalender: Claude darf hier nur lesen. So ist es gedacht.",
                        systemImage: calendar.mcpWrite ? "exclamationmark.shield.fill" : "lock.shield"
                    )
                    .foregroundStyle(calendar.mcpWrite ? Color.red : Color.orange)
                }
            } header: {
                Text("Claude (MCP)")
            } footer: {
                Text(MCPWriteGuard.canMark(calendar) ? MCPWriteGuard.explanation + " " + MCPWriteGuard.markExplanation : MCPWriteGuard.explanation)
            }

            Section {
                Toggle("Termine blockieren Openings", isOn: Binding(
                    get: { calendar.openingsBusy },
                    set: { update(CalendarSettings(openingsBusy: $0)) }
                ))
                Toggle("Ganztägige Termine blockieren", isOn: Binding(
                    get: { calendar.allDayBlocks },
                    set: { update(CalendarSettings(allDayBlocks: $0)) }
                ))
            } header: {
                Text("Openings")
            } footer: {
                Text("Belegte Zeiten dieses Kalenders fehlen in Buchungslinks und bei der Suche nach freien Zeiten. Ganztägige Termine zählen nur, wenn das zweite Häkchen gesetzt ist.")
            }
        }
        .formStyle(.grouped)
        .confirmationDialog(
            "Claude im Arbeitskalender schreiben lassen?",
            isPresented: $confirmsMCP,
            titleVisibility: .visible
        ) {
            Button("Trotzdem erlauben", role: .destructive) {
                update(CalendarSettings(mcpWrite: true))
            }
            Button("Abbrechen", role: .cancel) {}
        } message: {
            Text(MCPWriteGuard.confirmation(enabling: true, calendar: calendar, account: account) ?? "")
        }
    }

    private func update(_ change: CalendarSettings) {
        let settings = session.settings
        let id = calendarId
        Task { await settings.updateCalendar(id, change) }
    }
}
