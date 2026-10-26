import Foundation
@testable import LichtungKit
import Testing

@Suite("Server-Adresse")
struct ServerAddressTests {
    @Test(arguments: [
        ("lichtung.example.org", "https://lichtung.example.org"),
        ("  https://Lichtung.Example.org/  ", "https://lichtung.example.org"),
        ("https://lichtung.example.org/api", "https://lichtung.example.org"),
        ("https://lichtung.example.org/api/", "https://lichtung.example.org"),
        ("https://lichtung.example.org/.well-known/oauth-authorization-server", "https://lichtung.example.org"),
        ("https://lichtung.example.org:443", "https://lichtung.example.org"),
        ("https://lichtung.example.org:8443/", "https://lichtung.example.org:8443"),
        ("https://example.org/lichtung/", "https://example.org/lichtung"),
        ("https://lichtung.example.org/?x=1#a", "https://lichtung.example.org"),
        ("http://localhost:8080", "http://localhost:8080"),
        ("http://127.0.0.1:8080/", "http://127.0.0.1:8080"),
    ])
    func normalizes(_ input: String, _ expected: String) throws {
        #expect(try ServerAddress.normalize(input).absoluteString == expected)
    }

    @Test func apiPathFollowsBase() throws {
        let url = try ServerAddress.normalize("lichtung.example.org/")
        #expect(url.appendingPathComponent("api").absoluteString == "https://lichtung.example.org/api")
    }

    @Test func rejects() {
        #expect(ServerAddress.problem(with: "   ") == .empty)
        #expect(ServerAddress.problem(with: "lichtung example") == .invalid)
        #expect(ServerAddress.problem(with: "http://lichtung.example.org") == .insecure)
        #expect(ServerAddress.problem(with: "ftp://lichtung.example.org") == .unsupportedScheme("ftp"))
        #expect(ServerAddress.problem(with: "https://ruben:geheim@lichtung.example.org") == .containsCredentials)
        #expect(ServerAddress.problem(with: "https://") == .invalid)
        #expect(ServerAddress.problem(with: "lichtung.example.org") == nil)
    }

    @Test func messagesAreGerman() {
        #expect(ServerAddress.Problem.insecure.message.contains("https://"))
        #expect(ServerAddress.Problem.empty.errorDescription == "Bitte die Adresse des Lichtung-Servers eingeben.")
    }

    @Test func origins() {
        let a = URL(string: "https://lichtung.example.org")!
        #expect(ServerAddress.sameOrigin(a, URL(string: "https://LICHTUNG.example.org:443/oauth/token")!))
        #expect(!ServerAddress.sameOrigin(a, URL(string: "https://evil.example.org/oauth/token")!))
        #expect(!ServerAddress.sameOrigin(a, URL(string: "http://lichtung.example.org")!))
        #expect(ServerAddress.displayName(URL(string: "https://example.org:8443/lichtung")!) == "example.org:8443/lichtung")
    }

    @Test func cacheOnlyForSameServer() {
        let a = URL(string: "https://lichtung.example.org")!
        #expect(ServerAddress.cacheBelongs(to: a, newServer: a))
        #expect(!ServerAddress.cacheBelongs(to: a, newServer: URL(string: "https://andere.example.org")!))
        #expect(!ServerAddress.cacheBelongs(to: nil, newServer: a))
    }
}

@Suite("Geräte-Anmeldung (Ablauf)")
struct DeviceLoginFlowTests {
    static let metadata = """
    {"issuer": "https://lichtung.example.org",
     "authorization_endpoint": "https://lichtung.example.org/oauth/authorize",
     "token_endpoint": "https://lichtung.example.org/oauth/token",
     "registration_endpoint": "https://lichtung.example.org/oauth/register",
     "revocation_endpoint": "https://lichtung.example.org/oauth/revoke",
     "response_types_supported": ["code"],
     "code_challenge_methods_supported": ["S256"]}
    """

