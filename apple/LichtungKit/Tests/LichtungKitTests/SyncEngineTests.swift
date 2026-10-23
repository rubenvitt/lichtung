import Foundation
@testable import LichtungKit
import Testing

/// Gespielter Server: beantwortet Anfragen der Reihe nach aus einer Liste.
final class ScriptedTransport: HTTPTransport, @unchecked Sendable {
    typealias Handler = @Sendable (HTTPRequest) throws -> HTTPResponse
    private let lock = NSLock()
    private var handlers: [Handler]
    private(set) var requests: [HTTPRequest] = []

    init(_ handlers: [Handler]) { self.handlers = handlers }

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        let handler: Handler = try lock.withLock {
            requests.append(request)
            guard !handlers.isEmpty else { throw APIError.transport("keine Antwort mehr vorgesehen") }
            return handlers.removeFirst()
        }
        return try handler(request)
    }

    var recorded: [HTTPRequest] { lock.withLock { requests } }

    static func json(_ text: String, status: Int = 200) -> Handler {
        { _ in HTTPResponse(status: status, body: Data(text.utf8)) }
    }

    static func error(_ status: Int, code: String) -> Handler {
        json(#"{"error": {"code": "\#(code)", "message": "Testfehler"}}"#, status: status)
    }
}

enum EngineFixtures {
    static let base = URL(string: "https://lichtung.example.org")!

    static func eventJSON(_ id: String, version: Int = 1, title: String = "Termin") -> String {
        """
        {"id": "\(id)", "calendar_id": "cal_1", "version": \(version), "title": "\(title)",
         "start": {"date_time": "2026-11-03T09:00:00", "time_zone": "Europe/Berlin"},
         "end": {"date_time": "2026-11-03T10:00:00", "time_zone": "Europe/Berlin"},
         "status": "confirmed", "availability": "busy", "recurrence": null, "overrides": []}
        """
    }

    static func engine(_ transport: ScriptedTransport, store: CacheStore = InMemoryCacheStore(), token: String? = "ld_test") -> SyncEngine {
        SyncEngine(
            api: SyncAPI(baseURL: base, transport: transport, tokens: StaticToken(token)),
            store: store,
            deviceTimeZone: SyncFixtures.tz,
            now: { SyncFixtures.now }
        )
    }
}

@Suite("SyncEngine")
struct SyncEngineTests {
    typealias E = EngineFixtures

