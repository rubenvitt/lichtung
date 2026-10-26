import Foundation

/// iPhone: Halten auf einem ziehbaren Block (LIC-54). Nach dem kurzen Halten
/// ist der Block aufgenommen; wer ihn dann bewegt, verschiebt ihn, wer ohne
/// Bewegung loslässt, bekommt das Menü des Termins (Kopieren, Duplizieren …).
extension GridInteraction {
    /// Strecke in Punkten, die der Finger nach dem Aufnehmen höchstens
    /// wandern darf, damit Loslassen das Menü öffnet (wie die erlaubte
    /// Bewegung beim langen Drücken unter iOS).
    public static let holdMenuSlop: Double = 10

    /// Ob Loslassen nach dem Aufnehmen das Menü öffnet statt zu verschieben:
    /// Der Finger ist kaum gewandert, und der Termin würde sich nicht
    /// bewegen (unter einem halben Rasterschritt). `dy` enthält das
    /// automatische Scrollen am Rand; hat das Raster sich bewegt, war es ein Zug.
    public static func opensMenuOnRelease(dx: Double, dy: Double, columnHeight: Double) -> Bool {
        guard (dx * dx + dy * dy).squareRoot() < holdMenuSlop else { return false }
        return snapped(minutes(forDistance: dy, columnHeight: columnHeight)) == 0
    }
}
