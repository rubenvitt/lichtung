import Foundation

// Texte und kleine Regeln der Einstellungen (LIC-43), hier statt in SwiftUI,
// damit sie unter Linux testbar sind.

extension Account.Kind {
    public var displayName: String {
        switch self {
        case .fastmail: "Fastmail"
        case .icloud: "iCloud"
        case .caldav: "CalDAV"
        case .ics: "Kalender-Abo (ICS)"
        case .microsoft: "Microsoft 365"
        case .unknown: "Konto"
        }
    }

    /// Hinweis unter dem Passwortfeld beim Anlegen.
    public var passwordHint: String? {
        switch self {
        case .fastmail: "Ein App-Passwort aus den Fastmail-Einstellungen (Datenschutz & Sicherheit → App-Passwörter), Zugriff „Kalender (CalDAV)“."
        case .icloud: "Ein app-spezifisches Passwort von account.apple.com (Anmeldung und Sicherheit)."
        case .caldav: "Passwort oder App-Passwort des CalDAV-Servers."
        case .ics: "Nur nötig, wenn der Feed eine Anmeldung verlangt."
        case .microsoft, .unknown: nil
        }
    }
}

/// Zustand eines Kontos in einer Zeile.
public struct AccountStatusSummary: Hashable, Sendable {
    public enum Level: Hashable, Sendable { case ok, busy, warning, error }

    public var level: Level
    public var text: String
    /// Braucht das Konto neue Zugangsdaten?
    public var needsCredentials: Bool

    public init(_ status: Account.Status?, now: Date = Date()) {
        guard let status else {
            self.init(level: .busy, text: "Noch nicht abgeglichen", needsCredentials: false)
            return
        }
        switch status.state {
        case .ok:
            let when = status.lastSyncAt.map { " · " + Self.relative($0, now: now) } ?? ""
            self.init(level: .ok, text: "Aktuell" + when, needsCredentials: false)
        case .syncing:
            self.init(level: .busy, text: "Wird abgeglichen …", needsCredentials: false)
        case .error:
            let detail = status.message.map { ": \($0)" } ?? ""
            self.init(level: .warning, text: "Abgleich gestört" + detail, needsCredentials: false)
        case .authFailed:
            self.init(level: .error, text: "Zugangsdaten abgelehnt. Bitte das Passwort erneuern.", needsCredentials: true)
        case .unknown:
            self.init(level: .warning, text: status.message ?? "Unbekannter Zustand", needsCredentials: false)
        }
    }

    public init(level: Level, text: String, needsCredentials: Bool) {
        self.level = level
        self.text = text
        self.needsCredentials = needsCredentials
    }

    /// „gerade eben“, „vor 5 Min.“, „vor 3 Std.“, „vor 2 Tagen“.
    public static func relative(_ date: Date, now: Date) -> String {
        let seconds = max(0, now.timeIntervalSince(date))
        let minutes = Int(seconds / 60)
        if minutes < 1 { return "gerade eben" }
        if minutes < 60 { return "vor \(minutes) Min." }
        let hours = minutes / 60
        if hours < 24 { return "vor \(hours) Std." }
        let days = hours / 24
        return days == 1 ? "vor einem Tag" : "vor \(days) Tagen"
    }
}

/// Schreibschutz für Claude (Spec §7): Standard für den Arbeitskalender ist
/// „nein“, und wer ihn aufhebt, soll es bewusst tun.
public enum MCPWriteGuard {
    private static let workWords = ["arbeit", "work", "büro", "buero", "office", "firma"]

    /// Konten von Microsoft 365 (Graph) sind der Arbeitskalender; sonst
    /// entscheidet der Name.
    public static func isWorkCalendar(_ calendar: CalendarInfo, account: Account?) -> Bool {
        if account?.kind == .microsoft { return true }
        let names = [calendar.name, account?.name ?? ""].map { $0.lowercased() }
        return names.contains { name in workWords.contains { name.contains($0) } }
    }

