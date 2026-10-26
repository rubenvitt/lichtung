import Foundation

// Einstellungen und Verwaltung (docs/api/sync-api.md §7). Diese Aufrufe gehen
// nicht über die Outbox, sie brauchen eine Verbindung.
//
// `me` und Geräte stehen seit LIC-46 vollständig im Vertrag, Tokens nur
// knapp. Felder sind hier großzügig optional, damit ältere Server oder kleine
// Abweichungen nicht zum Fehler führen.

public struct ServerInfo: Decodable, Hashable, Sendable {
    /// Gerät hinter dem Token (nur bei Geräte-Tokens).
    public struct DeviceRef: Decodable, Hashable, Sendable {
        public var id: String
        public var name: String?
    }

    public var userId: String?
    public var name: String?
    public var email: String?
    /// Art des Zugangs: „app:<Gerätename>“, „web“ oder „api:<Tokenname>“.
    public var auth: String?
    public var device: DeviceRef?
    /// Name und Version der Server-Software (§7, seit LIC-46; ältere Server
    /// liefern sie nicht).
    public var serverName: String?
    public var serverVersion: String?

    /// „lichtung-server 0.1.0“, `nil` ohne Version.
    public var serverDescription: String? {
        guard let serverVersion, !serverVersion.isEmpty else { return nil }
        return [serverName, serverVersion].compactMap { $0 }.joined(separator: " ")
    }
}

public struct Device: Decodable, Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var createdAt: Date?
    /// Letzte Nutzung des Tokens (Vertrag §7), `nil` vor der ersten.
    public var lastUsedAt: Date?
    /// Das Gerät, das gefragt hat.
    public var current: Bool?
}

/// API-Token für MCP-Clients (`lt_…`).
public struct APIToken: Decodable, Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var createdAt: Date?
    public var lastUsedAt: Date?
    /// Steht nur in der Antwort auf das Anlegen, danach nie wieder.
    public var token: String?
}

/// Neues Konto (`POST /api/accounts`).
public enum NewAccount: Sendable, Hashable {
    case fastmail(username: String, appPassword: String, name: String = "Fastmail")
    case icloud(username: String, appPassword: String, name: String = "iCloud")
    case ics(url: URL, name: String)
    /// Feed mit Basic-Auth (LIC-14).
    case icsWithLogin(url: URL, name: String, username: String, password: String)
    /// Beliebiger CalDAV-Server (LIC-7).
    case caldav(url: URL, username: String, password: String, name: String)

    var body: [String: String] {
        var b: [String: String]
        switch self {
        case let .fastmail(u, p, n): b = ["kind": "fastmail", "username": u, "password": p, "name": n]
        case let .icloud(u, p, n): b = ["kind": "icloud", "username": u, "password": p, "name": n]
        case let .ics(url, n): b = ["kind": "ics", "url": url.absoluteString, "name": n]
        case let .icsWithLogin(url, n, u, p): b = ["kind": "ics", "url": url.absoluteString, "name": n, "username": u, "password": p]
        case let .caldav(url, u, p, n): b = ["kind": "caldav", "url": url.absoluteString, "username": u, "password": p, "name": n]
        }
        // `name` ist optional (Vertrag §7): leer heißt „Standard des Servers“.
        if b["name"]?.trimmingCharacters(in: .whitespaces).isEmpty == true { b["name"] = nil }
        return b
    }
}

/// Änderbare Einstellungen eines Kalenders (`PATCH /api/calendars/{id}`).
public struct CalendarSettings: Encodable, Hashable, Sendable {
    /// `.clear` setzt die Farbe des Anbieters zurück.
    public var color: Patch<String>?
    public var hidden: Bool?
    /// Darf Claude über MCP hier schreiben (Spec §7)?
    public var mcpWrite: Bool?
    public var openingsBusy: Bool?
    public var allDayBlocks: Bool?
    public var position: Int?
    /// Arbeitskalender kennzeichnen (LIC-46). Kennzeichnen sperrt beim
    /// Server auch MCP-Schreiben, außer `mcpWrite` ist mitgesetzt.
    public var isWork: Bool?

    public init(color: Patch<String>? = nil, hidden: Bool? = nil, mcpWrite: Bool? = nil,
                openingsBusy: Bool? = nil, allDayBlocks: Bool? = nil, position: Int? = nil, isWork: Bool? = nil) {
        self.color = color
        self.hidden = hidden
        self.mcpWrite = mcpWrite
        self.openingsBusy = openingsBusy
        self.allDayBlocks = allDayBlocks
        self.position = position
        self.isWork = isWork
    }

    private enum CodingKeys: String, CodingKey { case color, hidden, mcpWrite, openingsBusy, allDayBlocks, position, isWork }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch color {
        case nil: break
        case .clear?: try c.encodeNil(forKey: .color)
        case let .set(v)?: try c.encode(v, forKey: .color)
        }
        try c.encodeIfPresent(hidden, forKey: .hidden)
        try c.encodeIfPresent(mcpWrite, forKey: .mcpWrite)
        try c.encodeIfPresent(openingsBusy, forKey: .openingsBusy)
        try c.encodeIfPresent(allDayBlocks, forKey: .allDayBlocks)
        try c.encodeIfPresent(position, forKey: .position)
        try c.encodeIfPresent(isWork, forKey: .isWork)
    }
}

