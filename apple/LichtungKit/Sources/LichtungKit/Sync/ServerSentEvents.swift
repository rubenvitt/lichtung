import Foundation

/// Ein Ereignis aus einem `text/event-stream`.
public struct ServerSentEvent: Hashable, Sendable {
    public var event: String
    public var data: String
    public var id: String?
}

/// Zerlegt einen SSE-Strom Zeile für Zeile (WHATWG-Format, so weit gebraucht).
///
/// Kommentarzeilen (`: keep-alive`) werden übergangen, mehrere `data:`-Zeilen
/// mit Zeilenumbruch verbunden, eine Leerzeile schließt ein Ereignis ab.
public struct ServerSentEventParser: Sendable {
    private var event = ""
    private var data: [String] = []
    private var id: String?

    public init() {}

    public mutating func feed(line raw: String) -> ServerSentEvent? {
        let line = raw.hasSuffix("\r") ? String(raw.dropLast()) : raw
        if line.isEmpty {
            defer { event = ""; data = []; id = nil }
            guard !data.isEmpty else { return nil }
            return ServerSentEvent(event: event.isEmpty ? "message" : event, data: data.joined(separator: "\n"), id: id)
        }
        if line.hasPrefix(":") { return nil }
        let field: Substring
        var value: Substring
        if let colon = line.firstIndex(of: ":") {
            field = line[..<colon]
            value = line[line.index(after: colon)...]
            if value.hasPrefix(" ") { value = value.dropFirst() }
        } else {
            field = Substring(line)
            value = ""
        }
        switch field {
        case "event": event = String(value)
        case "data": data.append(String(value))
        case "id": id = String(value)
        default: break
        }
        return nil
    }
}

/// Wartezeiten beim Neuverbinden: 1 s, 2 s, 4 s … bis 60 s (Vertrag §5).
public struct ReconnectBackoff: Sendable {
    public var initial: TimeInterval = 1
    public var maximum: TimeInterval = 60
    public private(set) var attempt = 0

    public init() {}

    public mutating func nextDelay() -> TimeInterval {
        defer { attempt += 1 }
        return min(maximum, initial * pow(2, Double(attempt)))
    }

    public mutating func reset() { attempt = 0 }
}

#if canImport(Darwin)
/// Liest `GET /api/sync/events` und liefert die gemeldeten Cursor.
/// Nur auf Apple-Plattformen (`URLSession.bytes`).
public struct SyncEventStream: Sendable {
    public let api: SyncAPI
    public let session: URLSession

    public init(api: SyncAPI, session: URLSession = .shared) {
        self.api = api
        self.session = session
    }

    /// Endet, wenn die Verbindung abbricht; neu verbinden mit `ReconnectBackoff`.
    public func cursors() -> AsyncThrowingStream<Int, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let req = await api.eventStreamRequest()
                    var r = URLRequest(url: req.url)
                    r.timeoutInterval = .infinity
                    for (k, v) in req.headers { r.setValue(v, forHTTPHeaderField: k) }
                    let (bytes, response) = try await session.bytes(for: r)
                    if let http = response as? HTTPURLResponse, http.statusCode == 401 {
                        await api.reportUnauthorized(req)
                        throw APIError.unauthorized
                    }
                    if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
                        throw APIError.server(status: http.statusCode, code: nil, message: nil)
                    }
                    var parser = ServerSentEventParser()
                    // Nicht `bytes.lines`: Das verschluckt Leerzeilen, und die
                    // schließen bei SSE ein Ereignis ab.
                    var buffer: [UInt8] = []
                    for try await byte in bytes {
                        guard byte == 0x0A else { buffer.append(byte); continue }
                        let line = String(decoding: buffer, as: UTF8.self)
                        buffer.removeAll(keepingCapacity: true)
                        if let e = parser.feed(line: line), e.event == "cursor", let c = Int(e.data) {
                            continuation.yield(c)
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}
#endif