    @Test func fullFlow() async throws {
        let t = ScriptedTransport([
            ScriptedTransport.json(Self.metadata),
            ScriptedTransport.json(#"{"client_id": "cl_42", "client_name": "Rubens iPhone"}"#, status: 201),
            ScriptedTransport.json(#"{"access_token": "ld_erfunden", "token_type": "Bearer"}"#),
        ])
        let flow = DeviceLoginFlow(transport: t)
        let (server, meta) = try await flow.discover(serverInput: "lichtung.example.org")
        #expect(server.absoluteString == "https://lichtung.example.org")
        let prepared = try await flow.register(serverURL: server, metadata: meta, deviceName: "  Rubens iPhone ")
        #expect(prepared.login.clientId == "cl_42")

        let authorize = URLComponents(url: prepared.authorizeURL, resolvingAgainstBaseURL: false)!
        #expect(authorize.path == "/oauth/authorize")
        let items = Dictionary(uniqueKeysWithValues: (authorize.queryItems ?? []).map { ($0.name, $0.value ?? "") })
        #expect(items["resource"] == "https://lichtung.example.org/api")
        #expect(items["client_id"] == "cl_42")

        let callback = URL(string: "dev.rubeen.lichtung:/oauth?code=c_1&state=\(prepared.login.state)&iss=https%3A%2F%2Flichtung.example.org")!
        let creds = try await flow.finish(prepared, callback: callback)
        #expect(creds == DeviceCredentials(serverURL: server, token: "ld_erfunden", clientId: "cl_42"))

        let reqs = t.recorded
        #expect(reqs.map { "\($0.method) \($0.url.path)" } == [
            "GET /.well-known/oauth-authorization-server", "POST /oauth/register", "POST /oauth/token",
        ])
        #expect(reqs.allSatisfy { $0.headers["Authorization"] == nil })
        let reg = jsonObject(reqs[1].body ?? Data())
        #expect(reg["client_name"] as? String == "Rubens iPhone")
        #expect(reqs[2].headers["Content-Type"] == "application/x-www-form-urlencoded")
        let form = String(decoding: reqs[2].body ?? Data(), as: UTF8.self)
        #expect(form.contains("code=c_1"))
        #expect(form.contains("client_id=cl_42"))
    }

