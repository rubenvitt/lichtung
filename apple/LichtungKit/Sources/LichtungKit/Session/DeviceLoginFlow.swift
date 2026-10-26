import Foundation

/// Metadaten des OAuth-Servers (`/.well-known/oauth-authorization-server`,
/// RFC 8414). Nur die Felder, die die Anmeldung braucht.
public struct AuthorizationServerMetadata: Decodable, Hashable, Sendable {
    public var issuer: String
    public var authorizationEndpoint: URL
    public var tokenEndpoint: URL
    public var registrationEndpoint: URL?
    public var revocationEndpoint: URL?
    public var codeChallengeMethodsSupported: [String]?

    public init(issuer: String, authorizationEndpoint: URL, tokenEndpoint: URL, registrationEndpoint: URL? = nil,
                revocationEndpoint: URL? = nil, codeChallengeMethodsSupported: [String]? = nil) {
        self.issuer = issuer
        self.authorizationEndpoint = authorizationEndpoint
        self.tokenEndpoint = tokenEndpoint
        self.registrationEndpoint = registrationEndpoint
        self.revocationEndpoint = revocationEndpoint
        self.codeChallengeMethodsSupported = codeChallengeMethodsSupported
    }
}

/// Was bei der Anmeldung schiefgehen kann, mit Text für die Oberfläche.
public enum LoginFailure: Error, Hashable, Sendable, LocalizedError {
    case invalidAddress(ServerAddress.Problem)
    /// Keine Verbindung (offline, Name unbekannt, Zertifikat …).
    case unreachable(String)
    /// Unter der Adresse antwortet etwas, aber kein Lichtung-Server.
    case notLichtung
    /// Der Server nennt sich anders als eingegeben (RFC 8414 §3.3).
    case issuerMismatch(String)
    case registrationFailed(String?)
    /// Die Person hat das Anmeldefenster geschlossen.
    case cancelled
    /// Am Server abgelehnt (Bestätigungsseite, Pocket-ID).
    case denied(String)
    /// Rückruf passt nicht zur gestarteten Anmeldung.
    case stateMismatch
    case missingCode
    case tokenFailed(String?)
    /// Antwort enthält kein Geräte-Token (`ld_…`).
    case notDeviceToken
    case unexpected(String)

    public var message: String {
        switch self {
        case let .invalidAddress(p): p.message
        case .unreachable: "Der Server ist nicht erreichbar. Stimmt die Adresse, und ist das Gerät online?"
        case .notLichtung: "Unter dieser Adresse antwortet kein Lichtung-Server."
        case let .issuerMismatch(issuer): "Der Server meldet sich unter einer anderen Adresse (\(issuer)). Bitte diese eingeben."
        case let .registrationFailed(m): "Das Gerät ließ sich nicht registrieren" + Self.detail(m)
        case .cancelled: "Anmeldung abgebrochen."
        case let .denied(m): "Anmeldung abgelehnt" + Self.detail(m)
        case .stateMismatch: "Die Antwort gehört nicht zu dieser Anmeldung. Bitte noch einmal versuchen."
        case .missingCode: "Der Server hat keinen Anmeldecode geschickt. Bitte noch einmal versuchen."
        case let .tokenFailed(m): "Der Server hat kein Token ausgegeben" + Self.detail(m)
        case .notDeviceToken: "Der Server hat kein Geräte-Token ausgegeben. Passt die Server-Adresse zur öffentlichen URL des Servers?"
        case let .unexpected(m): "Unerwarteter Fehler: \(m)"
        }
    }

    public var errorDescription: String? { message }

    /// Lohnt ein Hinweis „noch einmal“? Abbrechen ist kein Fehler.
    public var isCancellation: Bool { self == .cancelled }

    private static func detail(_ m: String?) -> String {
        guard let m, !m.isEmpty else { return "." }
        return ": \(m)"
    }

