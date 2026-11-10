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
                VStack(spacing: 12) {
                    LichtungMark(size: 88)
                        .padding(.bottom, 4)
                    Text("Lichtung")
                        .font(.largeTitle.weight(.semibold))
                    Text(isReauthentication
                        ? "Dieses Gerät ist nicht mehr angemeldet. Termine und noch nicht gesendete Änderungen bleiben erhalten."
                        : "Melde dich an deinem eigenen Lichtung-Server an.")
                        .multilineTextAlignment(.center)
                        .foregroundStyle(Theme.text2)
                }

                VStack(alignment: .leading, spacing: 10) {
                    SectionLabel(text: "Server")
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
                            .foregroundStyle(Theme.text2)
                    }
                    SectionLabel(text: "Gerätename").padding(.top, 8)
                    TextField("Gerätename", text: $session.deviceName)
                        .disabled(busy)
                    Text("So erscheint dieses Gerät in der Geräteliste.")
                        .font(.footnote)
                        .foregroundStyle(Theme.text2)
                }
                .textFieldStyle(.roundedBorder)
                .padding(20)
                .background {
                    RoundedRectangle(cornerRadius: Theme.cardRadius + 4, style: .continuous)
                        .fill(.background.secondary)
                }
                .overlay {
                    RoundedRectangle(cornerRadius: Theme.cardRadius + 4, style: .continuous)
                        .strokeBorder(Theme.line, lineWidth: 1)
                }
                .frame(maxWidth: 420)

                VStack(spacing: 12) {
                    Button {
                        signIn()
                    } label: {
                        HStack(spacing: 8) {
                            if busy { ProgressView().controlSize(.small) }
                            Text("Anmelden").fontWeight(.semibold)
                        }
                        .frame(minWidth: 220)
                    }
                    .buttonStyle(Theme.PrimaryButtonStyle())
                    .keyboardShortcut(.defaultAction)
                    .disabled(busy || ServerAddress.problem(with: session.serverInput) != nil)

                    if let status = session.loginPhase.statusText {
                        Text(status)
                            .font(.callout)
                            .multilineTextAlignment(.center)
                            .foregroundStyle(session.loginPhase.failure == nil ? Theme.text2 : Color.red)
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
                            .foregroundStyle(Theme.text2)
                    }
                }
            }
            .padding(32)
            .frame(maxWidth: .infinity)
        }
        .background(Theme.shade)
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

/// Marke: Sonne über dem Horizont auf der Licht-Farbe.
struct LichtungMark: View {
    var size: CGFloat = 64

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.26, style: .continuous)
            .fill(Theme.light)
            .overlay {
                Image(systemName: "sun.horizon.fill")
                    .font(.system(size: size * 0.46, weight: .semibold))
                    .foregroundStyle(Theme.onLight)
            }
            .frame(width: size, height: size)
            .accessibilityHidden(true)
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