    @Test func issuerOnOtherHostIsRejected() async {
        let other = Self.metadata.replacingOccurrences(of: "\"issuer\": \"https://lichtung.example.org\"", with: "\"issuer\": \"https://evil.example.org\"")
        let t = ScriptedTransport([ScriptedTransport.json(other)])
        await #expect(throws: LoginFailure.issuerMismatch("https://evil.example.org")) {
            _ = try await DeviceLoginFlow(transport: t).discover(serverInput: "lichtung.example.org")
        }
    }

    @Test func issuerPathWins() throws {
        // Eingabe mit „/api“ oder anderem Pfad: Der Server weiß es besser.
        let typed = URL(string: "https://example.org/falsch")!
        let resolved = try DeviceLoginFlow.resolveServer(typed: typed, issuer: "https://example.org/lichtung/")
        #expect(resolved.absoluteString == "https://example.org/lichtung")
    }

    @Test func endpointOnOtherHostIsRejected() async {
        let other = Self.metadata.replacingOccurrences(of: "https://lichtung.example.org/oauth/token", with: "https://evil.example.org/oauth/token")
        let t = ScriptedTransport([ScriptedTransport.json(other)])
        await #expect(throws: LoginFailure.issuerMismatch("https://evil.example.org/oauth/token")) {
            _ = try await DeviceLoginFlow(transport: t).discover(serverInput: "lichtung.example.org")
        }
    }

    @Test func notLichtung() async {
        let t = ScriptedTransport([ScriptedTransport.json("<html>Hallo</html>"), { _ in HTTPResponse(status: 404) }])
        let flow = DeviceLoginFlow(transport: t)
        await #expect(throws: LoginFailure.notLichtung) { _ = try await flow.discover(serverInput: "example.org") }
        await #expect(throws: LoginFailure.notLichtung) { _ = try await flow.discover(serverInput: "example.org") }
    }

    @Test func invalidAddressNeedsNoRequest() async {
        let t = ScriptedTransport([])
        await #expect(throws: LoginFailure.invalidAddress(.insecure)) {
            _ = try await DeviceLoginFlow(transport: t).discover(serverInput: "http://lichtung.example.org")
        }
        #expect(t.recorded.isEmpty)
    }

    @Test func offline() async {
        let t = ScriptedTransport([{ _ in throw URLError(.notConnectedToInternet) }])
        do {
            _ = try await DeviceLoginFlow(transport: t).discover(serverInput: "lichtung.example.org")
            Issue.record("Fehler erwartet")
        } catch let f as LoginFailure {
            guard case .unreachable = f else { Issue.record("statt \(f)"); return }
            #expect(f.message.contains("nicht erreichbar"))
        } catch {
            Issue.record("falscher Fehler \(error)")
        }
    }

    func prepared() -> PreparedLogin {
        let meta = AuthorizationServerMetadata(
            issuer: "https://lichtung.example.org",
            authorizationEndpoint: URL(string: "https://lichtung.example.org/oauth/authorize")!,
            tokenEndpoint: URL(string: "https://lichtung.example.org/oauth/token")!
        )
        let server = URL(string: "https://lichtung.example.org")!
        return PreparedLogin(serverURL: server, metadata: meta, login: DeviceLogin(serverURL: server, clientId: "cl_1", state: "s1"))
    }

    @Test func callbackErrorsAreMapped() async {
        let flow = DeviceLoginFlow(transport: ScriptedTransport([]))
        await #expect(throws: LoginFailure.stateMismatch) {
            _ = try await flow.finish(prepared(), callback: URL(string: "dev.rubeen.lichtung:/oauth?code=c&state=anders")!)
        }
        await #expect(throws: LoginFailure.denied("Nicht bestätigt")) {
            _ = try await flow.finish(prepared(), callback: URL(string: "dev.rubeen.lichtung:/oauth?error=access_denied&error_description=Nicht%20best%C3%A4tigt&state=s1")!)
        }
    }

    @Test func tokenErrorsCarryDescription() async {
        let t = ScriptedTransport([ScriptedTransport.json(#"{"error": "invalid_grant", "error_description": "Code abgelaufen"}"#, status: 400)])
        await #expect(throws: LoginFailure.tokenFailed("Code abgelaufen")) {
            _ = try await DeviceLoginFlow(transport: t).finish(prepared(), callback: URL(string: "dev.rubeen.lichtung:/oauth?code=c&state=s1")!)
        }
    }

    @Test func nonDeviceTokenIsRejected() async {
        let t = ScriptedTransport([ScriptedTransport.json(#"{"access_token": "lo_mcp", "token_type": "Bearer"}"#)])
        await #expect(throws: LoginFailure.notDeviceToken) {
            _ = try await DeviceLoginFlow(transport: t).finish(prepared(), callback: URL(string: "dev.rubeen.lichtung:/oauth?code=c&state=s1")!)
        }
    }

    @Test func registrationError() async {
        let t = ScriptedTransport([
            ScriptedTransport.json(#"{"error": "invalid_redirect_uri", "error_description": "Schema nicht erlaubt"}"#, status: 400),
        ])
        await #expect(throws: LoginFailure.registrationFailed("Schema nicht erlaubt")) {
            _ = try await DeviceLoginFlow(transport: t).register(serverURL: prepared().serverURL, metadata: prepared().metadata, deviceName: "Mac")
        }
        // Ohne `registration_endpoint` gilt der Pfad aus dem Vertrag.
        #expect(t.recorded.first?.url.absoluteString == "https://lichtung.example.org/oauth/register")
    }

    @Test func revokeIsBestEffort() async {
        let t = ScriptedTransport([{ _ in HTTPResponse(status: 200) }, { _ in throw URLError(.timedOut) }])
        let flow = DeviceLoginFlow(transport: t)
        let server = URL(string: "https://lichtung.example.org")!
        #expect(await flow.revoke(token: "ld_a+b", serverURL: server))
        #expect(await flow.revoke(token: "ld_a", serverURL: server) == false)
        #expect(t.recorded[0].url.path == "/oauth/revoke")
        #expect(String(decoding: t.recorded[0].body ?? Data(), as: UTF8.self) == "token=ld_a%2Bb")
    }

    @Test func phases() {
        #expect(LoginPhase.checkingServer.isBusy)
        #expect(LoginPhase.waitingForBrowser.statusText == "Anmeldung im Browserfenster …")
        #expect(!LoginPhase.failed(.cancelled).isBusy)
        #expect(LoginPhase.failed(.cancelled).statusText == nil)
        #expect(LoginPhase.failed(.cancelled).failure == nil)
        #expect(LoginPhase.failed(.notLichtung).failure == .notLichtung)
        #expect(LoginFailure.from(CancellationError()) == .cancelled)
        #expect(LoginFailure.from(ServerAddress.Problem.empty) == .invalidAddress(.empty))
    }
}

@Suite("Start der App")
struct SessionStartTests {
    @Test func decisions() {
        #expect(SessionStart.decide(mode: "demo", serverURL: "https://lichtung.example.org", hasToken: true, hasCache: true) == .demo)
        #expect(SessionStart.decide(mode: nil, serverURL: nil, hasToken: false, hasCache: false) == .login(prefill: ""))
        let url = URL(string: "https://lichtung.example.org")!
        #expect(SessionStart.decide(mode: "server", serverURL: url.absoluteString, hasToken: true, hasCache: false) == .server(url))
        #expect(SessionStart.decide(mode: "server", serverURL: url.absoluteString, hasToken: false, hasCache: true) == .reauthenticate(url))
        #expect(SessionStart.decide(mode: "server", serverURL: url.absoluteString, hasToken: false, hasCache: false) == .login(prefill: "lichtung.example.org"))
        #expect(SessionStart.decide(mode: "server", serverURL: "kaputt adresse", hasToken: true, hasCache: true) == .login(prefill: "kaputt adresse"))
    }
}

@Suite("Geräte-Token und 401")
struct StoredDeviceTokenTests {
    final class Counter: @unchecked Sendable {
        private let lock = NSLock()
        private var n = 0
        func hit() { lock.withLock { n += 1 } }
        var value: Int { lock.withLock { n } }
    }

    @Test func readsOnceAndSaves() async throws {
        let store = InMemorySecretStore([StoredDeviceToken.key: "ld_alt"])
        let tokens = StoredDeviceToken(store: store)
        #expect(await tokens.deviceToken() == "ld_alt")
        try tokens.save("ld_neu")
        #expect(try store.read(StoredDeviceToken.key) == "ld_neu")
        #expect(await tokens.deviceToken() == "ld_neu")
        try tokens.clear()
        #expect(await tokens.deviceToken() == nil)
        #expect(try store.read(StoredDeviceToken.key) == nil)
    }

    @Test func lateRejectionOfOldTokenIsIgnored() async throws {
        let counter = Counter()
        let tokens = StoredDeviceToken(store: InMemorySecretStore(), onRejected: { counter.hit() })
        try tokens.save("ld_neu")
        await tokens.tokenRejected("ld_alt")
        #expect(tokens.hasToken)
        #expect(counter.value == 0)
        await tokens.tokenRejected("ld_neu")
        #expect(!tokens.hasToken)
        #expect(counter.value == 1)
    }

    @Test func unauthorizedSignalsAndKeepsCacheAndOutbox() async throws {
        let counter = Counter()
        let secrets = InMemorySecretStore([StoredDeviceToken.key: "ld_widerrufen"])
        let tokens = StoredDeviceToken(store: secrets, onRejected: { counter.hit() })
        let cache = InMemoryCacheStore()
        let t = ScriptedTransport([ScriptedTransport.error(401, code: "unauthorized")])
        let engine = SyncEngine(api: SyncAPI(baseURL: EngineFixtures.base, transport: t, tokens: tokens), store: cache)
        await engine.delete(eventId: "ev_1")

        await #expect(throws: APIError.unauthorized) { try await engine.sync() }

        #expect(counter.value == 1)
        #expect(try secrets.read(StoredDeviceToken.key) == nil)
        #expect(await engine.state.outbox.count == 1)
        #expect(try cache.load()?.outbox.count == 1)
        // Ohne Token kein weiterer Versuch beim Server.
        await #expect(throws: APIError.unauthorized) { try await engine.sync() }
        #expect(t.recorded.count == 1)
    }

    @Test func managementCallsSignalToo() async {
        let counter = Counter()
        let tokens = StoredDeviceToken(store: InMemorySecretStore([StoredDeviceToken.key: "ld_x"]), onRejected: { counter.hit() })
        let t = ScriptedTransport([ScriptedTransport.error(401, code: "unauthorized")])
        let m = ManagementAPI(sync: SyncAPI(baseURL: EngineFixtures.base, transport: t, tokens: tokens))
        await #expect(throws: APIError.unauthorized) { _ = try await m.devices() }
        #expect(counter.value == 1)
    }

    @Test func reportUnauthorizedReadsBearer() async throws {
        let counter = Counter()
        let tokens = StoredDeviceToken(store: InMemorySecretStore([StoredDeviceToken.key: "ld_sse"]), onRejected: { counter.hit() })
        let api = SyncAPI(baseURL: EngineFixtures.base, transport: ScriptedTransport([]), tokens: tokens)
        let req = await api.eventStreamRequest()
        #expect(req.headers["Authorization"] == "Bearer ld_sse")
        await api.reportUnauthorized(req)
        #expect(counter.value == 1)
    }
}
