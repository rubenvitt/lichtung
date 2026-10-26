import Foundation

/// Standarddauer neuer Termine (LIC-53): gilt für den Doppelklick ins Raster
/// (Mac), „Neuer Termin“ und die Schnelleingabe, wenn der Satz weder Ende
/// noch Dauer nennt. Aufziehen im Raster bestimmt die Länge selbst.
public enum NewEventDuration {
    /// Schlüssel in den `UserDefaults` (`@AppStorage`), Minuten; 0 = keine Wahl.
    public static let storageKey = "newEventMinutes"
    /// Wahl in den Einstellungen.
    public static let choices = [30, 45, 60, 90]
    public static let standard = 60

    /// Gespeicherter Wert, wenn er eine der Wahlen ist, sonst `standard`.
    public static func minutes(stored: Int) -> Int {
        choices.contains(stored) ? stored : standard
    }

    /// „30 Minuten“, „1 Stunde“, „1½ Stunden“.
    public static func label(_ minutes: Int) -> String {
        switch minutes {
        case 60: "1 Stunde"
        case 90: "1½ Stunden"
        case let m where m > 60 && m % 60 == 0: "\(m / 60) Stunden"
        default: "\(minutes) Minuten"
        }
    }
}

extension EventDraft {
    /// Derselbe Entwurf mit dem Ende `minutes` nach dem Beginn (Wanduhr in
    /// der Zone des Beginns). Ganztägige Entwürfe bleiben, wie sie sind.
    public func lasting(minutes: Int) -> EventDraft {
        guard !isAllDay else { return self }
        var draft = self
        draft.end = start.addingSeconds(max(1, minutes) * 60)
        return draft
    }
}
