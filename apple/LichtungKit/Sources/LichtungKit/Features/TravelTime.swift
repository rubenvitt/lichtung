import Foundation

/// Reisezeit vor einem Termin (Spec §1, Kriterium 3). Sie liegt direkt vor dem
/// Beginn, zählt als belegt und wird im Raster als eigener Block gezeichnet.
///
/// Gerechnet wird in echter Zeit, nicht in Wanduhrzeit: 90 Minuten vor 3:00
/// am Tag der Zeitumstellung im Herbst sind 2:30 Sommerzeit.
public enum TravelTime {
    /// Auswahl im Termin-Editor, in Minuten.
    public static let presets: [Int] = [5, 15, 30, 60, 90, 120]

    /// `[Beginn − Reisezeit, Beginn)`, oder `nil` ohne Reisezeit und bei
    /// ganztägigen Terminen.
    public static func travelBlock(for occ: Occurrence) -> DateInterval? {
        guard let minutes = occ.details.travelTimeMinutes, minutes > 0, !occ.isAllDay else { return nil }
        return DateInterval(start: occ.start.addingTimeInterval(-Double(minutes) * 60), end: occ.start)
    }

    /// Belegte Zeit inklusive Reisezeit, für die Prüfung freier Zeiten und das Raster.
    public static func busyInterval(for occ: Occurrence) -> DateInterval {
        let start = travelBlock(for: occ)?.start ?? occ.start
        return DateInterval(start: start, end: max(occ.end, occ.start))
    }

    /// „15 Min.“, „1 Std.“, „1,5 Std.“, „1 Std. 15 Min.“.
    public static func label(minutes: Int) -> String {
        guard minutes >= 60 else { return "\(minutes) Min." }
        let rem = minutes % 60
        if rem == 0 || rem == 30 { return TimeZoneDisplay.hoursText(minutes) + " Std." }
        return "\(minutes / 60) Std. \(rem) Min."
    }
}
