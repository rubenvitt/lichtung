// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import AuthenticationServices
import Foundation
import LichtungKit
import Observation
#if os(iOS)
import UIKit
#endif

/// Anmeldung, Wechsel zwischen Demo und Server, Live-Abgleich (LIC-43).
///
/// Gespeichert wird: die Server-Adresse, die Wahl „Demo“ und der OAuth-Client
/// je Server (LIC-49) in den UserDefaults, das Geräte-Token in der Keychain,
/// der Stand in `Application Support/Lichtung/cache.json`.
@MainActor
@Observable
final class AppSession {
    enum Stage: Equatable {
        case starting, signedOut, demo, server
    }

    private(set) var stage: Stage = .starting
    /// Stand für alle Ansichten. Auch abgemeldet gibt es eins (Demo), damit
    /// Menüs und Szenen nie ohne Modell dastehen.
    private(set) var model: AppModel
    /// Der Server hat das Token abgelehnt (`401`): Der Kalender bleibt
    /// lesbar, Cache und Outbox bleiben, die App bittet um neue Anmeldung.
    private(set) var needsSignIn = false
    private(set) var serverURL: URL?
    /// Verwaltung (Vertrag §7); `nil` im Demo-Modus.
    private(set) var management: ManagementAPI?
    /// Letzte Antwort von `GET /api/me` (Person, Gerät, Server-Version).
    /// Abgleich des Gerätenamens und Einstellungen teilen den Abruf (LIC-56).
    private(set) var me: ServerInfo?

    // Anmeldemaske
    var serverInput = ""
    var deviceName = ""
    private(set) var loginPhase: LoginPhase = .idle

    /// Einstellungen (Konten, Kalender, Geräte …), lebt so lange wie die Sitzung.
    let settings: SettingsModel

    private let tokens = StoredDeviceToken(store: KeychainSecretStore())
    /// Merkt sich den OAuth-Client je Server: Eine erneute Anmeldung
    /// registriert kein neues Gerät beim Server (Vertrag §2, LIC-49).
    private let flow = DeviceLoginFlow(clients: UserDefaultsClientStore())
    private let defaults = UserDefaults.standard
    @ObservationIgnored private var syncAPI: SyncAPI?
    @ObservationIgnored private var liveTask: Task<Void, Never>?
    @ObservationIgnored private var meTask: Task<Void, Never>?
    @ObservationIgnored private let meFetch: SharedFetch<ServerInfo>
    @ObservationIgnored private var isActive = false

    private enum Key {
        static let mode = "session.mode"
        static let server = "session.serverURL"
        /// Server, zu dem die Cache-Datei gehört.
        static let cacheServer = "session.cacheServerURL"
    }

    /// So lange gilt eine Antwort von `/api/me` für die Einstellungen, ohne
    /// neu zu fragen (Einstellungen direkt nach dem Aktivwerden öffnen).
    static let meMaxAge: TimeInterval = 30

    init() {
        meFetch = SharedFetch()
        model = AppModel.demo()
        settings = SettingsModel()
        deviceName = Self.defaultDeviceName
        settings.attach(self)
        // Die AppSession lebt so lange wie die App; der starke Verweis ist gewollt.
        tokens.setRejectionHandler {
            Task { @MainActor in self.tokenWasRejected() }
        }
        restore()
    }

