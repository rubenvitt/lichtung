import Foundation
@testable import LichtungKit
import Testing

/// Zählt Abrufe und hält sie an, bis der Test sie freigibt.
private actor Gate {
    private(set) var calls = 0
    private var waiting: [CheckedContinuation<Void, Never>] = []

    func enter() async -> Int {
        calls += 1
        let n = calls
        await withCheckedContinuation { waiting.append($0) }
        return n
    }

    func open() {
        for c in waiting { c.resume() }
        waiting = []
    }

    var waitingCount: Int { waiting.count }
}

private struct Failure: Error {}

@MainActor
@Suite("Gemeinsamer Abruf von /api/me (LIC-56)")
struct SharedFetchTests {
    /// Wartet, bis `count` Abrufe am Tor stehen.
    private func waitFor(_ gate: Gate, count: Int) async {
        for _ in 0..<1000 where await gate.waitingCount < count {
            await Task.yield()
            try? await Task.sleep(nanoseconds: 1_000_000)
        }
    }

    @Test func concurrentCallersShareOneRequest() async throws {
        let gate = Gate()
        let fetch = SharedFetch<Int>()
        // Aktivwerden und Einstellungen fragen zugleich.
        async let first = fetch.load { await gate.enter() }
        await waitFor(gate, count: 1)
        #expect(fetch.isRunning)
        async let second = fetch.load(maxAge: 30) { await gate.enter() }
        await Task.yield()
        await gate.open()
        let (a, b) = try await (first, second)
        #expect(a == 1)
        #expect(b == 1)
        let calls = await gate.calls
        #expect(calls == 1)
        #expect(!fetch.isRunning)
        #expect(fetch.value == 1)
    }

    @Test func freshResultIsReusedOnlyWithinMaxAge() async throws {
        let clock = Clock()
        let fetch = SharedFetch<Int>(now: { clock.now })
        let calls = FetchCounter()
        let next: @Sendable () async throws -> Int = { await calls.next() }

        #expect(try await fetch.load(next) == 1)
        clock.now += 10
        // Einstellungen kurz nach dem Aktivwerden: keine neue Anfrage.
        #expect(try await fetch.load(maxAge: 30, next) == 1)
        // maxAge 0 fragt immer neu.
        #expect(try await fetch.load(next) == 2)
        clock.now += 31
        #expect(try await fetch.load(maxAge: 30, next) == 3)
        fetch.invalidate()
        #expect(try await fetch.load(maxAge: 30, next) == 4)
        let total = await calls.value
        #expect(total == 4)
    }

    @Test func errorsAreNotRemembered() async throws {
        let fetch = SharedFetch<Int>()
        await #expect(throws: Failure.self) { try await fetch.load { throw Failure() } }
        #expect(fetch.value == nil)
        #expect(!fetch.isRunning)
        #expect(try await fetch.load(maxAge: 30) { 7 } == 7)
        // Ein späterer Fehler lässt das letzte Ergebnis stehen.
        await #expect(throws: Failure.self) { try await fetch.load { throw Failure() } }
        #expect(fetch.value == 7)
    }

    @Test func resetDropsRunningRequest() async throws {
        let gate = Gate()
        let fetch = SharedFetch<Int>()
        async let old = fetch.load { await gate.enter() }
        await waitFor(gate, count: 1)
        // Anderer Server: Die Antwort des alten zählt nicht mehr.
        fetch.reset()
        #expect(!fetch.isRunning)
        await gate.open()
        do {
            _ = try await old
            Issue.record("Nach reset darf der alte Abruf nichts liefern.")
        } catch {
            #expect(error is CancellationError)
        }
        #expect(fetch.value == nil)
        // Der nächste Aufruf fragt neu.
        #expect(try await fetch.load { 42 } == 42)
        #expect(fetch.value == 42)
    }
}

@MainActor
private final class Clock {
    var now = Date(timeIntervalSince1970: 1_000)
}

private actor FetchCounter {
    private(set) var value = 0
    func next() -> Int {
        value += 1
        return value
    }
}
