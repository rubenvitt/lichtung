// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import Combine
import SwiftUI
import UIKit

/// Schütteln zum Rückgängigmachen (LIC-42). SwiftUI hat dafür keinen eigenen
/// Modifier. Das Fenster sitzt am Ende der Responder-Kette und bekommt das
/// Schütteln auch dann, wenn gerade nichts den Fokus hat; es meldet es per
/// Notification an `onShake`.
extension Notification.Name {
    static let lichtungDeviceDidShake = Notification.Name("dev.rubeen.lichtung.deviceDidShake")
}

extension UIWindow {
    override open func motionEnded(_ motion: UIEvent.EventSubtype, with event: UIEvent?) {
        super.motionEnded(motion, with: event)
        if motion == .motionShake {
            NotificationCenter.default.post(name: .lichtungDeviceDidShake, object: nil)
        }
    }
}

private struct ShakeModifier: ViewModifier {
    let action: @MainActor () -> Void

    func body(content: Content) -> some View {
        content.onReceive(NotificationCenter.default.publisher(for: .lichtungDeviceDidShake)) { _ in
            action()
        }
    }
}

extension View {
    /// Wird beim Schütteln des Geräts aufgerufen.
    func onShake(perform action: @escaping @MainActor () -> Void) -> some View {
        modifier(ShakeModifier(action: action))
    }
}
