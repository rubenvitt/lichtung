import Foundation

/// OAuth-Client dieser Installation bei einem Server (LIC-49, Vertrag §2):
/// einmal registriert, bei jeder weiteren Anmeldung wiederverwendet.
public struct RememberedClient: Codable, Hashable, Sendable {
    public var clientId: String
    /// Name, unter dem das Gerät beim Server heißt (`client_name` bzw. der
    /// zuletzt vergebene Name).
    public var name: String

    public init(clientId: String, name: String) {
        self.clientId = clientId
        self.name = name
    }
}

/// Ablage der gemerkten Clients je Server. In der App die UserDefaults, in
/// Tests der Speicher.
public protocol ClientStore: Sendable {
    func client(for server: URL) -> RememberedClient?
    func remember(_ client: RememberedClient, for server: URL)
    func forget(server: URL)
}

extension ClientStore {
    /// Schlüssel: die Basis-URL in der Form von `ServerAddress.normalize`,
    /// damit „https://Lichtung.example.org/“ und die Eingabe ohne Schema
    /// denselben Eintrag treffen.
    static func key(_ server: URL) -> String {
        (try? ServerAddress.normalize(server.absoluteString))?.absoluteString ?? server.absoluteString
    }
}

public final class InMemoryClientStore: ClientStore, @unchecked Sendable {
    private let lock = NSLock()
    private var clients: [String: RememberedClient] = [:]

    public init() {}

    public func client(for server: URL) -> RememberedClient? { lock.withLock { clients[Self.key(server)] } }
    public func remember(_ client: RememberedClient, for server: URL) { lock.withLock { clients[Self.key(server)] = client } }
    public func forget(server: URL) { _ = lock.withLock { clients.removeValue(forKey: Self.key(server)) } }
}

/// Gemerkte Clients in den UserDefaults. Die `client_id` eines öffentlichen
/// Clients ist kein Geheimnis; anders als die Keychain verschwinden die
/// UserDefaults mit der App, und der Vertrag sieht einen Client pro
/// Installation vor.
public final class UserDefaultsClientStore: ClientStore, @unchecked Sendable {
    public static let defaultsKey = "session.oauthClients"

    private let defaults: UserDefaults
    private let lock = NSLock()

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    private func load() -> [String: RememberedClient] {
        guard let data = defaults.data(forKey: Self.defaultsKey),
              let all = try? JSONDecoder().decode([String: RememberedClient].self, from: data)
        else { return [:] }
        return all
    }

    private func store(_ all: [String: RememberedClient]) {
        guard let data = try? JSONEncoder().encode(all) else { return }
        defaults.set(data, forKey: Self.defaultsKey)
    }

    public func client(for server: URL) -> RememberedClient? {
        lock.withLock { load()[Self.key(server)] }
    }

    public func remember(_ client: RememberedClient, for server: URL) {
        lock.withLock {
            var all = load()
            all[Self.key(server)] = client
            store(all)
        }
    }

    public func forget(server: URL) {
        lock.withLock {
            var all = load()
            guard all.removeValue(forKey: Self.key(server)) != nil else { return }
            store(all)
        }
    }
}
