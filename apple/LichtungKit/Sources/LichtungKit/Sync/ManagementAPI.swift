import Foundation

// Einstellungen und Verwaltung (docs/api/sync-api.md §7). Diese Aufrufe gehen
// nicht über die Outbox, sie brauchen eine Verbindung.
//
// Der Vertrag beschreibt `me`, Geräte und Tokens nur knapp. Felder, die er
// nicht nennt, sind hier optional, damit kleine Abweichungen des Servers
// nicht zum Fehler führen.

public struct ServerInfo: Decodable, Hashable, Sendable {
    public var name: String?
    public var email: String?
    public var serverName: String?
    public var serverVersion: String?
}

public struct Device: Decodable, Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var createdAt: Date?
    public var lastSeenAt: Date?
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

    var body: [String: String] {
        switch self {
        case let .fastmail(u, p, n): ["kind": "fastmail", "username": u, "password": p, "name": n]
        case let .icloud(u, p, n): ["kind": "icloud", "username": u, "password": p, "name": n]
        case let .ics(url, n): ["kind": "ics", "url": url.absoluteString, "name": n]
        }
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

    public init(color: Patch<String>? = nil, hidden: Bool? = nil, mcpWrite: Bool? = nil,
                openingsBusy: Bool? = nil, allDayBlocks: Bool? = nil, position: Int? = nil) {
        self.color = color
        self.hidden = hidden
        self.mcpWrite = mcpWrite
        self.openingsBusy = openingsBusy
        self.allDayBlocks = allDayBlocks
        self.position = position
    }

    private enum CodingKeys: String, CodingKey { case color, hidden, mcpWrite, openingsBusy, allDayBlocks, position }

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
