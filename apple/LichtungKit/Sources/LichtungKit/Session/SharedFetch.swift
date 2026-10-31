import Foundation

/// Ein Abruf, den mehrere Stellen teilen (LIC-56): Wer fragt, während er
/// läuft, wartet auf dasselbe Ergebnis; ein Ergebnis, das jünger ist als
/// `maxAge`, gilt ohne neue Anfrage.
///
/// In der App für `GET /api/me`: Der Abgleich des Gerätenamens beim
/// Aktivwerden und die Einstellungen fragen den Server nur einmal.
///
/// Fehler werden nicht gemerkt; der nächste Aufruf fragt neu. Nach `reset()`
/// (anderer Server, Abmelden) liefert ein noch laufender Abruf nichts mehr,
/// seine Wartenden bekommen `CancellationError`.
@MainActor
public final class SharedFetch<Value: Sendable> {
    public private(set) var value: Value?
    public private(set) var fetchedAt: Date?
    private var running: Task<Value, Error>?
    private var generation = 0
    private let now: @MainActor () -> Date

    public init(now: @escaping @MainActor () -> Date = { Date() }) {
        self.now = now
    }

    /// Läuft gerade ein Abruf?
    public var isRunning: Bool { running != nil }

    /// Das gemerkte Ergebnis, wenn es höchstens `maxAge` Sekunden alt ist,
    /// sonst der laufende oder ein neuer Abruf. `maxAge: 0` fragt immer neu,
    /// außer es läuft schon einer.
    public func load(maxAge: TimeInterval = 0, _ fetch: @escaping @Sendable () async throws -> Value) async throws -> Value {
        if maxAge > 0, let value, let fetchedAt, now().timeIntervalSince(fetchedAt) <= maxAge {
            return value
        }
        let gen = generation
        let task: Task<Value, Error>
        if let running {
            task = running
        } else {
            task = Task { try await fetch() }
            running = task
        }
        do {
            let result = try await task.value
            guard gen == generation else { throw CancellationError() }
            if running == task {
                running = nil
                value = result
                fetchedAt = now()
            }
            return result
        } catch {
            if gen == generation, running == task { running = nil }
            throw error
        }
    }

    /// Vergisst nur das Ergebnis (es ist überholt); ein laufender Abruf
    /// bleibt und gilt.
    public func invalidate() {
        value = nil
        fetchedAt = nil
    }

    /// Vergisst Ergebnis und laufenden Abruf.
    public func reset() {
        generation += 1
        running = nil
        value = nil
        fetchedAt = nil
    }
}
