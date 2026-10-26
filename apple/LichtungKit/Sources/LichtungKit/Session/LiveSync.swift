import Foundation

/// Merkt, ob seit dem letzten Blick eine neue Änderung in der Outbox liegt.
///
/// Nur neue IDs zählen: Eine Änderung, die offline hängen bleibt, soll nicht
/// bei jedem gespeicherten Stand einen weiteren Versuch auslösen (dafür sind
/// SSE-Neuverbindung und Backoff da).
public struct OutboxWatch: Sendable {
    private var seen: Set<String> = []

    public init() {}

    public mutating func hasNewChanges(in state: SyncState) -> Bool {
        let ids = Set(state.outbox.map(\.id))
        defer { seen = ids }
        return !ids.subtracting(seen).isEmpty
    }
}

/// Abgleich, solange die App offen ist (LIC-43, Vertrag §5).
///
/// Zwei Stränge laufen nebeneinander: der SSE-Strom (jeder gemeldete Cursor
/// löst einen Pull aus, Abbrüche mit `ReconnectBackoff`) und das Senden neuer
/// lokaler Änderungen. Beide enden bei `401` oder wenn die Aufgabe abgebrochen
/// wird (App im Hintergrund).
public enum LiveSync {
    public enum Stop: Hashable, Sendable {
        case cancelled
        /// Token widerrufen: neu anmelden.
        case unauthorized
    }

    public typealias Connect = @Sendable () async -> AsyncThrowingStream<Int, Error>
    public typealias Sleep = @Sendable (TimeInterval) async throws -> Void

    public static let defaultSleep: Sleep = { seconds in
        try await Task.sleep(nanoseconds: UInt64(max(0, seconds) * 1_000_000_000))
    }

    /// Läuft bis `401` oder Abbruch.
    public static func run(engine: SyncEngine, connect: @escaping Connect, sleep: @escaping Sleep = LiveSync.defaultSleep) async -> Stop {
        await withTaskGroup(of: Stop.self) { group in
            group.addTask { await follow(engine: engine, connect: connect, sleep: sleep) }
            group.addTask { await pushChanges(engine: engine) }
            let first = await group.next() ?? .cancelled
            group.cancelAll()
            return first
        }
    }

    /// SSE-Strang: erst abgleichen, dann den Strom lesen, bei Abbruch warten
    /// und von vorn.
    public static func follow(engine: SyncEngine, connect: @escaping Connect, sleep: @escaping Sleep = LiveSync.defaultSleep) async -> Stop {
        var backoff = ReconnectBackoff()
        while !Task.isCancelled {
            do {
                try await engine.sync()
                for try await cursor in await connect() {
                    // Nach dem Verbinden kommt sofort der aktuelle Cursor: Die
                    // Verbindung steht, der Backoff beginnt von vorn.
                    backoff.reset()
                    try await engine.cursorAnnounced(cursor)
                }
            } catch APIError.unauthorized {
                return .unauthorized
            } catch {
                // Abbruch, Server weg, offline: unten warten und neu verbinden.
            }
            if Task.isCancelled { return .cancelled }
            do {
                try await sleep(backoff.nextDelay())
            } catch {
                return .cancelled
            }
        }
        return .cancelled
    }

    /// Sende-Strang: Jede neue Änderung in der Outbox stößt einen Abgleich an.
    public static func pushChanges(engine: SyncEngine) async -> Stop {
        var watch = OutboxWatch()
        for await state in await engine.updates() {
            guard watch.hasNewChanges(in: state) else { continue }
            do {
                try await engine.sync()
            } catch APIError.unauthorized {
                return .unauthorized
            } catch {}
        }
        return .cancelled
    }
}
