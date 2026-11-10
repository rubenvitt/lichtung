// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Blocker (LIC-75): Termine aus einem Kalender blocken die Zeit in einem
/// anderen. Der Server legt die Blocker an und hält sie aktuell.
struct BlockersSettingsView: View {
    @Environment(AppSession.self) private var session
    @State private var editing: BlockerRule?
    @State private var creating = false

    var body: some View {
        let settings = session.settings
        List {
            SettingsNotices()
            Section {
                if settings.blockerRules.isEmpty {
                    Text("Noch keine Regeln. Eine Regel trägt jeden Termin aus einem Kalender als „Blockiert“ in einen anderen ein, etwa private Termine in den Arbeitskalender.")
                        .foregroundStyle(Theme.text2)
                }
                ForEach(settings.blockerRules) { rule in
                    Button {
                        editing = rule
                    } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text("\(name(rule.sourceCalendarId)) blockt in \(name(rule.targetCalendarId))")
                                Text(summary(rule))
                                    .font(.caption)
                                    .foregroundStyle(Theme.text2)
                                if let error = rule.lastError, rule.failedCount > 0 {
                                    Label(error, systemImage: "exclamationmark.triangle.fill")
                                        .font(.caption)
                                        .foregroundStyle(.orange)
                                }
                            }
                            Spacer()
                            Image(systemName: "chevron.right")
                                .font(.caption)
                                .foregroundStyle(Theme.text2)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            } header: {
                Text("Regeln")
            } footer: {
                Text("Der Blocker trägt nur die Zeit, keinen Titel, Ort oder Teilnehmer des Termins. Er folgt beim Verschieben und Löschen. Abgesagte, abgelehnte und als frei markierte Termine blocken nicht.")
            }
            Section {
                Button {
                    creating = true
                } label: {
                    Label("Neue Regel", systemImage: "plus")
                }
                .disabled(!settings.isAvailable)
            }
        }
        .navigationTitle("Blocker")
        .task { await settings.loadBlockerRules() }
        .refreshable { await settings.loadBlockerRules() }
        .sheet(item: $editing) { rule in
            BlockerRuleEditor(rule: rule)
                .environment(session)
        }
        .sheet(isPresented: $creating) {
            BlockerRuleEditor(rule: nil)
                .environment(session)
        }
    }

    private func name(_ calendarId: String) -> String {
        session.settings.calendar(calendarId)?.name ?? "Unbekannter Kalender"
    }

    private func summary(_ rule: BlockerRule) -> String {
        var parts = ["Titel „\(rule.title)“"]
        if rule.allDay { parts.append("auch ganztägige") }
        parts.append(rule.blockerCount == 1 ? "1 Blocker" : "\(rule.blockerCount) Blocker")
        return parts.joined(separator: ", ")
    }
}

/// Neue Regel oder Titel und Ganztägiges einer bestehenden; gesendet wird
/// erst mit „Sichern“.
private struct BlockerRuleEditor: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    let original: BlockerRule?
    @State private var source: String?
    @State private var target: String?
    @State private var title: String
    @State private var allDay: Bool
    @State private var confirmsDelete = false

    init(rule: BlockerRule?) {
        original = rule
        _source = State(initialValue: rule?.sourceCalendarId)
        _target = State(initialValue: rule?.targetCalendarId)
        _title = State(initialValue: rule?.title ?? "Blockiert")
        _allDay = State(initialValue: rule?.allDay ?? false)
    }

    var body: some View {
        let settings = session.settings
        NavigationStack {
            Form {
                Section {
                    if let original {
                        LabeledContent("Termine aus", value: settings.calendar(original.sourceCalendarId)?.name ?? "–")
                        LabeledContent("blocken in", value: settings.calendar(original.targetCalendarId)?.name ?? "–")
                    } else {
                        Picker("Termine aus", selection: $source) {
                            Text("Bitte wählen").tag(String?.none)
                            ForEach(settings.blockerSources) { calendar in
                                Text(calendar.name).tag(String?.some(calendar.id))
                            }
                        }
                        Picker("blocken in", selection: $target) {
                            Text("Bitte wählen").tag(String?.none)
                            ForEach(settings.blockerTargets.filter { $0.id != source }) { calendar in
                                Text(calendar.name).tag(String?.some(calendar.id))
                            }
                        }
                    }
                } footer: {
                    if original == nil {
                        Text("Blocken lässt sich in jedem Kalender, in den Lichtung schreiben kann. Für jedes Kalenderpaar gibt es eine Regel.")
                    }
                }
                Section {
                    TextField("Titel", text: $title, prompt: Text("Blockiert"))
                    Toggle("Auch ganztägige Termine", isOn: $allDay)
                } footer: {
                    Text("Den Titel sehen alle, die den Zielkalender sehen. In Lichtung steht der Blocker hinter seinem Termin, wenn „Blocker ausblenden“ an ist.")
                }
                if original != nil {
                    Section {
                        Button("Regel löschen", role: .destructive) { confirmsDelete = true }
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle(original == nil ? "Neue Regel" : "Regel bearbeiten")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Abbrechen") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Sichern") { save() }
                        .disabled(!canSave || settings.isWorking)
                }
            }
            .confirmationDialog("Regel löschen?", isPresented: $confirmsDelete, titleVisibility: .visible) {
                Button("Löschen", role: .destructive) { delete() }
                Button("Abbrechen", role: .cancel) {}
            } message: {
                Text("Lichtung entfernt danach alle Blocker dieser Regel aus dem Zielkalender.")
            }
        }
        .settingsSheetFrame()
    }

    private var trimmedTitle: String { title.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var canSave: Bool {
        original != nil || session.settings.canCreateBlockerRule(source: source, target: target)
    }

    private func save() {
        let settings = session.settings
        Task {
            let ok: Bool
            if let original {
                let newTitle = trimmedTitle.isEmpty ? "Blockiert" : trimmedTitle
                ok = await settings.updateBlockerRule(original.id, BlockerRulePatch(
                    title: newTitle == original.title ? nil : newTitle,
                    allDay: allDay == original.allDay ? nil : allDay
                ))
            } else if let source, let target {
                ok = await settings.createBlockerRule(NewBlockerRule(
                    sourceCalendarId: source, targetCalendarId: target,
                    title: trimmedTitle.isEmpty ? nil : trimmedTitle, allDay: allDay
                ))
            } else {
                ok = false
            }
            if ok { dismiss() }
        }
    }

    private func delete() {
        guard let original else { return }
        let settings = session.settings
        Task {
            if await settings.deleteBlockerRule(original.id) { dismiss() }
        }
    }
}
