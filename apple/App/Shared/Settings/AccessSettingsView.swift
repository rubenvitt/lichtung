// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Angemeldete Geräte und API-Tokens für MCP-Clients (LIC-43).
struct AccessSettingsView: View {
    @Environment(AppSession.self) private var session
    @State private var newTokenName = ""
    @State private var removingDevice: Device?
    @State private var revokingToken: APIToken?

    var body: some View {
        let settings = session.settings
        Form {
            SettingsNotices()
            Section {
                if settings.devices.isEmpty {
                    Text(settings.isAvailable ? "Wird geladen …" : "Keine Geräte.")
                        .foregroundStyle(.secondary)
                }
                ForEach(settings.devices) { device in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(device.current == true ? "\(device.name) (dieses Gerät)" : device.name)
                            Text(Self.activity(created: device.createdAt, used: device.lastActivity))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        if device.current != true {
                            Button(role: .destructive) {
                                removingDevice = device
                            } label: {
                                Image(systemName: "xmark.circle")
                            }
                            .buttonStyle(.borderless)
                            .accessibilityLabel("\(device.name) abmelden")
                        }
                    }
                }
            } header: {
                Text("Geräte")
            } footer: {
                Text("Jedes Gerät hat ein eigenes Token. Ein verlorenes Gerät hier abmelden; dieses Gerät meldest du unter „Allgemein“ ab.")
            }

            Section {
                ForEach(settings.tokens) { token in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(token.name)
                            Text(Self.activity(created: token.createdAt, used: token.lastUsedAt))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button(role: .destructive) {
                            revokingToken = token
                        } label: {
                            Image(systemName: "trash")
                        }
                        .buttonStyle(.borderless)
                        .accessibilityLabel("\(token.name) widerrufen")
                    }
                }
                HStack {
                    TextField("Name des neuen Tokens", text: $newTokenName, prompt: Text("z. B. Claude Code"))
                        .onSubmit { createToken() }
                    Button("Erzeugen") { createToken() }
                        .disabled(newTokenName.trimmingCharacters(in: .whitespaces).isEmpty || !settings.isAvailable)
                }
            } header: {
                Text("API-Tokens für MCP")
            } footer: {
                Text("Ein Token öffnet /api und /mcp, etwa für Claude Code oder eine Routine. Claude.ai verbindet sich ohne Token per OAuth. Wo Claude schreiben darf, steht bei den Kalendern.")
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Geräte und Tokens")
        .task { await session.settings.loadOverview() }
        #if os(iOS)
        .refreshable { await session.settings.loadOverview() }
        #endif
        .sheet(item: Bindable(settings).createdToken) { token in
            CreatedTokenSheet(token: token)
        }
        .confirmationDialog(
            "Gerät abmelden?",
            isPresented: Binding(get: { removingDevice != nil }, set: { if !$0 { removingDevice = nil } }),
            titleVisibility: .visible,
            presenting: removingDevice
        ) { device in
            Button("„\(device.name)“ abmelden", role: .destructive) {
                Task { await settings.removeDevice(device.id) }
            }
            Button("Abbrechen", role: .cancel) {}
        } message: { _ in
            Text("Das Gerät muss sich danach neu anmelden. Seine noch nicht gesendeten Änderungen bleiben dort liegen.")
        }
        .confirmationDialog(
            "Token widerrufen?",
            isPresented: Binding(get: { revokingToken != nil }, set: { if !$0 { revokingToken = nil } }),
            titleVisibility: .visible,
            presenting: revokingToken
        ) { token in
            Button("„\(token.name)“ widerrufen", role: .destructive) {
                Task { await settings.revokeToken(token.id) }
            }
            Button("Abbrechen", role: .cancel) {}
        } message: { _ in
            Text("Clients mit diesem Token verlieren sofort den Zugang.")
        }
    }

    private func createToken() {
        let settings = session.settings
        let name = newTokenName
        Task {
            if await settings.createToken(name: name) { newTokenName = "" }
        }
    }

    static func activity(created: Date?, used: Date?) -> String {
        let now = Date()
        var parts: [String] = []
        if let created { parts.append("angelegt " + AccountStatusSummary.relative(created, now: now)) }
        parts.append(used.map { "zuletzt " + AccountStatusSummary.relative($0, now: now) } ?? "noch nie benutzt")
        return parts.joined(separator: " · ")
    }
}

/// Das neue Token, einmal sichtbar und kopierbar.
private struct CreatedTokenSheet: View {
    @Environment(\.dismiss) private var dismiss
    let token: APIToken
    @State private var copied = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(token.token ?? "–")
                        .font(.body.monospaced())
                        .textSelection(.enabled)
                    Button {
                        Clipboard.copy(token.token ?? "")
                        copied = true
                    } label: {
                        Label(copied ? "Kopiert" : "Kopieren", systemImage: copied ? "checkmark" : "doc.on.doc")
                    }
                    .disabled(token.token == nil)
                } header: {
                    Text(token.name)
                } footer: {
                    Text("Nur jetzt sichtbar. Lichtung speichert das Token nicht im Klartext; geht es verloren, einfach ein neues erzeugen.")
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Neues Token")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fertig") { dismiss() }
                }
            }
        }
        .settingsSheetFrame()
    }
}
