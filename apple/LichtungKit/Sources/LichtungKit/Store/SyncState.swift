import Foundation

/// Ausstehende Änderung in der Outbox.
public struct PendingMutation: Codable, Hashable, Sendable, Identifiable {
    public var mutation: Mutation
    public var enqueuedAt: Date
    /// Frühester nächster Sendeversuch nach einem `retry`.
    public var notBefore: Date?
    public var attempts: Int

    public var id: String { mutation.id }

    public init(_ mutation: Mutation, enqueuedAt: Date) {
        self.mutation = mutation
        self.enqueuedAt = enqueuedAt
        self.attempts = 0
    }
}

/// Meldung an die Person, z. B. eine abgelehnte Änderung (Spec §4: nichts
/// verschwindet still).
public struct SyncNotice: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var message: String
    public var code: String?
    public var createdAt: Date
}

/// Lokaler Stand eines Geräts: Spiegel des Servers plus Outbox.
///
/// Reine Werte, keine Ein-/Ausgabe. Was angezeigt wird (`visibleEvents`), ist
/// der Server-Stand mit den ausstehenden Änderungen darübergelegt. Wird eine
/// Änderung abgelehnt, fällt sie aus der Outbox und ist damit von selbst
/// zurückgenommen.
public struct SyncState: Codable, Hashable, Sendable {
    public static let placeholderPrefix = "local:"

    public var cursor: Int = 0
    public var accounts: [String: Account] = [:]
    public var calendars: [String: CalendarInfo] = [:]
    public var calendarSets: [String: CalendarSet] = [:]
    /// Server-Stand der Events, ohne lokale Änderungen.
    public var events: [String: Event] = [:]
    public var conflicts: [String: Conflict] = [:]
    public var outbox: [PendingMutation] = []
    public var notices: [SyncNotice] = []
    /// Mutations-ID eines `create_event` → ID des Events beim Server.
    public var createdEventIds: [String: String] = [:]

    public init() {}

    // MARK: - Pull

    /// Wendet eine Seite des Pulls an. Atomar: entweder alles oder nichts,
    /// weil `SyncState` ein Wert ist und erst nach Erfolg zurückgeschrieben wird.
    public mutating func apply(_ page: PullPage) {
        for change in page.changes {
            switch change {
            case let .upsertAccount(a): accounts[a.id] = a
            case let .upsertCalendar(c): calendars[c.id] = c
            case let .upsertCalendarSet(s): calendarSets[s.id] = s
            case let .upsertEvent(e): upsertServerEvent(e)
            case let .upsertConflict(c): conflicts[c.id] = c
            case let .delete(type, id):
                switch type {
                case "account": accounts[id] = nil
                case "calendar": calendars[id] = nil
                case "calendar_set": calendarSets[id] = nil
                case "event": events[id] = nil
                case "conflict": conflicts[id] = nil
                default: break
                }
            case .unknown: break
            }
        }
        cursor = page.cursor
    }

    /// Ältere Versionen ersetzen keine neueren (Push-Antwort und Pull können
    /// sich überholen; doppelt anwenden ist harmlos).
    mutating func upsertServerEvent(_ e: Event) {
        if let old = events[e.id], old.version > e.version { return }
        events[e.id] = e
    }

    /// Cursor zu alt (`410 cursor_expired`): Spiegel leeren, Outbox behalten.
    public mutating func resetMirror() {
        cursor = 0
        accounts = [:]
        calendars = [:]
        calendarSets = [:]
        events = [:]
        conflicts = [:]
    }

    // MARK: - Outbox

    /// Legt eine Änderung in die Outbox. Gibt die ID des (ggf. lokalen) Events zurück.
    @discardableResult
    public mutating func enqueue(_ mutation: Mutation, now: Date) -> String? {
        outbox.append(PendingMutation(mutation, enqueuedAt: now))
        if case .createEvent = mutation.operation { return Self.placeholderPrefix + mutation.id }
        return mutation.eventId
    }

    /// Version, gegen die eine neue Änderung an `eventId` gelten soll.
    public func baseVersion(of eventId: String) -> Int {
        events[eventId]?.version ?? 0
    }

    /// Nächster Stapel zum Senden: die Outbox von vorn, höchstens `limit`
    /// Mutationen. Halt vor einer Mutation, die
    /// - auf ein Event zielt, das im Stapel schon vorkommt (die zweite braucht
    ///   die neue `version` als `base_version`),
    /// - auf ein noch nicht angelegtes lokales Event zielt,
    /// - noch warten muss (`notBefore`).
    public func nextBatch(now: Date, limit: Int = 100) -> [Mutation] {
        var batch: [Mutation] = []
        var touched: Set<String> = []
        for p in outbox {
            if batch.count >= limit { break }
            if let nb = p.notBefore, nb > now { break }
            if let eid = p.mutation.eventId {
                if eid.hasPrefix(Self.placeholderPrefix) || touched.contains(eid) { break }
                touched.insert(eid)
            }
            batch.append(p.mutation)
        }
        return batch
    }