public struct ManagementAPI: Sendable {
    public var sync: SyncAPI

    public init(sync: SyncAPI) { self.sync = sync }

    // MARK: Allgemein

    public func me() async throws -> ServerInfo {
        try await get("api/me")
    }

    // MARK: Konten

    public func accounts() async throws -> [Account] {
        try await list("api/accounts")
    }

    /// Der Server prüft die Zugangsdaten sofort: `422 invalid_credentials`
    /// bzw. `502 provider_unavailable` kommen als `APIError.server`.
    public func addAccount(_ account: NewAccount) async throws -> Account {
        try await send("POST", "api/accounts", body: account.body)
    }

    public func renameAccount(_ id: String, name: String) async throws -> Account {
        try await send("PATCH", "api/accounts/\(id)", body: ["name": name])
    }

    public func updateCredentials(_ id: String, username: String, password: String) async throws -> Account {
        try await send("PATCH", "api/accounts/\(id)", body: ["username": username, "password": password])
    }

    public func removeAccount(_ id: String) async throws {
        try await sendEmpty("DELETE", "api/accounts/\(id)")
    }

    public func syncAccountNow(_ id: String) async throws {
        try await sendEmpty("POST", "api/accounts/\(id)/sync")
    }

    // MARK: Kalender und Sets

    public func updateCalendar(_ id: String, _ settings: CalendarSettings) async throws -> CalendarInfo {
        try await send("PATCH", "api/calendars/\(id)", body: settings)
    }

    public func createCalendarSet(name: String, calendarIds: [String], position: Int = 0) async throws -> CalendarSet {
        try await send("POST", "api/calendar-sets", body: SetBody(name: name, calendarIds: calendarIds, position: position))
    }

    public func updateCalendarSet(_ id: String, name: String? = nil, calendarIds: [String]? = nil, position: Int? = nil) async throws -> CalendarSet {
        try await send("PATCH", "api/calendar-sets/\(id)", body: SetBody(name: name, calendarIds: calendarIds, position: position))
    }

    public func deleteCalendarSet(_ id: String) async throws {
        try await sendEmpty("DELETE", "api/calendar-sets/\(id)")
    }

    public func dismissConflict(_ id: String) async throws {
        try await sendEmpty("POST", "api/conflicts/\(id)/dismiss")
    }

    // MARK: Geräte und Tokens

    public func devices() async throws -> [Device] {
        try await list("api/devices")
    }

    public func removeDevice(_ id: String) async throws {
        try await sendEmpty("DELETE", "api/devices/\(id)")
    }

    /// Gerät umbenennen (LIC-46). Der Server kürzt Leerraum und benennt auch
    /// den OAuth-Client des Geräts um, eine neue Anmeldung behält den Namen.
    public func renameDevice(_ id: String, name: String) async throws -> Device {
        try await send("PATCH", "api/devices/\(id)", body: ["name": name])
    }

    public func tokens() async throws -> [APIToken] {
        try await list("api/tokens")
    }

    /// Das Ergebnis enthält `token` – nur dieses eine Mal.
    public func createToken(name: String) async throws -> APIToken {
        try await send("POST", "api/tokens", body: ["name": name])
    }

    public func revokeToken(_ id: String) async throws {
        try await sendEmpty("DELETE", "api/tokens/\(id)")
    }

    // MARK: Hilfen

    struct SetBody: Encodable {
        var name: String?
        var calendarIds: [String]?
        var position: Int?
    }

    /// Listen dürfen nackt (`[…]`) oder eingepackt kommen (`{"accounts": […]}` o. ä.).
    struct Listing<T: Decodable>: Decodable {
        var items: [T]
        init(from decoder: Decoder) throws {
            if let plain = try? [T](from: decoder) { items = plain; return }
            let c = try decoder.container(keyedBy: AnyKey.self)
            for key in c.allKeys {
                if let list = try? c.decode([T].self, forKey: key) { items = list; return }
            }
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Keine Liste in der Antwort"))
        }
    }

    struct AnyKey: CodingKey {
        var stringValue: String
        var intValue: Int? { nil }
        init(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { nil }
    }

    func get<T: Decodable>(_ path: String) async throws -> T {
        let r = try await sync.send(HTTPRequest(method: "GET", url: sync.endpoint(path)))
        return try sync.decode(T.self, r)
    }

    func list<T: Decodable>(_ path: String) async throws -> [T] {
        let listing: Listing<T> = try await get(path)
        return listing.items
    }

    func send<T: Decodable>(_ method: String, _ path: String, body: some Encodable) async throws -> T {
        let r = try await sync.send(HTTPRequest(
            method: method, url: sync.endpoint(path),
            headers: ["Content-Type": "application/json"], body: try APICoding.encoder.encode(body)
        ))
        return try sync.decode(T.self, r)
    }

    func sendEmpty(_ method: String, _ path: String) async throws {
        _ = try await sync.send(HTTPRequest(method: method, url: sync.endpoint(path)))
    }
}
