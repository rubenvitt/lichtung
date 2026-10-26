import Foundation

/// Ablage für Geheimnisse. In der App die Keychain, in Tests der Speicher.
public protocol SecretStore: Sendable {
    func read(_ key: String) throws -> String?
    func write(_ value: String, for key: String) throws
    func delete(_ key: String) throws
}

public final class InMemorySecretStore: SecretStore, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: String]

    public init(_ values: [String: String] = [:]) { self.values = values }

    public func read(_ key: String) throws -> String? { lock.withLock { values[key] } }
    public func write(_ value: String, for key: String) throws { lock.withLock { values[key] = value } }
    public func delete(_ key: String) throws { lock.withLock { values[key] = nil } }
}

/// Geräte-Token aus einer `SecretStore` (LIC-43), ersetzt `StaticToken`.
///
/// Liest das Token einmal und hält es dann im Speicher. Lehnt der Server es
/// ab (`401`), wird es gelöscht und `onRejected` gerufen: Die App zeigt dann
/// „neu anmelden“. Cache und Outbox berührt das nicht (Vertrag §2).
public final class StoredDeviceToken: TokenProvider, @unchecked Sendable {
    public static let key = "device-token"

    private let store: SecretStore
    private let lock = NSLock()
    private var cached: String??
    private var rejectionHandler: (@Sendable () -> Void)?

    public init(store: SecretStore, onRejected: (@Sendable () -> Void)? = nil) {
        self.store = store
        self.rejectionHandler = onRejected
    }

    /// Für den Fall, dass der Empfänger erst nach dem Provider entsteht.
    public func setRejectionHandler(_ handler: (@Sendable () -> Void)?) {
        lock.withLock { rejectionHandler = handler }
    }

    public var current: String? {
        lock.withLock {
            if let cached { return cached }
            let value = try? store.read(Self.key)
            cached = .some(value)
            return value
        }
    }

    public var hasToken: Bool { current != nil }

    public func deviceToken() async -> String? { current }

    public func save(_ token: String) throws {
        try store.write(token, for: Self.key)
        lock.withLock { cached = .some(token) }
    }

    public func clear() throws {
        lock.withLock { cached = .some(nil) }
        try store.delete(Self.key)
    }

    /// Nur wenn das abgelehnte Token noch das aktuelle ist: Eine späte `401`
    /// zum alten Token darf eine frische Anmeldung nicht wieder löschen.
    public func tokenRejected(_ token: String) async {
        let handler: (@Sendable () -> Void)? = lock.withLock {
            let now: String?
            if let cached { now = cached } else { now = try? store.read(Self.key) }
            guard now == token else { return nil }
            cached = .some(nil)
            try? store.delete(Self.key)
            return rejectionHandler
        }
        handler?()
    }
}
