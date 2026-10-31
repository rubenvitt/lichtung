import Foundation
@testable import LichtungKit
import Testing

@Suite("Gemerkter OAuth-Client (LIC-49)")
struct RememberedClientTests {
    static let server = URL(string: "https://lichtung.example.org")!

    static let metadata = AuthorizationServerMetadata(
        issuer: "https://lichtung.example.org",
        authorizationEndpoint: URL(string: "https://lichtung.example.org/oauth/authorize")!,
        tokenEndpoint: URL(string: "https://lichtung.example.org/oauth/token")!,
        registrationEndpoint: URL(string: "https://lichtung.example.org/oauth/register")!
    )

    static let invalidGrant = ScriptedTransport.json(#"{"error": "invalid_grant", "error_description": "Unbekannter Autorisierungscode"}"#, status: 400)
    static let invalidClient = ScriptedTransport.json(#"{"error": "invalid_client", "error_description": "Unbekannter Client"}"#, status: 401)

    static func token(_ value: String) -> ScriptedTransport.Handler {
        ScriptedTransport.json(#"{"access_token": "\#(value)", "token_type": "Bearer"}"#)
    }

    static func callback(_ prepared: PreparedLogin) -> URL {
        URL(string: "dev.rubeen.lichtung:/oauth?code=c_1&state=\(prepared.login.state)")!
    }

    /// Discovery, Schritt 2 und Tausch, wie die App sie aufruft.
    static func login(_ flow: DeviceLoginFlow, name: String = "Rubens iPhone") async throws -> DeviceCredentials {
        let (server, meta) = try await flow.discover(serverInput: "lichtung.example.org")
        let prepared = try await flow.prepare(serverURL: server, metadata: meta, deviceName: name)
        return try await flow.finish(prepared, callback: callback(prepared))
    }

    static let metadataJSON = ScriptedTransport.json(DeviceLoginFlowTests.metadata)

    @Test func twoLoginsRegisterOnce() async throws {
        let t = ScriptedTransport([
            Self.metadataJSON,
            ScriptedTransport.json(#"{"client_id": "cl_42", "client_name": "Rubens iPhone"}"#, status: 201),
            Self.token("ld_eins"),
            // Zweite Anmeldung: Prüfung des gemerkten Clients, dann der Tausch.
            Self.metadataJSON,
            Self.invalidGrant,
            Self.token("ld_zwei"),
        ])
        let store = InMemoryClientStore()
        let flow = DeviceLoginFlow(transport: t, clients: store)

        let first = try await Self.login(flow)
        #expect(store.client(for: Self.server) == RememberedClient(clientId: "cl_42", name: "Rubens iPhone"))
        let second = try await Self.login(flow)
        #expect(first.clientId == "cl_42")
        #expect(second.clientId == "cl_42")
        #expect(second.token == "ld_zwei")

        let calls = t.recorded.map { "\($0.method) \($0.url.path)" }
        #expect(calls.filter { $0 == "POST /oauth/register" }.count == 1)
        #expect(calls == [
            "GET /.well-known/oauth-authorization-server", "POST /oauth/register", "POST /oauth/token",
            "GET /.well-known/oauth-authorization-server", "POST /oauth/token", "POST /oauth/token",
        ])
        // Die Prüfung schickt die gemerkte client_id mit einem erfundenen Code.
        let probe = String(decoding: t.recorded[4].body ?? Data(), as: UTF8.self)
        #expect(probe.contains("client_id=cl_42"))
        #expect(probe.contains("code=\(DeviceLoginFlow.probeCode)"))
        #expect(probe.contains("grant_type=authorization_code"))
    }

    @Test func unknownClientIsRegisteredAgain() async throws {
        let t = ScriptedTransport([
            Self.invalidClient,
            ScriptedTransport.json(#"{"client_id": "cl_neu"}"#, status: 201),
        ])
        let store = InMemoryClientStore()
        store.remember(RememberedClient(clientId: "cl_alt", name: "Mac"), for: Self.server)
        let flow = DeviceLoginFlow(transport: t, clients: store)

        let prepared = try await flow.prepare(serverURL: Self.server, metadata: Self.metadata, deviceName: "Mac")
        #expect(prepared.login.clientId == "cl_neu")
        #expect(prepared.pendingName == nil)
        #expect(store.client(for: Self.server)?.clientId == "cl_neu")
        #expect(t.recorded.map(\.url.path) == ["/oauth/token", "/oauth/register"])
    }

    @Test func invalidClientAtTokenForgetsClient() async throws {
        let t = ScriptedTransport([Self.invalidGrant, Self.invalidClient])
        let store = InMemoryClientStore()
        store.remember(RememberedClient(clientId: "cl_1", name: "Mac"), for: Self.server)
        let flow = DeviceLoginFlow(transport: t, clients: store)

        let prepared = try await flow.prepare(serverURL: Self.server, metadata: Self.metadata, deviceName: "Mac")
        #expect(prepared.login.clientId == "cl_1")
        await #expect(throws: LoginFailure.tokenFailed("Unbekannter Client")) {
            _ = try await flow.finish(prepared, callback: Self.callback(prepared))
        }
        // Der nächste Versuch registriert neu.
        #expect(store.client(for: Self.server) == nil)
    }

    @Test func otherNameRenamesTheNewDevice() async throws {
        let t = ScriptedTransport([
            Self.invalidGrant,
            Self.token("ld_neu"),
            ScriptedTransport.json(#"{"user_id": "u1", "auth": "app:iPhone", "device": {"id": "d7", "name": "iPhone"}}"#),
            ScriptedTransport.json(#"{"id": "d7", "name": "Rubens iPhone", "current": true}"#),
        ])
        let store = InMemoryClientStore()
        store.remember(RememberedClient(clientId: "cl_1", name: "iPhone"), for: Self.server)
        let flow = DeviceLoginFlow(transport: t, clients: store)

        let prepared = try await flow.prepare(serverURL: Self.server, metadata: Self.metadata, deviceName: " Rubens iPhone ")
        #expect(prepared.pendingName == "Rubens iPhone")
        let creds = try await flow.finish(prepared, callback: Self.callback(prepared))
        #expect(creds.token == "ld_neu")
        #expect(store.client(for: Self.server) == RememberedClient(clientId: "cl_1", name: "Rubens iPhone"))

        let reqs = t.recorded
        #expect(reqs.map { "\($0.method) \($0.url.path)" } == [
            "POST /oauth/token", "POST /oauth/token", "GET /api/me", "PATCH /api/devices/d7",
        ])
        #expect(reqs[2].headers["Authorization"] == "Bearer ld_neu")
        #expect(reqs[3].headers["Authorization"] == "Bearer ld_neu")
        #expect(jsonObject(reqs[3].body ?? Data())["name"] as? String == "Rubens iPhone")
    }

    @Test func failedRenameKeepsLogin() async throws {
        let t = ScriptedTransport([
            Self.invalidGrant,
            Self.token("ld_neu"),
            { _ in throw URLError(.timedOut) },
        ])
        let store = InMemoryClientStore()
        store.remember(RememberedClient(clientId: "cl_1", name: "iPhone"), for: Self.server)
        let flow = DeviceLoginFlow(transport: t, clients: store)
        let prepared = try await flow.prepare(serverURL: Self.server, metadata: Self.metadata, deviceName: "Neu")
        let creds = try await flow.finish(prepared, callback: Self.callback(prepared))
        #expect(creds.token == "ld_neu")
        #expect(store.client(for: Self.server)?.name == "iPhone")
    }

    @Test func probeNeedsConnection() async {
        let t = ScriptedTransport([{ _ in throw URLError(.notConnectedToInternet) }])
        let store = InMemoryClientStore()
        store.remember(RememberedClient(clientId: "cl_1", name: "Mac"), for: Self.server)
        let flow = DeviceLoginFlow(transport: t, clients: store)
        do {
            _ = try await flow.prepare(serverURL: Self.server, metadata: Self.metadata, deviceName: "Mac")
            Issue.record("Fehler erwartet")
        } catch let f as LoginFailure {
            guard case .unreachable = f else { Issue.record("statt \(f)"); return }
        } catch {
            Issue.record("falscher Fehler \(error)")
        }
        // Ohne Antwort bleibt der Client gemerkt.
        #expect(store.client(for: Self.server)?.clientId == "cl_1")
    }

    @Test func withoutStoreEveryLoginRegisters() async throws {
        let reg = ScriptedTransport.json(#"{"client_id": "cl_x"}"#, status: 201)
        let t = ScriptedTransport([reg, reg])
        let flow = DeviceLoginFlow(transport: t)
        _ = try await flow.prepare(serverURL: Self.server, metadata: Self.metadata, deviceName: "Mac")
        _ = try await flow.prepare(serverURL: Self.server, metadata: Self.metadata, deviceName: "Mac")
        #expect(t.recorded.map(\.url.path) == ["/oauth/register", "/oauth/register"])
    }

    @Test func namesForTheLoginForm() {
        let store = InMemoryClientStore()
        store.remember(RememberedClient(clientId: "cl_1", name: "Rubens Mac"), for: URL(string: "https://Lichtung.example.org/")!)
        let flow = DeviceLoginFlow(transport: ScriptedTransport([]), clients: store)
        #expect(flow.rememberedDeviceName(serverInput: "lichtung.example.org") == "Rubens Mac")
        #expect(flow.rememberedDeviceName(serverInput: "andere.example.org") == nil)
        #expect(flow.rememberedDeviceName(serverInput: "") == nil)
        flow.rememberDeviceName("Büro-Mac", for: Self.server)
        #expect(store.client(for: Self.server)?.name == "Büro-Mac")
        // Ohne gemerkten Client entsteht durch Umbenennen keiner.
        flow.rememberDeviceName("x", for: URL(string: "https://andere.example.org")!)
        #expect(store.client(for: URL(string: "https://andere.example.org")!) == nil)
    }

    static func me(_ json: String) throws -> ServerInfo {
        try APICoding.decoder.decode(ServerInfo.self, from: Data(json.utf8))
    }

    /// LIC-55: Ein anderes Gerät hat dieses umbenannt.
    @Test func deviceNameFollowsTheServer() throws {
        let renamed = try Self.me(#"{"auth": "app:Büro-Mac", "device": {"id": "d1", "name": "Büro-Mac"}}"#)
        #expect(DeviceLoginFlow.deviceNameUpdate(remembered: "Rubens Mac", me: renamed) == "Büro-Mac")
        #expect(DeviceLoginFlow.deviceNameUpdate(remembered: "Büro-Mac", me: renamed) == nil)
        // Ohne Gerät (Browser, API-Token, älterer Server) oder ohne Namen bleibt der gemerkte.
        #expect(DeviceLoginFlow.deviceNameUpdate(remembered: "Mac", me: try Self.me(#"{"auth": "web", "device": null}"#)) == nil)
        #expect(DeviceLoginFlow.deviceNameUpdate(remembered: "Mac", me: try Self.me(#"{"device": {"id": "d1"}}"#)) == nil)
        #expect(DeviceLoginFlow.deviceNameUpdate(remembered: "Mac", me: try Self.me(#"{"device": {"id": "d1", "name": "  "}}"#)) == nil)
        #expect(DeviceLoginFlow.deviceNameUpdate(remembered: "Mac", me: try Self.me(#"{"device": {"id": "d1", "name": " Mac Mini "}}"#)) == "Mac Mini")

        let store = InMemoryClientStore()
        store.remember(RememberedClient(clientId: "cl_1", name: "Rubens Mac"), for: Self.server)
        let flow = DeviceLoginFlow(transport: ScriptedTransport([]), clients: store)
        #expect(flow.adoptDeviceName(from: renamed, for: Self.server) == "Büro-Mac")
        #expect(store.client(for: Self.server) == RememberedClient(clientId: "cl_1", name: "Büro-Mac"))
        #expect(flow.rememberedDeviceName(serverInput: "lichtung.example.org") == "Büro-Mac")
        #expect(flow.adoptDeviceName(from: renamed, for: Self.server) == nil)
        // Ohne gemerkten Client entsteht keiner.
        let other = URL(string: "https://andere.example.org")!
        #expect(flow.adoptDeviceName(from: renamed, for: other) == nil)
        #expect(store.client(for: other) == nil)
    }

    /// LIC-56: Installationen ohne gemerkten Client (vor LIC-49) bekommen den
    /// Namen für die Maske, ohne dass ein Client entsteht.
    @Test func deviceNameForMaskWithAndWithoutClient() throws {
        let renamed = try Self.me(#"{"auth": "app:Büro-Mac", "device": {"id": "d1", "name": "Büro-Mac"}}"#)
        let store = InMemoryClientStore()
        let flow = DeviceLoginFlow(transport: ScriptedTransport([]), clients: store)

        // Ohne Client: nur die Maske folgt, gemerkt wird nichts.
        #expect(flow.deviceNameForMask(from: renamed, for: Self.server, current: "Mac") == "Büro-Mac")
        #expect(flow.deviceNameForMask(from: renamed, for: Self.server, current: "Büro-Mac") == nil)
        #expect(store.client(for: Self.server) == nil)
        let browser = try Self.me(#"{"auth": "web", "device": null}"#)
        #expect(flow.deviceNameForMask(from: browser, for: Self.server, current: "Mac") == nil)

        // Mit Client: wie adoptDeviceName, der gemerkte Name zählt, nicht die Maske.
        store.remember(RememberedClient(clientId: "cl_1", name: "Rubens Mac"), for: Self.server)
        #expect(flow.deviceNameForMask(from: renamed, for: Self.server, current: "Büro-Mac") == "Büro-Mac")
        #expect(store.client(for: Self.server) == RememberedClient(clientId: "cl_1", name: "Büro-Mac"))
        #expect(flow.deviceNameForMask(from: renamed, for: Self.server, current: "Büro-Mac") == nil)

        // Ohne Speicher (Tests, ältere Aufrufer) wie ohne Client.
        let bare = DeviceLoginFlow(transport: ScriptedTransport([]))
        #expect(bare.deviceNameForMask(from: renamed, for: Self.server, current: "Mac") == "Büro-Mac")
    }

    @Test func userDefaultsStore() throws {
        let suite = "lichtung-test-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = UserDefaultsClientStore(defaults: defaults)
        #expect(store.client(for: Self.server) == nil)
        store.remember(RememberedClient(clientId: "cl_1", name: "Mac"), for: Self.server)
        store.remember(RememberedClient(clientId: "cl_2", name: "Mac"), for: URL(string: "http://localhost:8080")!)
        // Ein neuer Speicher auf denselben Defaults liest denselben Stand.
        let again = UserDefaultsClientStore(defaults: defaults)
        #expect(again.client(for: URL(string: "https://lichtung.example.org/")!)?.clientId == "cl_1")
        again.forget(server: Self.server)
        #expect(store.client(for: Self.server) == nil)
        #expect(store.client(for: URL(string: "http://localhost:8080")!)?.clientId == "cl_2")
    }
}
