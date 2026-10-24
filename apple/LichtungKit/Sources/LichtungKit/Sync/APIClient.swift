import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// Eine HTTP-Anfrage, unabhängig von URLSession (für Tests austauschbar).
public struct HTTPRequest: Hashable, Sendable {
    public var method: String
    public var url: URL
    public var headers: [String: String]
    public var body: Data?

    public init(method: String, url: URL, headers: [String: String] = [:], body: Data? = nil) {
        self.method = method
        self.url = url
        self.headers = headers
        self.body = body
    }
}

public struct HTTPResponse: Hashable, Sendable {
    public var status: Int
    public var headers: [String: String]
    public var body: Data

    public init(status: Int, headers: [String: String] = [:], body: Data = Data()) {
        self.status = status
        self.headers = headers
        self.body = body
    }
}

public protocol HTTPTransport: Sendable {
    func send(_ request: HTTPRequest) async throws -> HTTPResponse
}

/// Transport über `URLSession`.
public struct URLSessionTransport: HTTPTransport {
    public let session: URLSession

    public init(session: URLSession = .shared) { self.session = session }

    public func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        var r = URLRequest(url: request.url)
        r.httpMethod = request.method
        r.httpBody = request.body
        for (k, v) in request.headers { r.setValue(v, forHTTPHeaderField: k) }
        let (data, response) = try await session.data(for: r)
        guard let http = response as? HTTPURLResponse else { throw APIError.transport("Keine HTTP-Antwort") }
        var headers: [String: String] = [:]
        for (k, v) in http.allHeaderFields {
            if let k = k as? String, let v = v as? String { headers[k.lowercased()] = v }
        }
        return HTTPResponse(status: http.statusCode, headers: headers, body: data)
    }
}

/// Fehler der Sync-API (Vertrag §9).
public enum APIError: Error, Hashable, Sendable {
    /// Token ungültig oder widerrufen: neu anmelden. Cache und Outbox bleiben.
    case unauthorized
    /// Pull-Cursor zu alt: Spiegel leeren und mit `since=0` neu ziehen.
    case cursorExpired
    case tooManyRequests(retryAfter: TimeInterval?)
    case server(status: Int, code: String?, message: String?)
    case transport(String)
    case decoding(String)

    /// Lohnt sich ein späterer neuer Versuch?
    public var isTransient: Bool {
        switch self {
        case .tooManyRequests, .transport: true
        case let .server(status, _, _): status == 502 || status == 503 || status >= 500
        default: false
        }
    }
}

/// Zugangsdaten für die API.
public protocol TokenProvider: Sendable {
    func deviceToken() async -> String?
}

public struct StaticToken: TokenProvider {
    public var token: String?
    public init(_ token: String?) { self.token = token }
    public func deviceToken() async -> String? { token }
}

/// Aufrufe der Sync-API (`/api/sync/…`).
public struct SyncAPI: Sendable {
    public var baseURL: URL
    public var transport: HTTPTransport
    public var tokens: TokenProvider

    public init(baseURL: URL, transport: HTTPTransport = URLSessionTransport(), tokens: TokenProvider) {
        self.baseURL = baseURL
        self.transport = transport
        self.tokens = tokens
    }

    public func pull(since cursor: Int, limit: Int = 500) async throws -> PullPage {
        var c = URLComponents(url: endpoint("api/sync/pull"), resolvingAgainstBaseURL: false)!
        c.queryItems = [URLQueryItem(name: "since", value: String(cursor)), URLQueryItem(name: "limit", value: String(limit))]
        let response = try await send(HTTPRequest(method: "GET", url: c.url!))
        return try decode(PullPage.self, response)
    }

    public func push(_ mutations: [Mutation]) async throws -> [MutationResult] {
        let body = try APICoding.encoder.encode(PushRequest(mutations: mutations))
        let response = try await send(HTTPRequest(
            method: "POST", url: endpoint("api/sync/push"),
            headers: ["Content-Type": "application/json"], body: body
        ))
        return try decode(PushResponse.self, response).results
    }

    /// Anfrage für den SSE-Strom `GET /api/sync/events`.
    public func eventStreamRequest() async -> HTTPRequest {
        var r = HTTPRequest(method: "GET", url: endpoint("api/sync/events"), headers: ["Accept": "text/event-stream"])
        if let t = await tokens.deviceToken() { r.headers["Authorization"] = "Bearer \(t)" }
        return r
    }

    func endpoint(_ path: String) -> URL {
        baseURL.appendingPathComponent(path)
    }

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        var r = request
        guard let token = await tokens.deviceToken() else { throw APIError.unauthorized }
        r.headers["Authorization"] = "Bearer \(token)"
        r.headers["Accept"] = r.headers["Accept"] ?? "application/json"
        let response: HTTPResponse
        do {
            response = try await transport.send(r)
        } catch let e as APIError {
            throw e
        } catch {
            throw APIError.transport(error.localizedDescription)
        }
        guard (200..<300).contains(response.status) else { throw Self.error(from: response) }
        return response
    }

    static func error(from response: HTTPResponse) -> APIError {
        struct Envelope: Decodable { var error: APIErrorBody }
        let body = try? APICoding.decoder.decode(Envelope.self, from: response.body).error
        switch (response.status, body?.code) {
        case (401, _): return .unauthorized
        case (410, _), (_, "cursor_expired"?): return .cursorExpired
        case (429, _):
            return .tooManyRequests(retryAfter: response.headers["retry-after"].flatMap(TimeInterval.init))
        default:
            return .server(status: response.status, code: body?.code, message: body?.message)
        }
    }

    func decode<T: Decodable>(_ type: T.Type, _ response: HTTPResponse) throws -> T {
        do {
            return try APICoding.decoder.decode(type, from: response.body)
        } catch {
            throw APIError.decoding(String(describing: error))
        }
    }
}