    /// Verarbeitet die Antwort auf einen Push (Vertrag §6 „Antwort“).
    public mutating func handle(_ results: [MutationResult], now: Date) {
        for r in results {
            guard let idx = outbox.firstIndex(where: { $0.mutation.id == r.id }) else { continue }
            let pending = outbox[idx]
            switch r.status {
            case .applied:
                outbox.remove(at: idx)
                for e in r.events ?? [] { upsertServerEvent(e) }
                for id in r.deletedEventIds ?? [] { events[id] = nil }
                if case .createEvent = pending.mutation.operation, let created = r.events?.first {
                    createdEventIds[pending.mutation.id] = created.id
                    rewrite(eventId: Self.placeholderPrefix + pending.mutation.id, to: created.id, version: created.version)
                } else if let eid = pending.mutation.eventId, let v = events[eid]?.version {
                    rewrite(eventId: eid, to: eid, version: v)
                }
                // Serie geteilt („dieser und folgende“): Folgeänderungen bleiben am alten Event.
            case .conflict:
                outbox.remove(at: idx)
                if let c = r.conflict { conflicts[c.id] = c }
                if let current = r.events, !current.isEmpty {
                    for e in current { events[e.id] = e }
                    if let eid = pending.mutation.eventId, let v = events[eid]?.version {
                        rewrite(eventId: eid, to: eid, version: v)
                    }
                } else if let eid = pending.mutation.eventId {
                    events[eid] = nil
                }
            case .rejected:
                outbox.remove(at: idx)
                notices.append(SyncNotice(
                    id: r.id,
                    message: r.error?.message ?? "Die Änderung wurde vom Server abgelehnt.",
                    code: r.error?.code,
                    createdAt: now
                ))
                // Folgeänderungen an einem nie angelegten Event sind gegenstandslos.
                if case .createEvent = pending.mutation.operation {
                    let local = Self.placeholderPrefix + pending.mutation.id
                    outbox.removeAll { $0.mutation.eventId == local }
                }
            case .retry:
                var p = pending
                p.attempts += 1
                let wait = r.retryAfterSeconds.map(TimeInterval.init) ?? Self.backoff(attempt: p.attempts)
                p.notBefore = now.addingTimeInterval(wait)
                outbox[idx] = p
            }
        }
    }

    /// Wartezeit nach dem n-ten Fehlversuch ohne Vorgabe vom Server: 5 s, 10 s, … bis 10 min.
    static func backoff(attempt: Int) -> TimeInterval {
        min(600, 5 * pow(2, Double(max(0, attempt - 1))))
    }

    /// Ersetzt in den restlichen Mutationen eine Event-ID und setzt die neue Basisversion.
    mutating func rewrite(eventId old: String, to new: String, version: Int) {
        for i in outbox.indices {
            let m = outbox[i].mutation
            switch m.operation {
            case let .updateEvent(eid, _, scope, occ, changes) where eid == old:
                outbox[i].mutation.operation = .updateEvent(eventId: new, baseVersion: version, scope: scope, occurrence: occ, changes: changes)
            case let .deleteEvent(eid, _, scope, occ) where eid == old:
                outbox[i].mutation.operation = .deleteEvent(eventId: new, baseVersion: version, scope: scope, occurrence: occ)
            case let .respond(eid, occ, status, comment) where eid == old:
                outbox[i].mutation.operation = .respond(eventId: new, occurrence: occ, status: status, comment: comment)
            default:
                break
            }
        }
    }

    public mutating func dismissNotice(_ id: String) {
        notices.removeAll { $0.id == id }
    }

    /// ID, unter der ein Event jetzt zu finden ist (lokale Platzhalter werden
    /// nach dem Anlegen zur Server-ID).
    public func currentId(for eventId: String) -> String {
        guard eventId.hasPrefix(Self.placeholderPrefix) else { return eventId }
        return createdEventIds[String(eventId.dropFirst(Self.placeholderPrefix.count))] ?? eventId
    }

    // MARK: - Anzeige

    /// Server-Stand mit allen ausstehenden Änderungen darübergelegt.
    public func visibleEvents(deviceTimeZone: TimeZone) -> [String: Event] {
        var result = events
        for p in outbox {
            OptimisticApply.apply(p.mutation, to: &result, deviceTimeZone: deviceTimeZone)
        }
        return result
    }

    /// Hat dieses Event noch nicht gesendete Änderungen?
    public func isPending(_ eventId: String) -> Bool {
        outbox.contains { $0.mutation.eventId == eventId || Self.placeholderPrefix + $0.mutation.id == eventId }
    }
}