    /// Ordnet beliebige Fehler aus dem Ablauf ein.
    public static func from(_ error: Error) -> LoginFailure {
        switch error {
        case let f as LoginFailure: return f
        case let p as ServerAddress.Problem: return .invalidAddress(p)
        case let c as DeviceLogin.CallbackError:
            switch c {
            case .stateMismatch: return .stateMismatch
            case let .denied(m): return .denied(m)
            case .missingCode: return .missingCode
            }
        case let a as APIError:
            switch a {
            case let .transport(m): return .unreachable(m)
            case .decoding: return .notLichtung
            default: return .unexpected(a.userMessage)
            }
        case is CancellationError: return .cancelled
        default: return .unexpected(String(describing: error))
        }
    }
}

/// Phasen der Anmeldung für die Oberfläche (LIC-43).
public enum LoginPhase: Hashable, Sendable {
    case idle
    case checkingServer
    case registering
    /// Das Anmeldefenster (`ASWebAuthenticationSession`) ist offen.
    case waitingForBrowser
    case fetchingToken
    case signedIn
    case failed(LoginFailure)

    public var isBusy: Bool {
        switch self {
        case .checkingServer, .registering, .waitingForBrowser, .fetchingToken: true
        default: false
        }
    }

    public var statusText: String? {
        switch self {
        case .idle, .signedIn: nil
        case .checkingServer: "Server wird geprüft …"
        case .registering: "Gerät wird registriert …"
        case .waitingForBrowser: "Anmeldung im Browserfenster …"
        case .fetchingToken: "Zugang wird eingerichtet …"
        case let .failed(f): f.isCancellation ? nil : f.message
        }
    }

    public var failure: LoginFailure? {
        if case let .failed(f) = self, !f.isCancellation { return f }
        return nil
    }
}

/// Bis zum Öffnen des Browserfensters vorbereitete Anmeldung.
public struct PreparedLogin: Sendable {
    public var serverURL: URL
    public var metadata: AuthorizationServerMetadata
    public var login: DeviceLogin

    public var authorizeURL: URL { login.authorizeURL(endpoint: metadata.authorizationEndpoint) }

    /// Schema, auf das `ASWebAuthenticationSession` wartet.
    public static let callbackScheme = "dev.rubeen.lichtung"
}

/// Ergebnis einer Anmeldung.
public struct DeviceCredentials: Hashable, Sendable {
    public var serverURL: URL
    public var token: String
    public var clientId: String
}

/// Ablauf der Geräte-Anmeldung ohne Oberfläche (Vertrag §2): Discovery,
/// Registrierung, Token. Das Browserfenster dazwischen öffnet die App.
///
/// Die Schritte sind einzeln aufrufbar, damit die App zwischen ihnen die
/// Phase anzeigen kann.
public struct DeviceLoginFlow: Sendable {
    public static let deviceTokenPrefix = "ld_"

    public var transport: HTTPTransport

    public init(transport: HTTPTransport = URLSessionTransport()) {
        self.transport = transport
    }

    /// Schritt 1: Adresse prüfen, Metadaten holen. Liefert die Basis-URL, wie
    /// der Server selbst sie nennt (`issuer`).
    public func discover(serverInput: String) async throws -> (serverURL: URL, metadata: AuthorizationServerMetadata) {
        let typed: URL
        do { typed = try ServerAddress.normalize(serverInput) } catch { throw LoginFailure.from(error) }
        let response = try await send(HTTPRequest(
            method: "GET", url: typed.appendingPathComponent(".well-known/oauth-authorization-server"),
            headers: ["Accept": "application/json"]
        ))
        guard (200..<300).contains(response.status),
              let metadata = try? APICoding.decoder.decode(AuthorizationServerMetadata.self, from: response.body)
        else { throw LoginFailure.notLichtung }
        let server = try Self.resolveServer(typed: typed, issuer: metadata.issuer)
        for endpoint in [metadata.authorizationEndpoint, metadata.tokenEndpoint] where !ServerAddress.sameOrigin(endpoint, server) {
            throw LoginFailure.issuerMismatch(endpoint.absoluteString)
        }
        if let methods = metadata.codeChallengeMethodsSupported, !methods.contains("S256") { throw LoginFailure.notLichtung }
        return (server, metadata)
    }

