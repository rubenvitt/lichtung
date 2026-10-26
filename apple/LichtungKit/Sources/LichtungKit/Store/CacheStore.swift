import Foundation

/// Ablage des lokalen Stands (Spec §12: Speichertechnik ist Builder-Sache).
public protocol CacheStore: Sendable {
    func load() throws -> SyncState?
    func save(_ state: SyncState) throws
    func clear() throws
}

/// Hält den Stand nur im Speicher (Tests, Vorschauen).
public final class InMemoryCacheStore: CacheStore, @unchecked Sendable {
    private let lock = NSLock()
    private var state: SyncState?

    public init(_ state: SyncState? = nil) { self.state = state }

    public func load() throws -> SyncState? { lock.withLock { state } }
    public func save(_ state: SyncState) throws { lock.withLock { self.state = state } }
    public func clear() throws { lock.withLock { state = nil } }
}

/// Speichert den Stand als eine JSON-Datei, atomar ersetzt.
///
/// Für eine Person mit einigen tausend Terminen reicht das: eine Datei von
/// wenigen MB, in Millisekunden gelesen. Wird es zu groß, kommt SQLite.
public struct FileCacheStore: CacheStore {
    /// 2: Buchungslinks im Stand (LIC-35). Ein älterer Stand hat die Links,
    /// die vorher schon angelegt waren, übersprungen; deshalb neu ziehen.
    public static let formatVersion = 2

    public let url: URL

    public init(url: URL) { self.url = url }

    private struct Envelope: Codable {
        var formatVersion: Int
        var state: SyncState
    }

    public func load() throws -> SyncState? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        let data = try Data(contentsOf: url)
        let envelope = try JSONDecoder().decode(Envelope.self, from: data)
        // Anderes Format: neu vom Server ziehen, aber die Outbox nicht verlieren.
        guard envelope.formatVersion == Self.formatVersion else {
            var fresh = SyncState()
            fresh.outbox = envelope.state.outbox
            return fresh
        }
        return envelope.state
    }

    public func save(_ state: SyncState) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let data = try JSONEncoder().encode(Envelope(formatVersion: Self.formatVersion, state: state))
        try data.write(to: url, options: .atomic)
    }

    public func clear() throws {
        if FileManager.default.fileExists(atPath: url.path) {
            try FileManager.default.removeItem(at: url)
        }
    }
}

/// Zeitraum, den das Gerät vorhält (Spec §5: z. B. −3 bis +12 Monate).
public struct CacheWindow: Hashable, Sendable {
    public var monthsBack: Int
    public var monthsAhead: Int

    public init(monthsBack: Int = 3, monthsAhead: Int = 12) {
        self.monthsBack = monthsBack
        self.monthsAhead = monthsAhead
    }

    public func interval(around now: Date, calendar: Calendar) -> DateInterval {
        let start = calendar.date(byAdding: .month, value: -monthsBack, to: calendar.startOfDay(for: now))!
        let end = calendar.date(byAdding: .month, value: monthsAhead, to: calendar.startOfDay(for: now))!
        return DateInterval(start: start, end: end)
    }
}

extension SyncState {
    /// Entfernt Einzeltermine außerhalb des Fensters. Serien bleiben immer
    /// (sie können ins Fenster hineinreichen), ebenso alles, woran eine
    /// ausstehende Änderung hängt.
    public mutating func prune(to window: DateInterval, deviceTimeZone tz: TimeZone) {
        let pinned = Set(outbox.compactMap(\.mutation.eventId))
        events = events.filter { id, e in
            if e.isRecurring || pinned.contains(id) { return true }
            let start = e.details.start.instant(deviceTimeZone: tz)
            let end = e.details.end.instant(deviceTimeZone: tz)
            return end > window.start && start < window.end
        }
    }
}
