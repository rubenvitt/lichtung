import Foundation

/// Kalender für neue Termine (LIC-47): der in den Einstellungen gewählte,
/// solange er beschreibbar und sichtbar ist, sonst der erste beschreibbare.
public enum DefaultCalendar {
    /// Schlüssel in den `UserDefaults` (`@AppStorage`), leer = keine Wahl.
    public static let storageKey = "defaultCalendarID"

    /// `writable` in der Reihenfolge der Seitenleiste (beschreibbar, nicht
    /// ausgeblendet). `nil`, wenn es keinen beschreibbaren Kalender gibt.
    public static func choose(stored: String?, writable: [CalendarInfo]) -> String? {
        if let stored, !stored.isEmpty, writable.contains(where: { $0.id == stored }) { return stored }
        return writable.first?.id
    }

    /// Hinweis in den Einstellungen, wenn die Wahl nicht (mehr) greift;
    /// `nil`, wenn alles stimmt oder nichts gewählt ist.
    public static func fallbackNote(stored: String?, writable: [CalendarInfo]) -> String? {
        guard let stored, !stored.isEmpty, !writable.contains(where: { $0.id == stored }) else { return nil }
        guard let first = writable.first else { return "Der gewählte Kalender ist nicht mehr beschreibbar." }
        return "Der gewählte Kalender ist nicht mehr beschreibbar. Neue Termine landen in „\(first.name)“."
    }
}