    public static let explanation = "Erlaubt Claude (MCP), in diesem Kalender Termine anzulegen, zu ändern, zu löschen und Einladungen zu beantworten. Ohne Erlaubnis kann Claude hier nur lesen."

    /// Rückfrage, bevor der Schutz aufgehoben wird, oder `nil`, wenn keine
    /// nötig ist (Schutz einschalten, privater Kalender).
    public static func confirmation(enabling: Bool, calendar: CalendarInfo, account: Account?) -> String? {
        guard enabling, isWorkCalendar(calendar, account: account) else { return nil }
        return "„\(calendar.name)“ sieht nach dem Arbeitskalender aus. Erlaubst du Claude das Schreiben, landen Fehlgriffe dort, wo andere sie sehen, und Antworten auf Einladungen gehen an echte Teilnehmende. Wirklich erlauben?"
    }
}

/// Farben zur Auswahl (`#RRGGBB`, wie die API sie erwartet).
public enum CalendarPalette {
    public struct Swatch: Hashable, Sendable, Identifiable {
        public var id: String { hex }
        public var hex: String
        public var name: String
    }

    public static let swatches: [Swatch] = [
        Swatch(hex: "#E5484D", name: "Rot"),
        Swatch(hex: "#F76B15", name: "Orange"),
        Swatch(hex: "#FFC53D", name: "Gelb"),
        Swatch(hex: "#46A758", name: "Grün"),
        Swatch(hex: "#12A594", name: "Türkis"),
        Swatch(hex: "#0090FF", name: "Blau"),
        Swatch(hex: "#6E56CF", name: "Violett"),
        Swatch(hex: "#D6409F", name: "Pink"),
        Swatch(hex: "#8D8D86", name: "Grau"),
        Swatch(hex: "#A18072", name: "Braun"),
    ]

    /// `#rrggbb`, `rrggbb`, ` #RRGGBB ` → `#RRGGBB`; sonst `nil`.
    public static func normalizedHex(_ text: String) -> String? {
        var t = text.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        if t.hasPrefix("#") { t.removeFirst() }
        guard t.count == 6, t.allSatisfy({ $0.isHexDigit }) else { return nil }
        return "#" + t
    }
}

extension APIError: LocalizedError {
    /// Text für die Oberfläche (deutsch, ohne Fachbegriffe, wo es geht).
    public var userMessage: String {
        switch self {
        case .unauthorized:
            return "Dieses Gerät ist nicht mehr angemeldet. Bitte neu anmelden."
        case .cursorExpired:
            return "Der lokale Stand war zu alt und wird neu geladen."
        case let .tooManyRequests(retry):
            if let retry, retry > 0 { return "Zu viele Anfragen. Bitte in \(Int(retry.rounded(.up))) s erneut versuchen." }
            return "Zu viele Anfragen. Bitte gleich noch einmal versuchen."
        case .transport:
            return "Keine Verbindung zum Server."
        case .decoding:
            return "Unerwartete Antwort vom Server."
        case let .server(status, code, message):
            switch code {
            case "invalid_credentials"?:
                return "Der Anbieter hat die Zugangsdaten abgelehnt. Fastmail und iCloud brauchen ein App-Passwort."
            case "invalid_feed"?:
                return "Unter dieser Adresse liegt kein lesbarer Kalender."
            case "provider_unavailable"?:
                return "Der Anbieter ist gerade nicht erreichbar. Bitte später noch einmal."
            case "conflict"?:
                return message ?? "Das gibt es schon."
            case "forbidden"?:
                return "Mit diesem Zugang nicht erlaubt."
            case "not_found"?:
                return "Nicht (mehr) vorhanden. Die Liste wird aktualisiert."
            default:
                if let message, !message.isEmpty { return message }
                return status >= 500 ? "Fehler auf dem Server (\(status))." : "Anfrage abgelehnt (\(status))."
            }
        }
    }

    public var errorDescription: String? { userMessage }
}
