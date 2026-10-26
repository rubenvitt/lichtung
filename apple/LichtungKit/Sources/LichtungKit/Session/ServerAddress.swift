import Foundation

/// Adresse des eigenen Lichtung-Servers, wie sie beim ersten Start
/// eingegeben wird (LIC-43).
///
/// Aus „lichtung.example.org“, „https://lichtung.example.org/“ oder einer aus
/// der Doku kopierten „…/api“ wird immer dieselbe Basis-URL ohne Schrägstrich
/// am Ende. Von ihr hängen die Ressource der Anmeldung (`<basis>/api`, Vertrag
/// §2) und alle API-Pfade ab, deshalb muss sie eindeutig sein.
public enum ServerAddress {
    public enum Problem: Error, Hashable, Sendable, LocalizedError {
        case empty
        case invalid
        case unsupportedScheme(String)
        /// `http://` außerhalb des eigenen Rechners.
        case insecure
        /// Benutzername oder Passwort in der Adresse.
        case containsCredentials

        public var message: String {
            switch self {
            case .empty: "Bitte die Adresse des Lichtung-Servers eingeben."
            case .invalid: "Das sieht nicht nach einer Server-Adresse aus, z. B. „lichtung.example.org“."
            case let .unsupportedScheme(s): "„\(s)://“ geht nicht, die Adresse beginnt mit https://."
            case .insecure: "Ohne Verschlüsselung (http://) geht es nur auf dem eigenen Rechner. Bitte https:// verwenden."
            case .containsCredentials: "Zugangsdaten gehören nicht in die Adresse. Angemeldet wird im nächsten Schritt."
            }
        }

        public var errorDescription: String? { message }
    }

    /// Pfade, die Leute aus Doku oder Browser mitkopieren; sie gehören nicht
    /// zur Basis.
    private static let strippedSuffixes = ["/.well-known/oauth-authorization-server", "/api"]

    /// Prüft und vereinheitlicht eine Eingabe.
    public static func normalize(_ input: String) throws -> URL {
        let text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { throw Problem.empty }
        guard !text.contains(where: { $0.isWhitespace }) else { throw Problem.invalid }
        let withScheme = text.contains("://") ? text : "https://" + text
        guard var c = URLComponents(string: withScheme), let scheme = c.scheme?.lowercased() else { throw Problem.invalid }
        guard let host = c.host?.lowercased(), !host.isEmpty, !host.hasPrefix("."), !host.hasSuffix(".") else {
            throw Problem.invalid
        }
        guard c.user == nil, c.password == nil else { throw Problem.containsCredentials }
        switch scheme {
        case "https": break
        case "http": guard isLoopback(host) else { throw Problem.insecure }
        default: throw Problem.unsupportedScheme(scheme)
        }
        c.scheme = scheme
        if c.host != host { c.host = host }
        if (scheme == "https" && c.port == 443) || (scheme == "http" && c.port == 80) { c.port = nil }
        c.query = nil
        c.fragment = nil
        var path = c.path
        while path.hasSuffix("/") { path.removeLast() }
        for suffix in strippedSuffixes where path.lowercased().hasSuffix(suffix) {
            path.removeLast(suffix.count)
            while path.hasSuffix("/") { path.removeLast() }
        }
        c.path = path
        guard let url = c.url else { throw Problem.invalid }
        return url
    }

    /// Fehlertext zu einer Eingabe oder `nil`, wenn sie taugt (für die
    /// Anzeige unter dem Eingabefeld).
    public static func problem(with input: String) -> Problem? {
        do {
            _ = try normalize(input)
            return nil
        } catch let p as Problem {
            return p
        } catch {
            return .invalid
        }
    }

    /// Gleicher Server (Schema, Host, Port)? Pfade zählen nicht.
    public static func sameOrigin(_ a: URL, _ b: URL) -> Bool {
        a.scheme?.lowercased() == b.scheme?.lowercased()
            && a.host?.lowercased() == b.host?.lowercased()
            && effectivePort(a) == effectivePort(b)
    }

    /// Darf der lokale Stand nach einer Anmeldung bleiben? Nur, wenn es
    /// derselbe Server ist; sonst gehörten Cache und Outbox jemand anderem.
    public static func cacheBelongs(to cachedServer: URL?, newServer: URL) -> Bool {
        guard let cachedServer else { return false }
        return cachedServer.absoluteString == newServer.absoluteString
    }

    /// Kurzform für die Anzeige: „lichtung.example.org“, mit Port und Pfad,
    /// wenn vorhanden.
    public static func displayName(_ url: URL) -> String {
        var text = url.host ?? url.absoluteString
        if let port = url.port { text += ":\(port)" }
        if !url.path.isEmpty, url.path != "/" { text += url.path }
        return text
    }

    private static func isLoopback(_ host: String) -> Bool {
        ["localhost", "127.0.0.1", "::1", "[::1]"].contains(host) || host.hasSuffix(".localhost")
    }

    private static func effectivePort(_ url: URL) -> Int? {
        if let p = url.port { return p }
        switch url.scheme?.lowercased() {
        case "https": return 443
        case "http": return 80
        default: return nil
        }
    }
}
