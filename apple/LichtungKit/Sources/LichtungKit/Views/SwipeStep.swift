import Foundation

/// Blättern per Wisch im iPhone-Raster und -Monat (LIC-65): Nur ein Zug,
/// der klar in die eine Richtung geht und weit genug ist, blättert. Ein
/// senkrechter Zug im Tagesraster scrollt und wechselt nicht den Tag.
public enum SwipeStep {
    /// Mindeststrecke in Punkten.
    public static let minimumDistance: Double = 50
    /// Wie viel stärker die Hauptrichtung sein muss als die andere.
    public static let dominance: Double = 1.5

    /// Waagerecht: −1 = zurück (Zug nach rechts), +1 = vor (Zug nach links), sonst `nil`.
    public static func horizontal(dx: Double, dy: Double) -> Int? {
        guard abs(dx) >= minimumDistance, abs(dx) >= abs(dy) * dominance else { return nil }
        return dx < 0 ? 1 : -1
    }

    /// Senkrecht: −1 = zurück (Zug nach unten), +1 = vor (Zug nach oben), sonst `nil`.
    public static func vertical(dx: Double, dy: Double) -> Int? {
        horizontal(dx: dy, dy: dx)
    }
}
