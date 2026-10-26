import Foundation

/// Eingaben im Blatt „Konto hinzufügen“ (LIC-43). Prüft, ob alles da ist,
/// und baut daraus den Aufruf `POST /api/accounts`.
public struct NewAccountForm: Hashable, Sendable {
    /// Arten, die sich in der App anlegen lassen (Microsoft folgt später).
    public static let kinds: [Account.Kind] = [.fastmail, .icloud, .caldav, .ics]

    public var kind: Account.Kind
    public var name = ""
    public var username = ""
    public var password = ""
    public var url = ""

    public init(kind: Account.Kind = .fastmail) { self.kind = kind }

    public var needsURL: Bool { kind == .caldav || kind == .ics }
    /// Bei ICS-Abos ist die Anmeldung freiwillig.
    public var credentialsOptional: Bool { kind == .ics }

    private func trimmed(_ s: String) -> String { s.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// Adresse für CalDAV/ICS: `webcal(s)://` nimmt der Server an (LIC-14),
    /// ohne Schema gilt `https://`.
    public var parsedURL: URL? {
        let t = trimmed(url)
        guard !t.isEmpty, !t.contains(where: { $0.isWhitespace }) else { return nil }
        let text = t.contains("://") ? t : "https://" + t
        guard let u = URL(string: text), let scheme = u.scheme?.lowercased(), u.host?.isEmpty == false else { return nil }
        let allowed = kind == .ics ? ["https", "http", "webcal", "webcals"] : ["https", "http"]
        return allowed.contains(scheme) ? u : nil
    }

    /// Was noch fehlt, als Satz; `nil`, wenn das Formular vollständig ist.
    public var missing: String? {
        if needsURL, parsedURL == nil { return trimmed(url).isEmpty ? "Adresse fehlt." : "Die Adresse ist ungültig." }
        if credentialsOptional {
            if trimmed(username).isEmpty, !password.isEmpty { return "Zum Passwort gehört ein Benutzername." }
            return nil
        }
        if trimmed(username).isEmpty { return "Benutzername fehlt." }
        if password.isEmpty { return "Passwort fehlt." }
        return nil
    }

    public var isComplete: Bool { missing == nil }

    public func build() -> NewAccount? {
        guard isComplete else { return nil }
        let n = trimmed(name)
        let u = trimmed(username)
        switch kind {
        case .fastmail: return .fastmail(username: u, appPassword: password, name: n.isEmpty ? "Fastmail" : n)
        case .icloud: return .icloud(username: u, appPassword: password, name: n.isEmpty ? "iCloud" : n)
        case .caldav:
            guard let link = parsedURL else { return nil }
            return .caldav(url: link, username: u, password: password, name: n)
        case .ics:
            guard let link = parsedURL else { return nil }
            return u.isEmpty ? .ics(url: link, name: n) : .icsWithLogin(url: link, name: n, username: u, password: password)
        case .microsoft, .unknown:
            return nil
        }
    }
}

extension CalendarSettings {
    /// Die Änderung lokal angewandt, für die sofortige Anzeige bis zur
    /// Antwort des Servers.
    public func applied(to calendar: CalendarInfo) -> CalendarInfo {
        var c = calendar
        switch color {
        case nil: break
        case .clear?: c.color = nil
        case let .set(v)?: c.color = v
        }
        if let hidden { c.hidden = hidden }
        if let mcpWrite { c.mcpWrite = mcpWrite }
        if let openingsBusy { c.openingsBusy = openingsBusy }
        if let allDayBlocks { c.allDayBlocks = allDayBlocks }
        if let position { c.position = position }
        if let isWork {
            // Wie der Server: Kennzeichnen sperrt Claude, außer ausdrücklich anders.
            if isWork, c.isWork != true, mcpWrite == nil { c.mcpWrite = false }
            c.isWork = isWork
        }
        return c
    }
}
