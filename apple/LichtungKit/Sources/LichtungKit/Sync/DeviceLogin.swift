import Foundation

/// Anmeldung eines Geräts per OAuth (Authorization Code mit PKCE, Vertrag §2).
///
/// Nur die Bausteine ohne Oberfläche: URLs bauen, Antworten lesen. Das
/// Browserfenster (`ASWebAuthenticationSession`) öffnet die App.
public struct DeviceLogin: Sendable {
    public static let redirectURI = "dev.rubeen.lichtung:/oauth"

    public var serverURL: URL
    public var clientId: String
    public var codeVerifier: String
    public var state: String

    public init(serverURL: URL, clientId: String, codeVerifier: String = DeviceLogin.randomToken(), state: String = DeviceLogin.randomToken(length: 16)) {
        self.serverURL = serverURL
        self.clientId = clientId
        self.codeVerifier = codeVerifier
        self.state = state
    }

    /// `S256`-Challenge zum Verifier (RFC 7636).
    public var codeChallenge: String {
        Data(SHA256.hash(Array(codeVerifier.utf8))).base64URLEncoded
    }

    /// Ressource, für die das Token gilt.
    public var resource: String {
        serverURL.appendingPathComponent("api").absoluteString
    }

    /// Body für `POST /oauth/register`.
    public static func registrationBody(deviceName: String) throws -> Data {
        struct Body: Encodable {
            var clientName: String
            var redirectUris = [DeviceLogin.redirectURI]
            var tokenEndpointAuthMethod = "none"
            var grantTypes = ["authorization_code"]
            var responseTypes = ["code"]
        }
        return try APICoding.encoder.encode(Body(clientName: deviceName))
    }

    public func authorizeURL(endpoint: URL) -> URL {
        var c = URLComponents(url: endpoint, resolvingAgainstBaseURL: false)!
        c.queryItems = [
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "client_id", value: clientId),
            URLQueryItem(name: "redirect_uri", value: Self.redirectURI),
            URLQueryItem(name: "code_challenge", value: codeChallenge),
            URLQueryItem(name: "code_challenge_method", value: "S256"),
            URLQueryItem(name: "state", value: state),
            URLQueryItem(name: "resource", value: resource),
        ]
        return c.url!
    }

    public enum CallbackError: Error, Equatable {
        case stateMismatch
        case denied(String)
        case missingCode
    }

    /// Liest `dev.rubeen.lichtung:/oauth?code=…&state=…`.
    public func code(fromCallback url: URL) throws -> String {
        let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        func value(_ n: String) -> String? { items.first { $0.name == n }?.value }
        if let e = value("error") { throw CallbackError.denied(value("error_description") ?? e) }
        guard value("state") == state else { throw CallbackError.stateMismatch }
        guard let code = value("code"), !code.isEmpty else { throw CallbackError.missingCode }
        return code
    }

    /// Body für `POST /oauth/token` (`application/x-www-form-urlencoded`).
    public func tokenRequestBody(code: String) -> Data {
        let fields = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", Self.redirectURI),
            ("client_id", clientId),
            ("code_verifier", codeVerifier),
            ("resource", resource),
        ]
        return Data(fields.map { "\($0)=\(Self.formEncode($1))" }.joined(separator: "&").utf8)
    }

    public struct TokenResponse: Decodable, Sendable {
        public var accessToken: String
        public var tokenType: String
    }

    static func formEncode(_ s: String) -> String {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~")
        return s.addingPercentEncoding(withAllowedCharacters: allowed) ?? s
    }

    /// Zufälliger URL-sicherer Text (43–128 Zeichen für PKCE).
    public static func randomToken(length: Int = 64) -> String {
        let alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
        var g = SystemRandomNumberGenerator()
        return String((0..<length).map { _ in alphabet[Int(g.next(upperBound: UInt64(alphabet.count)))] })
    }
}

extension Data {
    var base64URLEncoded: String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

/// SHA-256 (FIPS 180-4). Eigene Umsetzung, weil CryptoKit unter Linux fehlt
/// und der Kern ohne Abhängigkeiten auskommen soll. Nur für PKCE gedacht.
enum SHA256 {
    private static let k: [UInt32] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ]

    static func hash(_ message: [UInt8]) -> [UInt8] {
        var h: [UInt32] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]
        var data = message
        let bitLength = UInt64(message.count) * 8
        data.append(0x80)
        while data.count % 64 != 56 { data.append(0) }
        for i in (0..<8).reversed() { data.append(UInt8(truncatingIfNeeded: bitLength >> (UInt64(i) * 8))) }

        func rotr(_ x: UInt32, _ n: UInt32) -> UInt32 { (x >> n) | (x << (32 - n)) }

        var w = [UInt32](repeating: 0, count: 64)
        for chunk in stride(from: 0, to: data.count, by: 64) {
            for i in 0..<16 {
                let b = chunk + i * 4
                w[i] = UInt32(data[b]) << 24 | UInt32(data[b + 1]) << 16 | UInt32(data[b + 2]) << 8 | UInt32(data[b + 3])
            }
            for i in 16..<64 {
                let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3)
                let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10)
                w[i] = w[i - 16] &+ s0 &+ w[i - 7] &+ s1
            }
            var (a, b, c, d, e, f, g, hh) = (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7])
            for i in 0..<64 {
                let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)
                let ch = (e & f) ^ (~e & g)
                let t1 = hh &+ s1 &+ ch &+ k[i] &+ w[i]
                let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)
                let maj = (a & b) ^ (a & c) ^ (b & c)
                let t2 = s0 &+ maj
                hh = g; g = f; f = e; e = d &+ t1
                d = c; c = b; b = a; a = t1 &+ t2
            }
            h[0] = h[0] &+ a; h[1] = h[1] &+ b; h[2] = h[2] &+ c; h[3] = h[3] &+ d
            h[4] = h[4] &+ e; h[5] = h[5] &+ f; h[6] = h[6] &+ g; h[7] = h[7] &+ hh
        }
        return h.flatMap { v in (0..<4).map { UInt8(truncatingIfNeeded: v >> (24 - $0 * 8)) } }
    }
}