    static var cacheURL: URL {
        let base = (try? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true))
            ?? FileManager.default.temporaryDirectory
        return base.appendingPathComponent("Lichtung", isDirectory: true).appendingPathComponent("cache.json")
    }

    static var defaultDeviceName: String {
        #if os(macOS)
        return Host.current().localizedName ?? "Mac"
        #else
        // Seit iOS 16 liefert `name` ohne Sonderrecht nur „iPhone“; das Modell
        // reicht, die Person kann den Namen in der Maske ändern.
        return UIDevice.current.model
        #endif
    }

    /// Ausstehende Änderungen, die beim Abmelden verloren gingen.
    var pendingChangeCount: Int { model.state.outbox.count }

    // MARK: Start

    private func restore() {
        let start = SessionStart.decide(
            mode: defaults.string(forKey: Key.mode),
            serverURL: defaults.string(forKey: Key.server),
            hasToken: tokens.hasToken,
            hasCache: FileManager.default.fileExists(atPath: Self.cacheURL.path)
        )
        switch start {
        case .demo:
            enterDemo()
        case let .server(url):
            enterServer(url)
        case let .reauthenticate(url):
            enterServer(url)
            needsSignIn = true
            serverInput = ServerAddress.displayName(url)
            prefillDeviceName()
        case let .login(prefill):
            serverInput = prefill
            prefillDeviceName()
            stage = .signedOut
        }
    }

    /// Name, unter dem dieses Gerät beim Server der Maske schon bekannt ist.
    /// Sonst würde eine erneute Anmeldung den Namen auf den Standard
    /// zurücksetzen.
    private func prefillDeviceName() {
        if let name = flow.rememberedDeviceName(serverInput: serverInput) { deviceName = name }
    }

    // MARK: /api/me

    /// Gemeinsamer Abruf von `GET /api/me` (LIC-56) für den Abgleich beim
    /// Aktivwerden und die Einstellungen: Läuft schon einer, wartet der
    /// Aufruf auf ihn; eine Antwort jünger als `maxAge` gilt ohne Anfrage.
    ///
    /// Jede Antwort gleicht den Gerätenamen ab (LIC-55): Ein anderes Gerät
    /// kann dieses umbenannt haben, der gemerkte Name folgt dem Server, damit
    /// die Anmeldemaske nach dem nächsten Abmelden den aktuellen zeigt.
    /// Liefert `nil` ohne Server oder wenn inzwischen ein anderer gilt.
    @discardableResult
    func loadMe(maxAge: TimeInterval = 0) async throws -> ServerInfo? {
        guard let management, let serverURL else { return nil }
        let info = try await meFetch.load(maxAge: maxAge) { try await management.me() }
        guard self.serverURL == serverURL else { return nil }
        if me != info { me = info }
        if let name = flow.deviceNameForMask(from: info, for: serverURL, current: deviceName), name != deviceName {
            deviceName = name
        }
        return info
    }

    /// Beim Start des Live-Abgleichs, also beim Aktivwerden.
    private func refreshMe() {
        guard stage == .server, !needsSignIn, management != nil else { return }
        meTask?.cancel()
        meTask = Task { [weak self] in
            // Ohne Verbindung bleibt der gemerkte Name; `401` meldet der Token-Provider.
            _ = try? await self?.loadMe()
        }
    }

    /// Anderer Server, Demo, Abmelden: nichts vom alten stehen lassen.
    private func forgetMe() {
        meTask?.cancel()
        meTask = nil
        meFetch.reset()
        me = nil
    }

    /// Dieses Gerät wurde in den Einstellungen umbenannt (LIC-46).
    func currentDeviceRenamed(_ name: String) {
        deviceName = name
        me?.device?.name = name
        // Die gemerkte Antwort trägt noch den alten Namen, ein laufender
        // Abruf vielleicht auch: verwerfen, damit er den neuen nicht überschreibt.
        meFetch.reset()
        if let serverURL { flow.rememberDeviceName(name, for: serverURL) }
    }

    // MARK: Anmelden

    /// Ganzer Ablauf (Vertrag §2). `authenticate` öffnet das Browserfenster
    /// (`WebAuthenticationSession` aus der Ansicht) und liefert den Rückruf.
    func signIn(authenticate: @MainActor (URL) async throws -> URL) async {
        guard !loginPhase.isBusy else { return }
        do {
            loginPhase = .checkingServer
            let (server, metadata) = try await flow.discover(serverInput: serverInput)
            loginPhase = .registering
            // Gemerkter Client, wenn der Server ihn noch kennt, sonst neu registriert.
            let prepared = try await flow.prepare(serverURL: server, metadata: metadata, deviceName: deviceName)
            loginPhase = .waitingForBrowser
            let callback = try await authenticate(prepared.authorizeURL)
            loginPhase = .fetchingToken
            let credentials = try await flow.finish(prepared, callback: callback)
            try tokens.save(credentials.token)
            completeSignIn(credentials.serverURL)
            loginPhase = .signedIn
        } catch {
            loginPhase = .failed(Self.failure(from: error))
        }
    }

    private static func failure(from error: Error) -> LoginFailure {
        if let e = error as? ASWebAuthenticationSessionError, e.code == .canceledLogin { return .cancelled }
        if let e = error as? KeychainError { return .unexpected(e.localizedDescription) }
        return LoginFailure.from(error)
    }

    private func completeSignIn(_ url: URL) {
        let cached = defaults.string(forKey: Key.cacheServer).flatMap { URL(string: $0) }
        let keepsCache = ServerAddress.cacheBelongs(to: cached, newServer: url)
        if !keepsCache {
            // Anderer Server: Der alte Stand gehört nicht hierher.
            try? FileCacheStore(url: Self.cacheURL).clear()
        }
        defaults.set(url.absoluteString, forKey: Key.server)
        defaults.set(url.absoluteString, forKey: Key.cacheServer)
        defaults.set(SessionStart.serverMode, forKey: Key.mode)
        needsSignIn = false
        if stage == .server, serverURL == url, keepsCache {
            // Neu angemeldet nach `401`: Engine und Outbox bleiben, das neue
            // Token liest derselbe Provider.
            if isActive { startLive() }
        } else {
            enterServer(url)
        }
    }

    // MARK: Modi

    private func enterServer(_ url: URL) {
        stopLive()
        let api = SyncAPI(baseURL: url, tokens: tokens)
        syncAPI = api
        model = AppModel(engine: SyncEngine(api: api, store: FileCacheStore(url: Self.cacheURL)))
        management = ManagementAPI(sync: api)
        serverURL = url
        stage = .server
        forgetMe()
        settings.reset()
        if isActive { startLive() }
    }

    private func enterDemo() {
        stopLive()
        model = AppModel.demo()
        syncAPI = nil
        management = nil
        serverURL = nil
        needsSignIn = false
        stage = .demo
        forgetMe()
        settings.reset()
    }

    /// „Ohne Server ausprobieren“.
    func startDemo() {
        defaults.set(SessionStart.demoMode, forKey: Key.mode)
        loginPhase = .idle
        enterDemo()
    }

    /// Aus dem Demo-Modus zur Anmeldung.
    func leaveDemo() {
        defaults.removeObject(forKey: Key.mode)
        serverInput = defaults.string(forKey: Key.server).flatMap { URL(string: $0) }.map(ServerAddress.displayName) ?? ""
        prefillDeviceName()
        loginPhase = .idle
        model = AppModel.demo()
        stage = .signedOut
    }

    /// Abmelden: Token beim Server widerrufen (ohne Verbindung bleibt das
    /// Gerät in der Liste und lässt sich dort entfernen), Keychain und Cache
    /// leeren. Die Adresse bleibt für die nächste Anmeldung stehen.
    func signOut() async {
        stopLive()
        if let url = serverURL, let token = tokens.current {
            await flow.revoke(token: token, serverURL: url)
        }
        try? tokens.clear()
        try? FileCacheStore(url: Self.cacheURL).clear()
        defaults.removeObject(forKey: Key.cacheServer)
        defaults.removeObject(forKey: Key.mode)
        serverInput = serverURL.map(ServerAddress.displayName) ?? serverInput
        prefillDeviceName()
        syncAPI = nil
        management = nil
        serverURL = nil
        needsSignIn = false
        loginPhase = .idle
        model = AppModel.demo()
        forgetMe()
        settings.reset()
        stage = .signedOut
    }

    // MARK: 401

    private func tokenWasRejected() {
        guard stage == .server else { return }
        needsSignIn = true
        loginPhase = .idle
        if let serverURL { serverInput = ServerAddress.displayName(serverURL) }
        prefillDeviceName()
        stopLive()
    }

    // MARK: Live-Abgleich (SSE)

    /// Aus `scenePhase`: SSE nur, solange die App offen ist (Vertrag §5).
    func setActive(_ active: Bool) {
        isActive = active
        if active { startLive() } else { stopLive() }
    }

    private func startLive() {
        guard stage == .server, !needsSignIn, liveTask == nil, let api = syncAPI else { return }
        refreshMe()
        let engine = model.engine
        liveTask = Task { [weak self] in
            let stop = await LiveSync.run(engine: engine, connect: { SyncEventStream(api: api).cursors() })
            if stop == .unauthorized { self?.liveSyncLostAuthorization() }
        }
    }

    private func liveSyncLostAuthorization() {
        // Auch ohne gespeichertes Token (dann meldet der Provider nichts).
        liveTask = nil
        tokenWasRejected()
    }

    private func stopLive() {
        liveTask?.cancel()
        liveTask = nil
        meTask?.cancel()
        meTask = nil
    }
}
