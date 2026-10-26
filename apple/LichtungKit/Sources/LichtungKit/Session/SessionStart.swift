import Foundation

/// Womit die App startet (LIC-43). Gespeichert sind nur die Server-Adresse
/// und die Wahl „Ohne Server ausprobieren“ (UserDefaults), das Token liegt in
/// der Keychain, der Stand in der Cache-Datei.
public enum SessionStart: Hashable, Sendable {
    /// Anmeldemaske, ggf. mit vorbelegter Adresse.
    case login(prefill: String)
    /// Erfundene Daten, nichts verlässt das Gerät.
    case demo
    /// Angemeldet: echter Abgleich.
    case server(URL)
    /// Token fehlt (abgelehnt oder gelöscht), aber ein lokaler Stand ist da:
    /// Kalender lesbar zeigen, Hinweis „neu anmelden“. Outbox bleibt.
    case reauthenticate(URL)

    public static let demoMode = "demo"
    public static let serverMode = "server"

    public static func decide(mode: String?, serverURL: String?, hasToken: Bool, hasCache: Bool) -> SessionStart {
        if mode == demoMode { return .demo }
        let url = serverURL.flatMap { try? ServerAddress.normalize($0) }
        guard let url else { return .login(prefill: serverURL ?? "") }
        if hasToken { return .server(url) }
        if hasCache { return .reauthenticate(url) }
        return .login(prefill: ServerAddress.displayName(url))
    }
}
