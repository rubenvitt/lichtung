import Foundation
@testable import LichtungKit
import Testing

/// Gespielter Server nach Pfad: Pull liefert leere Seiten mit steigendem
/// Cursor, Push nimmt jede Mutation an. `onPush` läuft vor der Antwort.
final class RoutingTransport: HTTPTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var requests: [HTTPRequest] = []
    private var cursor = 0
    var onPush: (@Sendable (Int) async -> Void)?

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        let (count, pushes) = lock.withLock { () -> (Int, Int) in
            requests.append(request)
            return (requests.count, requests.filter { $0.url.path == "/api/sync/push" }.count)
        }
        _ = count
        if request.url.path == "/api/sync/push" {
            await onPush?(pushes)
            let body = jsonObject(request.body ?? Data())
            let ids = (body["mutations"] as? [[String: Any]] ?? []).compactMap { $0["id"] as? String }
            let results = ids.map { #"{"id": "\#($0)", "status": "applied", "events": [], "deleted_event_ids": []}"# }
            return HTTPResponse(status: 200, body: Data(#"{"results": [\#(results.joined(separator: ","))]}"#.utf8))
        }
        let c = lock.withLock { cursor += 1; return cursor }
        return HTTPResponse(status: 200, body: Data(ContractSamples.pull("", cursor: c).utf8))
    }

    var paths: [String] { lock.withLock { requests.map(\.url.path) } }
    func count(_ path: String) -> Int { paths.filter { $0 == path }.count }
}

/// Liefert der Reihe nach vorbereitete SSE-Ströme.
final class FakeStreams: @unchecked Sendable {
    enum Script { case cursors([Int], then: Error?), hang }
    private let lock = NSLock()
    private var scripts: [Script]
    private(set) var connects = 0

    init(_ scripts: [Script]) { self.scripts = scripts }

    func next() -> AsyncThrowingStream<Int, Error> {
        let script: Script = lock.withLock {
            connects += 1
            return scripts.isEmpty ? .cursors([], then: APIError.transport("weg")) : scripts.removeFirst()
        }
        return AsyncThrowingStream { c in
            switch script {
            case let .cursors(values, error):
                for v in values { c.yield(v) }
                c.finish(throwing: error)
            case .hang:
                break
            }
        }
    }

    var connectCount: Int { lock.withLock { connects } }
}

final class SleepLog: @unchecked Sendable {
    private let lock = NSLock()
    private var delays: [TimeInterval] = []
    let limit: Int
    init(limit: Int) { self.limit = limit }

    func sleep(_ d: TimeInterval) throws {
        try lock.withLock {
            delays.append(d)
            if delays.count >= limit { throw CancellationError() }
        }
    }

    var recorded: [TimeInterval] { lock.withLock { delays } }
}

func engine(_ t: HTTPTransport) -> SyncEngine {
    SyncEngine(api: SyncAPI(baseURL: EngineFixtures.base, transport: t, tokens: StaticToken("ld_test")), store: InMemoryCacheStore())
}

/// Wartet höchstens eine Sekunde auf eine Bedingung.
func eventually(_ condition: @Sendable () async -> Bool) async -> Bool {
    for _ in 0..<200 {
        if await condition() { return true }
        try? await Task.sleep(nanoseconds: 5_000_000)
    }
    return await condition()
}

@Suite("Live-Abgleich")
struct LiveSyncTests {
    @Test func outboxWatchSeesOnlyNewIds() {
        var watch = OutboxWatch()
        var state = SyncState()
        let empty = watch.hasNewChanges(in: state)
        #expect(!empty)
        _ = state.enqueue(Mutation(.deleteEvent(eventId: "ev_1", baseVersion: 1, scope: .all, occurrence: nil)), now: Date())
        let added = watch.hasNewChanges(in: state)
        #expect(added)
        // Gleiche Outbox, z. B. nach einem gescheiterten Versuch: kein Anlass.
        let again = watch.hasNewChanges(in: state)
        #expect(!again)
        state.outbox.removeAll()
        let removed = watch.hasNewChanges(in: state)
        #expect(!removed)
    }

    @Test func reconnectsWithBackoffAndResetsOnCursor() async {
        let t = RoutingTransport()
        let streams = FakeStreams([
            .cursors([], then: APIError.transport("weg")),
            .cursors([], then: nil),
            .cursors([50], then: APIError.transport("weg")),
            .cursors([], then: APIError.transport("weg")),
        ])
        let log = SleepLog(limit: 4)
        let e = engine(t)
        let stop = await LiveSync.follow(engine: e, connect: { streams.next() }, sleep: { try log.sleep($0) })
        #expect(stop == .cancelled)
        #expect(log.recorded == [1, 2, 1, 2])
        #expect(streams.connectCount == 4)
        // Vor jedem Verbinden ein Abgleich, dazu einer für den Cursor 50.
        #expect(t.count("/api/sync/pull") == 5)
    }

    @Test func unauthorizedStopsFollowing() async {
        let t = RoutingTransport()
        let streams = FakeStreams([.cursors([], then: APIError.unauthorized)])
        let log = SleepLog(limit: 10)
        let stop = await LiveSync.follow(engine: engine(t), connect: { streams.next() }, sleep: { try log.sleep($0) })
        #expect(stop == .unauthorized)
        #expect(log.recorded.isEmpty)
    }

    @Test func unauthorizedFromSyncStops() async {
        let t = ScriptedTransport([ScriptedTransport.error(401, code: "unauthorized")])
        let e = EngineFixtures.engine(t)
        let streams = FakeStreams([])
        let stop = await LiveSync.run(engine: e, connect: { streams.next() }, sleep: { _ in })
        #expect(stop == .unauthorized)
        #expect(streams.connectCount == 0)
    }

    @Test func newChangeIsPushed() async {
        let t = RoutingTransport()
        let e = engine(t)
        let task = Task { await LiveSync.pushChanges(engine: e) }
        await e.delete(eventId: "ev_1")
        #expect(await eventually { t.count("/api/sync/push") == 1 })
        #expect(await eventually { await e.state.outbox.isEmpty })
        task.cancel()
        #expect(await task.value == .cancelled)
    }

    @Test func runEndsWhenCancelled() async {
        let t = RoutingTransport()
        let e = engine(t)
        let streams = FakeStreams([.hang])
        let task = Task { await LiveSync.run(engine: e, connect: { streams.next() }) }
        #expect(await eventually { streams.connectCount == 1 })
        task.cancel()
        #expect(await task.value == .cancelled)
    }

    @Test func syncDuringSyncRunsOnceMore() async throws {
        let t = RoutingTransport()
        let e = engine(t)
        t.onPush = { n in
            guard n == 1 else { return }
            // Während der erste Push unterwegs ist: neue Änderung, neuer Aufruf.
            await e.delete(eventId: "ev_2")
            _ = try? await e.sync()
        }
        await e.delete(eventId: "ev_1")
        try await e.sync()
        #expect(t.count("/api/sync/push") == 2)
        // Die zweite Runde zieht noch einmal (ohne sie: ein Pull).
        #expect(t.count("/api/sync/pull") == 2)
        #expect(await e.state.outbox.isEmpty)
    }
}