    @Test func pullsAllPagesAndSendsToken() async throws {
        let t = ScriptedTransport([
            ScriptedTransport.json(ContractSamples.pull("{\"type\": \"event\", \"op\": \"upsert\", \"data\": \(E.eventJSON("ev_1"))}", cursor: 3, hasMore: true)),
            ScriptedTransport.json(ContractSamples.pull("{\"type\": \"event\", \"op\": \"upsert\", \"data\": \(E.eventJSON("ev_2"))}", cursor: 5)),
        ])
        let store = InMemoryCacheStore()
        let engine = E.engine(t, store: store)
        let outcome = try await engine.sync()
        #expect(outcome == .synced)
        let state = await engine.state
        #expect(state.cursor == 5)
        #expect(Set(state.events.keys) == ["ev_1", "ev_2"])
        #expect(try store.load()?.cursor == 5)
        let urls = t.recorded.map { $0.url.absoluteString }
        #expect(urls == [
            "https://lichtung.example.org/api/sync/pull?since=0&limit=500",
            "https://lichtung.example.org/api/sync/pull?since=3&limit=500",
        ])
        #expect(t.recorded.allSatisfy { $0.headers["Authorization"] == "Bearer ld_test" })
    }

    @Test func pushesOutboxBeforePulling() async throws {
        let t = ScriptedTransport([
            { req in
                let body = jsonObject(req.body ?? Data())
                let mutations = body["mutations"] as? [[String: Any]] ?? []
                let id = mutations.first?["id"] as? String ?? "?"
                return HTTPResponse(status: 200, body: Data("""
                {"results": [{"id": "\(id)", "status": "applied", "events": [\(E.eventJSON("ev_9", title: "Zahnarzt"))], "deleted_event_ids": []}]}
                """.utf8))
            },
            ScriptedTransport.json(ContractSamples.pull("", cursor: 1)),
        ])
        let engine = E.engine(t)
        let localId = await engine.enqueue(.createEvent(calendarId: "cal_1", event: EventInput(
            title: "Zahnarzt", start: SyncFixtures.at(3, 9), end: SyncFixtures.at(3, 10)
        )))
        #expect(localId?.hasPrefix("local:") == true)
        #expect(await engine.visibleEvents()[localId!]?.title == "Zahnarzt")

        try await engine.sync()
        let state = await engine.state
        #expect(state.outbox.isEmpty)
        #expect(state.events["ev_9"]?.title == "Zahnarzt")
        #expect(state.currentId(for: localId!) == "ev_9")
        #expect(t.recorded.first?.method == "POST")
        #expect(t.recorded.first?.url.path == "/api/sync/push")
    }

    @Test func offlineKeepsOutboxAndDefers() async throws {
        let t = ScriptedTransport([{ _ in throw URLError(.notConnectedToInternet) }])
        let engine = E.engine(t)
        await engine.delete(eventId: "ev_1")
        let outcome = try await engine.sync()
        guard case .deferred(.transport) = outcome else { Issue.record("erwartet: später, war \(outcome)"); return }
        #expect(await engine.state.outbox.count == 1)
    }

    @Test func unauthorizedIsThrown() async {
        let t = ScriptedTransport([ScriptedTransport.error(401, code: "unauthorized")])
        let engine = E.engine(t)
        await #expect(throws: APIError.unauthorized) { try await engine.sync() }
    }

    @Test func missingTokenIsUnauthorized() async {
        let engine = E.engine(ScriptedTransport([]), token: nil)
        await #expect(throws: APIError.unauthorized) { try await engine.sync() }
    }

    @Test func expiredCursorStartsOverButKeepsOutbox() async throws {
        var seeded = SyncState()
        seeded.cursor = 99
        seeded.events["ev_alt"] = SyncFixtures.event("ev_alt")
        let store = InMemoryCacheStore(seeded)
        let t = ScriptedTransport([
            ScriptedTransport.error(410, code: "cursor_expired"),
            ScriptedTransport.json(ContractSamples.pull("{\"type\": \"event\", \"op\": \"upsert\", \"data\": \(E.eventJSON("ev_neu"))}", cursor: 120)),
        ])
        let engine = E.engine(t, store: store)
        try await engine.sync()
        let state = await engine.state
        #expect(Set(state.events.keys) == ["ev_neu"])
        #expect(state.cursor == 120)
        #expect(t.recorded.last?.url.query?.contains("since=0") == true)
    }

    @Test func retryStopsPushingAndStillPulls() async throws {
        let t = ScriptedTransport([
            { req in
                let ids = (jsonObject(req.body ?? Data())["mutations"] as? [[String: Any]] ?? []).compactMap { $0["id"] as? String }
                let results = ids.map { #"{"id": "\#($0)", "status": "retry", "error": {"code": "provider_unavailable", "message": "weg"}, "retry_after_seconds": 30}"# }
                return HTTPResponse(status: 200, body: Data(#"{"results": [\#(results.joined(separator: ","))]}"#.utf8))
            },
            ScriptedTransport.json(ContractSamples.pull("", cursor: 2)),
        ])
        let engine = E.engine(t)
        await engine.delete(eventId: "ev_1")
        await engine.delete(eventId: "ev_2")
        try await engine.sync()
        let state = await engine.state
        #expect(state.outbox.count == 2)
        #expect(state.outbox.allSatisfy { $0.notBefore != nil })
        #expect(t.recorded.count == 2)
    }

    @Test func observersSeeChanges() async {
        let engine = E.engine(ScriptedTransport([]))
        let seen = Counter()
        await engine.observe { _ in seen.increment() }
        await engine.delete(eventId: "ev_1")
        #expect(seen.value == 2)
    }

    @Test func cursorAnnouncementOnlySyncsWhenNewer() async throws {
        var seeded = SyncState()
        seeded.cursor = 10
        let t = ScriptedTransport([ScriptedTransport.json(ContractSamples.pull("", cursor: 12))])
        let engine = E.engine(t, store: InMemoryCacheStore(seeded))
        try await engine.cursorAnnounced(10)
        #expect(t.recorded.isEmpty)
        try await engine.cursorAnnounced(12)
        #expect(t.recorded.count == 1)
    }

    @Test func fileStoreRoundTrip() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("lichtung-test-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileCacheStore(url: dir.appendingPathComponent("cache.json"))
        #expect(try store.load() == nil)
        var s = SyncState()
        s.cursor = 42
        s.events["ev_1"] = SyncFixtures.event("ev_1", rrule: "FREQ=DAILY")
        s.enqueue(Mutation(id: "m", .updateEvent(eventId: "ev_1", baseVersion: 1, scope: .this, occurrence: SyncFixtures.at(4, 9),
                                                 changes: EventChanges(location: .clear))), now: SyncFixtures.now)
        try store.save(s)
        #expect(try store.load() == s)
        try store.clear()
        #expect(try store.load() == nil)
    }
}

final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    func increment() { lock.withLock { n += 1 } }
    var value: Int { lock.withLock { n } }
}

@Suite("SSE und Anmeldung")
struct StreamAndLoginTests {
    @Test func parsesEventStream() {
        var p = ServerSentEventParser()
        let lines = ["event: cursor", "data: 1834", "", ": keep-alive", "", "event: cursor", "data: 1837", "", "data: a", "data: b", "id: 7", ""]
        let events = lines.compactMap { p.feed(line: $0) }
        #expect(events == [
            ServerSentEvent(event: "cursor", data: "1834"),
            ServerSentEvent(event: "cursor", data: "1837"),
            ServerSentEvent(event: "message", data: "a\nb", id: "7"),
        ])
    }

    @Test func toleratesCRLF() {
        var p = ServerSentEventParser()
        _ = p.feed(line: "event: cursor\r")
        _ = p.feed(line: "data:5\r")
        #expect(p.feed(line: "\r") == ServerSentEvent(event: "cursor", data: "5"))
    }

    @Test func reconnectBackoff() {
        var b = ReconnectBackoff()
        let delays = (0..<8).map { _ in b.nextDelay() }
        #expect(delays == [1, 2, 4, 8, 16, 32, 60, 60])
        b.reset()
        #expect(b.nextDelay() == 1)
    }

    @Test func sha256KnownValues() {
        func hex(_ s: String) -> String { SHA256.hash(Array(s.utf8)).map { String(format: "%02x", $0) }.joined() }
        #expect(hex("") == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        #expect(hex("abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        #expect(hex(String(repeating: "a", count: 1000)) == "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3")
    }

    @Test func pkceChallengeMatchesRFC7636() {
        let login = DeviceLogin(serverURL: EngineFixtures.base, clientId: "c1", codeVerifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", state: "s1")
        #expect(login.codeChallenge == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
    }

    @Test func authorizeAndCallback() throws {
        let login = DeviceLogin(serverURL: EngineFixtures.base, clientId: "client-1", codeVerifier: DeviceLogin.randomToken(), state: "zustand")
        let url = login.authorizeURL(endpoint: URL(string: "https://lichtung.example.org/oauth/authorize")!)
        let items = URLComponents(url: url, resolvingAgainstBaseURL: false)!.queryItems!
        let q = Dictionary(uniqueKeysWithValues: items.map { ($0.name, $0.value ?? "") })
        #expect(q["redirect_uri"] == "dev.rubeen.lichtung:/oauth")
        #expect(q["code_challenge_method"] == "S256")
        #expect(q["resource"] == "https://lichtung.example.org/api")

        #expect(try login.code(fromCallback: URL(string: "dev.rubeen.lichtung:/oauth?code=abc&state=zustand")!) == "abc")
        #expect(throws: DeviceLogin.CallbackError.stateMismatch) {
            try login.code(fromCallback: URL(string: "dev.rubeen.lichtung:/oauth?code=abc&state=fremd")!)
        }
        #expect(throws: DeviceLogin.CallbackError.denied("abgelehnt")) {
            try login.code(fromCallback: URL(string: "dev.rubeen.lichtung:/oauth?error=access_denied&error_description=abgelehnt")!)
        }
        let body = String(decoding: login.tokenRequestBody(code: "a/b+c"), as: UTF8.self)
        #expect(body.contains("code=a%2Fb%2Bc"))
        #expect(body.contains("redirect_uri=dev.rubeen.lichtung%3A%2Foauth"))
        #expect(DeviceLogin.randomToken().count == 64)
    }

    @Test func registrationBodyIsSnakeCase() throws {
        let o = jsonObject(try DeviceLogin.registrationBody(deviceName: "iPhone"))
        #expect(o["client_name"] as? String == "iPhone")
        #expect(o["redirect_uris"] as? [String] == ["dev.rubeen.lichtung:/oauth"])
        #expect(o["token_endpoint_auth_method"] as? String == "none")
    }
}
