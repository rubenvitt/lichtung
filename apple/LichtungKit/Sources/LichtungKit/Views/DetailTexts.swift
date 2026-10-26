import Foundation

// Kleine Texte für Detailansicht und Rückgängig-Knöpfe (LIC-42).

extension TravelTime {
    /// „30 Min. Reisezeit · Aufbruch 8:30“, oder `nil` ohne Reisezeit (und bei
    /// ganztägigen Terminen, siehe `travelBlock`).
    public static func detailText(for occ: Occurrence, calendar cal: Calendar) -> String? {
        guard let block = travelBlock(for: occ), let minutes = occ.details.travelTimeMinutes else { return nil }
        let departure = LocalDateTime(block.start, calendar: cal)
        return "\(label(minutes: minutes)) Reisezeit · Aufbruch \(TimeZoneDisplay.timeText(departure))"
    }
}

extension UndoHistory {
    /// „Rückgängig: Termin löschen“, ohne Eintrag nur „Rückgängig“.
    public var undoTitle: String { undoLabel.map { "Rückgängig: \($0)" } ?? "Rückgängig" }
    /// „Wiederholen: Termin löschen“, ohne Eintrag nur „Wiederholen“.
    public var redoTitle: String { redoLabel.map { "Wiederholen: \($0)" } ?? "Wiederholen" }
}
