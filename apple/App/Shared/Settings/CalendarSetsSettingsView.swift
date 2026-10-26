// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Kalender-Sets wie in Fantastical (Spec §5): anlegen, umbenennen,
/// Kalender zuordnen, sortieren, löschen (LIC-43).
struct CalendarSetsSettingsView: View {
    @Environment(AppSession.self) private var session
    @State private var editing: CalendarSet?
    @State private var creating = false

    var body: some View {
        let settings = session.settings
        List {
            SettingsNotices()
            Section {
                if settings.calendarSets.isEmpty {
                    Text("Noch keine Sets. Ein Set blendet mehrere Kalender gemeinsam ein, z. B. „Arbeit“ oder „Privat“.")
                        .foregroundStyle(.secondary)
                }
                ForEach(settings.calendarSets) { set in
                    Button {
                        editing = set
                    } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(set.name)
                                Text(set.calendarIds.count == 1 ? "1 Kalender" : "\(set.calendarIds.count) Kalender")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Image(systemName: "chevron.right")
                                .font(.caption)
                                .foregroundStyle(.tertiary)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
                .onMove { from, to in
                    Task { await settings.moveSets(from: from, to: to) }
                }
            } header: {
                Text("Sets")
            }
            Section {
                Button {
                    creating = true
                } label: {
                    Label("Neues Set", systemImage: "plus")
                }
                .disabled(!settings.isAvailable)
            }
        }
        .navigationTitle("Kalender-Sets")
        #if os(iOS)
        .toolbar { EditButton() }
        #endif
        .sheet(item: $editing) { set in
            CalendarSetEditor(set: set)
                .environment(session)
        }
        .sheet(isPresented: $creating) {
            CalendarSetEditor(set: nil)
                .environment(session)
        }
    }
}

/// Name und Kalender eines Sets; gesendet wird erst mit „Sichern“.
private struct CalendarSetEditor: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    let original: CalendarSet?
    @State private var name: String
    @State private var members: Set<String>
    @State private var error: String?
    @State private var confirmsDelete = false

    init(set: CalendarSet?) {
        original = set
        _name = State(initialValue: set?.name ?? "")
        _members = State(initialValue: Set(set?.calendarIds ?? []))
    }

    var body: some View {
        let settings = session.settings
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name, prompt: Text("z. B. Arbeit"))
                }
                ForEach(settings.calendarSections) { section in
                    Section(section.title) {
                        ForEach(section.calendars) { calendar in
                            Toggle(isOn: membership(calendar.id)) {
                                HStack(spacing: 8) {
                                    CalendarDot(color: Color(hex: calendar.displayColor), size: 10)
                                    Text(calendar.name)
                                }
                            }
                        }
                    }
                }
                if let error {
                    Section {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red)
                    }
                }
                if original != nil {
                    Section {
                        Button("Set löschen", role: .destructive) { confirmsDelete = true }
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle(original == nil ? "Neues Set" : "Set bearbeiten")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Abbrechen") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Sichern") { save() }
                        .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || settings.isWorking)
                }
            }
            .confirmationDialog("Set löschen?", isPresented: $confirmsDelete, titleVisibility: .visible) {
                Button("Löschen", role: .destructive) { delete() }
                Button("Abbrechen", role: .cancel) {}
            } message: {
                Text("Die Kalender bleiben, nur die Gruppe verschwindet.")
            }
        }
        .settingsSheetFrame()
    }

    private func membership(_ id: String) -> Binding<Bool> {
        Binding(
            get: { members.contains(id) },
            set: { isMember in
                if isMember { members.insert(id) } else { members.remove(id) }
            }
        )
    }

    /// Reihenfolge der Kalender wie in der Liste, nicht wie angeklickt.
    private var orderedMembers: [String] {
        session.settings.calendarSections.flatMap(\.calendars).map(\.id).filter { members.contains($0) }
    }

    private func save() {
        let settings = session.settings
        let ids = orderedMembers
        let n = name
        let id = original?.id
        Task {
            if await settings.saveSet(id: id, name: n, calendarIds: ids) {
                dismiss()
            } else {
                error = settings.errorMessage
                settings.errorMessage = nil
            }
        }
    }

    private func delete() {
        guard let id = original?.id else { return }
        let settings = session.settings
        Task {
            await settings.deleteSet(id)
            if settings.errorMessage == nil {
                dismiss()
            } else {
                error = settings.errorMessage
                settings.errorMessage = nil
            }
        }
    }
}
