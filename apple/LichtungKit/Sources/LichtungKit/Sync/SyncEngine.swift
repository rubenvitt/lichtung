import Foundation

/// Hält den lokalen Stand eines Geräts und gleicht ihn mit dem Server ab.
///
/// Ablauf (Vertrag §4–6): erst ausstehende Änderungen senden, dann ziehen,
/// bis `has_more` falsch ist. Jeder Schritt wird sofort gespeichert, damit
/// ein Absturz keine Änderung verliert.
public actor SyncEngine {
    public private(set) var state: SyncState
    public let api: SyncAPI
    public let store: CacheStore
    public var deviceTimeZone: TimeZone
    public var cacheWindow: CacheWindow
    private let now: @Sendable () -> Date
    private var observers: [UUID: @Sendable (SyncState) -> Void] = [:]
    private var running = false
    /// Rückgängig/Wiederholen für Änderungen über `perform` (LIC-23).
    public internal(set) var undoHistory = UndoHistory()

    public init(
        api: SyncAPI,
        store: CacheStore,
        deviceTimeZone: TimeZone = .current,
        cacheWindow: CacheWindow = CacheWindow(),
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.api = api
        self.store = store
        self.deviceTimeZone = deviceTimeZone
        self.cacheWindow = cacheWindow
        self.now = now
        self.state = (try? store.load()) ?? SyncState()
    }

    /// Meldet jede Änderung des Stands (für View-Modelle). Gibt einen Schlüssel
    /// zum Abmelden zurück.
    @discardableResult
    public func observe(_ handler: @escaping @Sendable (SyncState) -> Void) -> UUID {
        let id = UUID()
        observers[id] = handler
        handler(state)
        return id
    }

    public func removeObserver(_ id: UUID) {
        observers[id] = nil
    }

    // MARK: Lokale Änderungen

    /// Legt eine Änderung in die Outbox und zeigt sie sofort an.
    /// Gibt die ID des betroffenen (ggf. lokalen) Events zurück.
    @discardableResult
    public func enqueue(_ operation: Mutation.Operation) -> String? {
        let mutation = Mutation(operation)
        let id = state.enqueue(mutation, now: now())
        commit()
        return id
    }

    /// Bequemer Weg für Änderungen an einem bestehenden Event: setzt die
    /// aktuelle Basisversion selbst.
    @discardableResult
    public func update(eventId: String, scope: EditScope = .all, occurrence: DateValue? = nil, changes: EventChanges) -> String? {
        enqueue(.updateEvent(eventId: eventId, baseVersion: state.baseVersion(of: eventId), scope: scope, occurrence: occurrence, changes: changes))
    }

    @discardableResult
    public func delete(eventId: String, scope: EditScope = .all, occurrence: DateValue? = nil) -> String? {
        enqueue(.deleteEvent(eventId: eventId, baseVersion: state.baseVersion(of: eventId), scope: scope, occurrence: occurrence))
    }

    public func dismissNotice(_ id: String) {
        state.dismissNotice(id)
        commit()
    }

    // MARK: Abgleich

    public enum Outcome: Hashable, Sendable {
        case synced
        /// Ein Schritt konnte nicht laufen (offline, Server weg). Später erneut.
        case deferred(APIError)
    }

    /// Senden, dann Ziehen. Gleichzeitige Aufrufe laufen nicht doppelt.
    /// `unauthorized` wird geworfen (neu anmelden), alles Vorübergehende kommt
    /// als `.deferred` zurück.
    @discardableResult
    public func sync() async throws -> Outcome {
        guard !running else { return .synced }
        running = true
        defer { running = false }
        do {
            try await pushPending()
            try await pullAll()
            return .synced
        } catch let e as APIError {
            if e == .unauthorized { throw e }
            return .deferred(e)
        }
    }

    /// Reaktion auf einen Cursor aus SSE oder einer Push-Nachricht.
    public func cursorAnnounced(_ cursor: Int) async throws {
        if cursor > state.cursor { try await sync() }
    }

    func pushPending() async throws {
        while true {
            let batch = state.nextBatch(now: now())
            guard !batch.isEmpty else { return }
            let results = try await api.push(batch)
            state.handle(results, now: now())
            commit()
            // Nach einem `retry` wartet der Rest; nichts mehr zu tun.
            if results.contains(where: { $0.status == .retry }) || results.isEmpty { return }
        }
    }

    func pullAll() async throws {
        var expiredOnce = false
        while true {
            do {
                let page = try await api.pull(since: state.cursor)
                var next = state
                next.apply(page)
                state = next
                if !page.hasMore {
                    state.prune(to: cacheWindow.interval(around: now(), calendar: .lichtung(timeZone: deviceTimeZone)), deviceTimeZone: deviceTimeZone)
                }
                commit()
                if !page.hasMore { return }
            } catch APIError.cursorExpired where !expiredOnce {
                expiredOnce = true
                state.resetMirror()
                commit()
            }
        }
    }

    private func commit() {
        try? store.save(state)
        let snapshot = state
        for o in observers.values { o(snapshot) }
    }

    // MARK: Abfragen

    /// Server-Stand plus ausstehende Änderungen.
    public func visibleEvents() -> [String: Event] {
        state.visibleEvents(deviceTimeZone: deviceTimeZone)
    }
}
