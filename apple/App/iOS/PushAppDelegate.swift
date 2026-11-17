import LichtungKit
import UIKit

/// Verbindet den App-Delegate (APNs) mit der Sitzung (LIC-27).
@MainActor
final class PushBridge {
    static let shared = PushBridge()
    /// Setzt `LichtungApp` beim Start.
    weak var session: AppSession?

    private init() {}

    /// Builds aus Xcode laufen mit dem Entwicklungsprofil (APNs-Sandbox),
    /// TestFlight-Builds in Produktion. Liegt der Server daneben, probiert er
    /// die andere Umgebung und merkt sie sich.
    #if DEBUG
    static let environment: PushRegistration.Environment = .sandbox
    #else
    static let environment: PushRegistration.Environment = .production
    #endif

    func requestToken() {
        // Nicht mitten im Start (die Sitzung stellt sich schon in `init` wieder her).
        Task { @MainActor in
            UIApplication.shared.registerForRemoteNotifications()
        }
    }

    func tokenReceived(_ deviceToken: Data) {
        session?.pushTokenReceived(PushRegistration(deviceToken: deviceToken, environment: Self.environment))
    }

    func silentPush() async -> UIBackgroundFetchResult {
        guard let session else { return .noData }
        return await session.handleSilentPush() ? .newData : .failed
    }
}

/// Nimmt den APNs-Token und stille Push-Nachrichten des Servers an. Eine
/// stille Push-Nachricht braucht keine Erlaubnis für Mitteilungen.
@MainActor
final class PushAppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        PushBridge.shared.tokenReceived(deviceToken)
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: any Error) {
        // Ohne Push-Berechtigung im Profil (Simulator, Build ohne Capability)
        // bleiben die Erinnerungen wie bisher beim Stand der letzten Öffnung.
    }

    func application(
        _ application: UIApplication,
        didReceiveRemoteNotification userInfo: [AnyHashable: Any]
    ) async -> UIBackgroundFetchResult {
        await PushBridge.shared.silentPush()
    }
}
