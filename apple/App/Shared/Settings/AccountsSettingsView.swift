// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Konten anlegen, umbenennen, Passwort erneuern, entfernen (LIC-43).
struct AccountsSettingsView: View {
    @Environment(AppSession.self) private var session
    @State private var adding = false
    @State private var renaming: Account?
    @State private var renewing: Account?
    @State private var removing: Account?

    var body: some View {
        let settings = session.settings
        Form {
            SettingsNotices()
            Section {
                if settings.accounts.isEmpty {
                    Text("Noch kein Konto verbunden.")
                        .foregroundStyle(.secondary)
                }
                ForEach(settings.accounts) { account in
                    AccountRow(
                        account: account,
                        rename: { renaming = account },
                        renew: { renewing = account },
                        syncNow: { Task { await settings.syncAccountNow(account.id) } },
                        remove: { removing = account }
                    )
                }
            } header: {
                Text("Konten")
            } footer: {
                Text("Der Server gleicht die Konten ab und hält die Zugangsdaten. Entfernen löscht nichts beim Anbieter.")
            }
            Section {
                Button {
                    adding = true
                } label: {
                    Label("Konto hinzufügen", systemImage: "plus")
                }
                .disabled(!settings.isAvailable)
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Konten")
        .sheet(isPresented: $adding) {
            AddAccountSheet()
                .environment(session)
        }
        .sheet(item: $renaming) { account in
            RenameAccountSheet(account: account)
                .environment(session)
        }
        .sheet(item: $renewing) { account in
            CredentialsSheet(account: account)
                .environment(session)
        }
        .confirmationDialog(
            "Konto entfernen?",
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            titleVisibility: .visible,
            presenting: removing
        ) { account in
            Button("„\(account.name)“ entfernen", role: .destructive) {
                Task { await settings.removeAccount(account.id) }
            }
            Button("Abbrechen", role: .cancel) {}
        } message: { _ in
            Text("Kalender und Termine dieses Kontos verschwinden aus Lichtung. Beim Anbieter bleibt alles.")
        }
    }
}

private struct AccountRow: View {
    let account: Account
    let rename: @MainActor () -> Void
    let renew: @MainActor () -> Void
    let syncNow: @MainActor () -> Void
    let remove: @MainActor () -> Void

    var body: some View {
        let summary = AccountStatusSummary(account.status, now: Date())
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Text(account.name).font(.body.weight(.medium))
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                HStack(spacing: 6) {
                    StatusDot(level: summary.level)
                    Text(summary.text)
                        .font(.caption)
                        .foregroundStyle(summary.level == .error ? Color.red : Color.secondary)
                }
                if summary.needsCredentials {
                    Button("Passwort erneuern") { renew() }
                        .font(.caption)
                        .buttonStyle(.borderless)
                }
            }
            Spacer()
            Menu {
                Button("Umbenennen …") { rename() }
                Button(account.kind == .ics ? "Anmeldung ändern …" : "Passwort erneuern …") { renew() }
                Button("Jetzt abgleichen") { syncNow() }
                Divider()
                Button("Entfernen …", role: .destructive) { remove() }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .fixedSize()
            .accessibilityLabel("Aktionen für \(account.name)")
        }
        .padding(.vertical, 2)
    }

    private var subtitle: String {
        [account.kind.displayName, account.username].compactMap { $0 }.joined(separator: " · ")
    }
}

/// Neues Konto; der Server prüft die Zugangsdaten sofort.
private struct AddAccountSheet: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    @State private var form = NewAccountForm()
    @State private var error: String?
    @State private var working = false

    var body: some View {
        NavigationStack {
            Form {
                Picker("Art", selection: $form.kind) {
                    ForEach(NewAccountForm.kinds, id: \.self) { kind in
                        Text(kind.displayName).tag(kind)
                    }
                }
                if form.needsURL {
                    Section("Adresse") {
                        TextField("Adresse", text: $form.url, prompt: Text(form.kind == .ics ? "https://… oder webcal://…" : "https://dav.example.org/"))
                            .plainTextInput()
                    }
                }
                Section {
                    TextField(form.credentialsOptional ? "Benutzername (optional)" : "Benutzername", text: $form.username)
                        .plainTextInput()
                    SecureField(form.credentialsOptional ? "Passwort (optional)" : "App-Passwort", text: $form.password)
                } header: {
                    Text("Anmeldung")
                } footer: {
                    if let hint = form.kind.passwordHint { Text(hint) }
                }
                Section {
                    TextField("Name (optional)", text: $form.name)
                } footer: {
                    Text("Ohne Namen nimmt der Server den des Anbieters bzw. des Abos.")
                }
                if let error {
                    Section {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red)
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Konto hinzufügen")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Abbrechen") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if working {
                        ProgressView().controlSize(.small)
                    } else {
                        Button("Hinzufügen") { add() }
                            .disabled(!form.isComplete)
                    }
                }
            }
        }
        .settingsSheetFrame()
    }

    private func add() {
        guard let account = form.build() else {
            error = form.missing
            return
        }
        working = true
        Task {
            let settings = session.settings
            let ok = await settings.addAccount(account)
            working = false
            if ok {
                dismiss()
            } else {
                error = settings.errorMessage
                settings.errorMessage = nil
            }
        }
    }
}

private struct RenameAccountSheet: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    let account: Account
    @State private var name: String
    @State private var error: String?

    init(account: Account) {
        self.account = account
        _name = State(initialValue: account.name)
    }

    var body: some View {
        NavigationStack {
            Form {
                TextField("Name", text: $name)
                if let error {
                    Text(error).foregroundStyle(.red)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Konto umbenennen")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Abbrechen") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Sichern") { save() }
                        .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
        .settingsSheetFrame()
    }

    private func save() {
        Task {
            let settings = session.settings
            if await settings.renameAccount(account.id, to: name) {
                dismiss()
            } else {
                error = settings.errorMessage
                settings.errorMessage = nil
            }
        }
    }
}

/// Neues (App-)Passwort; der Server prüft es beim Anbieter, bevor er es speichert.
private struct CredentialsSheet: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    let account: Account
    @State private var username: String
    @State private var password = ""
    @State private var error: String?
    @State private var working = false

    init(account: Account) {
        self.account = account
        _username = State(initialValue: account.username ?? "")
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Benutzername", text: $username)
                        .plainTextInput()
                    SecureField(account.kind == .icloud || account.kind == .fastmail ? "Neues App-Passwort" : "Neues Passwort", text: $password)
                } footer: {
                    if let hint = account.kind.passwordHint { Text(hint) }
                }
                if let error {
                    Section {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red)
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle(account.name)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Abbrechen") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if working {
                        ProgressView().controlSize(.small)
                    } else {
                        Button("Sichern") { save() }
                            .disabled(password.isEmpty && account.kind != .ics)
                    }
                }
            }
        }
        .settingsSheetFrame()
    }

    private func save() {
        working = true
        Task {
            let settings = session.settings
            let ok = await settings.updateCredentials(account.id, username: username, password: password)
            working = false
            if ok {
                dismiss()
            } else {
                error = settings.errorMessage
                settings.errorMessage = nil
            }
        }
    }
}
