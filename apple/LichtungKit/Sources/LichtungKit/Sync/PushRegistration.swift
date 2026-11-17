import Foundation

/// Anmeldung für stille Push-Nachrichten (APNs, LIC-27; Vertrag §8).
///
/// Ändern sich Termine, weckt der Server die App mit einer stillen
/// Push-Nachricht; sie zieht nach und plant ihre Erinnerungen neu, auch für
/// Termine, die sie nie gesehen hat (z. B. morgens per MCP angelegt).
public struct PushRegistration: Hashable, Sendable, Encodable {
    /// Umgebung, für die APNs den Token ausgestellt hat.
    public enum Environment: String, Hashable, Sendable, Encodable {
        /// Builds aus Xcode (Entwicklungsprofil).
        case sandbox
        /// TestFlight und App Store.
        case production
    }

    /// Device-Token als Hex in Kleinbuchstaben.
    public var apnsToken: String
    public var environment: Environment

    public init(apnsToken: String, environment: Environment) {
        self.apnsToken = apnsToken
        self.environment = environment
    }

    /// Aus den Bytes, die iOS in `didRegisterForRemoteNotificationsWithDeviceToken` liefert.
    public init(deviceToken: Data, environment: Environment) {
        let digits = Array("0123456789abcdef")
        var hex = ""
        hex.reserveCapacity(deviceToken.count * 2)
        for byte in deviceToken {
            hex.append(digits[Int(byte >> 4)])
            hex.append(digits[Int(byte & 0x0F)])
        }
        self.init(apnsToken: hex, environment: environment)
    }
}

extension ManagementAPI {
    // MARK: Push (LIC-27)

    /// `PUT /api/devices/current/push`: Dieses Gerät bekommt stille
    /// Push-Nachrichten. Nur mit einem Geräte-Token erlaubt.
    public func registerPush(_ registration: PushRegistration) async throws {
        _ = try await sync.send(HTTPRequest(
            method: "PUT", url: sync.endpoint("api/devices/current/push"),
            headers: ["Content-Type": "application/json"],
            body: try APICoding.encoder.encode(registration)
        ))
    }

    /// `DELETE /api/devices/current/push`: keine Push-Nachrichten mehr.
    public func unregisterPush() async throws {
        try await sendEmpty("DELETE", "api/devices/current/push")
    }
}