    /// Basis-URL nach RFC 8414 §3.3: der `issuer`, sofern er auf demselben
    /// Server liegt wie die Eingabe. Ein fremder Host wird abgelehnt.
    static func resolveServer(typed: URL, issuer: String) throws -> URL {
        guard let issuerURL = try? ServerAddress.normalize(issuer) else { throw LoginFailure.notLichtung }
        guard ServerAddress.sameOrigin(issuerURL, typed) else { throw LoginFailure.issuerMismatch(issuer) }
        return issuerURL
    }

    /// Schritt 2: Gerät registrieren, Verifier und `state` erzeugen.
    public func register(serverURL: URL, metadata: AuthorizationServerMetadata, deviceName: String) async throws -> PreparedLogin {
        let name = deviceName.trimmingCharacters(in: .whitespacesAndNewlines)
        let endpoint = metadata.registrationEndpoint ?? serverURL.appendingPathComponent("oauth/register")
        let response = try await send(HTTPRequest(
            method: "POST", url: endpoint,
            headers: ["Content-Type": "application/json", "Accept": "application/json"],
            body: try DeviceLogin.registrationBody(deviceName: name.isEmpty ? "Lichtung" : name)
        ))
        guard (200..<300).contains(response.status) else {
            throw LoginFailure.registrationFailed(Self.oauthErrorText(response))
        }
        struct Registration: Decodable { var clientId: String }
        guard let reg = try? APICoding.decoder.decode(Registration.self, from: response.body), !reg.clientId.isEmpty else {
            throw LoginFailure.registrationFailed(nil)
        }
        return PreparedLogin(serverURL: serverURL, metadata: metadata, login: DeviceLogin(serverURL: serverURL, clientId: reg.clientId))
    }

    /// Schritt 4: Rückruf lesen und Code gegen das Geräte-Token tauschen.
    public func finish(_ prepared: PreparedLogin, callback: URL) async throws -> DeviceCredentials {
        let code: String
        do { code = try prepared.login.code(fromCallback: callback) } catch { throw LoginFailure.from(error) }
        let response = try await send(HTTPRequest(
            method: "POST", url: prepared.metadata.tokenEndpoint,
            headers: ["Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"],
            body: prepared.login.tokenRequestBody(code: code)
        ))
        guard (200..<300).contains(response.status) else { throw LoginFailure.tokenFailed(Self.oauthErrorText(response)) }
        guard let token = try? APICoding.decoder.decode(DeviceLogin.TokenResponse.self, from: response.body) else {
            throw LoginFailure.tokenFailed(nil)
        }
        guard token.accessToken.hasPrefix(Self.deviceTokenPrefix) else { throw LoginFailure.notDeviceToken }
        return DeviceCredentials(serverURL: prepared.serverURL, token: token.accessToken, clientId: prepared.login.clientId)
    }

    /// Abmelden (Vertrag §2, Schritt 5). Ohne Verbindung bleibt das Gerät in
    /// der Liste stehen und lässt sich dort entfernen; deshalb kein Fehler.
    @discardableResult
    public func revoke(token: String, serverURL: URL) async -> Bool {
        let body = Data("token=\(DeviceLogin.formEncode(token))".utf8)
        let request = HTTPRequest(
            method: "POST", url: serverURL.appendingPathComponent("oauth/revoke"),
            headers: ["Content-Type": "application/x-www-form-urlencoded"], body: body
        )
        guard let r = try? await send(request) else { return false }
        return (200..<300).contains(r.status)
    }

    private func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        do {
            return try await transport.send(request)
        } catch let e as APIError {
            throw LoginFailure.from(e)
        } catch is CancellationError {
            throw LoginFailure.cancelled
        } catch {
            throw LoginFailure.unreachable(error.localizedDescription)
        }
    }

    /// `error_description` bzw. `error` einer OAuth-Fehlerantwort (RFC 6749 §5.2).
    static func oauthErrorText(_ response: HTTPResponse) -> String? {
        struct OAuthError: Decodable { var error: String?; var errorDescription: String? }
        guard let e = try? APICoding.decoder.decode(OAuthError.self, from: response.body) else { return nil }
        return e.errorDescription ?? e.error
    }
}
