// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import AuthenticationServices
import LichtungKit
import SwiftUI

/// Erster Start und neue Anmeldung (LIC-43): Server-Adresse, Gerätename,
/// dann das Browserfenster des Servers (Pocket-ID, Bestätigung).
///
/// `webAuthenticationSession` aus der Umgebung kümmert sich selbst um das
/// Fenster, an dem das Anmeldeblatt hängt; einen
/// `presentationContextProvider` wie bei `ASWebAuthenticationSession`
/// braucht es damit auf keiner Plattform.
struct LoginView: View {
    @Environment(AppSession.self) private var session
    @Environment(\.webAuthenticationSession) private var webAuthenticationSession
    @Environment(\.dismiss) private var dismiss
    /// Nach `401`: Adresse steht fest, Termine und Outbox bleiben.
    let isReauthentication: Bool

    init(isReauthentication: Bool = false) {
        self.isReauthentication = isReauthentication
    }

    private var busy: Bool { session.loginPhase.isBusy }

    var body: some View {
        @Bindable var session = session
        ScrollView {
            VStack(spacing: 24) {
                VStack(spacing: 8) {
                    Image(systemName: "calendar")
                        .font(.system(size: 44))
                        .foregroundStyle(.tint)
                    Text("Lichtung")
                        .font(.largeTitle.weight(.semibold))
                    Text(isReauthentication
                        ? "Dieses Gerät ist nicht mehr angemeldet. Termine und noch nicht gesendete Änderungen bleiben erhalten."
                        : "Melde dich an deinem eigenen Lichtung-Server an.")
                        .multilineTextAlignment(.center)
                        .foregroundStyle(.secondary)
                }

                VStack(alignment: .leading, spacing: 10) {
                    Text("Server").font(.headline)
                    TextField("Server-Adresse", text: $session.serverInput, prompt: Text("lichtung.example.org"))
                        .plainTextInput()
                        #if os(iOS)
                        .keyboardType(.URL)
                        #endif
                        .disabled(isReauthentication || busy)
                        .onSubmit { signIn() }
                    if !session.serverInput.isEmpty, let problem = ServerAddress.problem(with: session.serverInput) {
                        Text(problem.message)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    Text("Gerätename").font(.headline).padding(.top, 6)
                    TextField("Gerätename", text: $session.deviceName)
                        .disabled(busy)
                    Text("So erscheint dieses Gerät in der Geräteliste.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: 380)

                VStack(spacing: 12) {
                    Button {
                        signIn()
                    } label: {
                        HStack(spacing: 8) {
                            if busy { ProgressView().controlSize(.small) }
                            Text("Anmelden")
                        }
                        .frame(minWidth: 180)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .disabled(busy || ServerAddress.problem(with: session.serverInput) != nil)

                    if let status = session.loginPhase.statusText {
                        Text(status)
                            .font(.callout)
                            .multilineTextAlignment(.center)
                            .foregroundStyle(session.loginPhase.failure == nil ? Color.secondary : Color.red)
                            .frame(maxWidth: 380)
                    }

                    if isReauthentication {
                        Button("Später") { dismiss() }
                            .buttonStyle(.borderless)
                            .disabled(busy)
                    } else {
                        Button("Ohne Server ausprobieren") { session.startDemo() }
                            .buttonStyle(.borderless)
                            .disabled(busy)
                        Text("Mit erfundenen Terminen; nichts verlässt das Gerät.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .padding(32)
            .frame(maxWidth: .infinity)
        }
        #if os(macOS)
        .frame(minWidth: 460, minHeight: 520)
        #endif
    }

    private func signIn() {
        guard !busy, ServerAddress.problem(with: session.serverInput) == nil else { return }
        let auth = webAuthenticationSession
        Task {
            await session.signIn { url in
                // Ephemer: keine Cookies aus Safari, dafür keine Rückfrage
                // „… möchte sich anmelden“. Pocket-ID meldet per Passkey an.
                try await auth.authenticate(
                    using: url,
                    callbackURLScheme: PreparedLogin.callbackScheme,
                    preferredBrowserSession: .ephemeral
                )
            }
        }
    }
}

extension View {
    /// Eingabe ohne Autokorrektur und Großschreibung (Adressen, Benutzernamen).
    func plainTextInput() -> some View {
        #if os(iOS)
        self
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
        #else
        self.autocorrectionDisabled()
        #endif
    }
}
