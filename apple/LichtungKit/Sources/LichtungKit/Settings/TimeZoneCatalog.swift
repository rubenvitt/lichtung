import Foundation

/// Auswahl der zweiten Zeitzone in den Einstellungen (LIC-43).
///
/// Die Einträge kommen aus den IANA-Zonen des Systems, benannt nach Stadt
/// (`TimeZoneDisplay.cityName`), mit Abstand zur Heimatzone.
public enum TimeZoneCatalog {
    public struct Choice: Hashable, Sendable, Identifiable {
        public var id: String
        /// „New York“, „Wien“.
        public var city: String
        /// „America“, „Europe“ …
        public var region: String
        /// „−6 Std.“ zur Heimatzone, zum Zeitpunkt der Abfrage.
        public var offset: String

        public var title: String { "\(city) (\(offset))" }
    }

    /// Zonen außer `Etc/…` und Aliasen ohne Region, sortiert nach Stadt.
    public static func choices(home: TimeZone, at now: Date,
                               identifiers: [String] = TimeZone.knownTimeZoneIdentifiers) -> [Choice] {
        identifiers
            .filter { $0.contains("/") && !$0.hasPrefix("Etc/") && !$0.hasPrefix("SystemV/") }
            .compactMap { id in
                guard let zone = TimeZone(identifier: id) else { return nil }
                return Choice(
                    id: id,
                    city: TimeZoneDisplay.cityName(for: zone),
                    region: String(id.split(separator: "/").first ?? ""),
                    offset: TimeZoneDisplay.offsetLabel(zone, relativeTo: home, at: now)
                )
            }
            .sorted { a, b in
                let c = a.city.localizedStandardCompare(b.city)
                return c == .orderedSame ? a.id < b.id : c == .orderedAscending
            }
    }

    /// Suche ohne Rücksicht auf Groß-/Kleinschreibung und Akzente, in Stadt
    /// und IANA-ID („york“, „wien“, „europe/vienna“).
    public static func filter(_ choices: [Choice], query: String) -> [Choice] {
        let q = fold(query.trimmingCharacters(in: .whitespacesAndNewlines))
        guard !q.isEmpty else { return choices }
        return choices.filter { fold($0.city).contains(q) || fold($0.id).replacingOccurrences(of: "_", with: " ").contains(q) || fold($0.id).contains(q) }
    }

    /// Gespeicherter Wert (`@AppStorage`): IANA-ID oder leer für „keine“.
    public static func zone(fromStored text: String) -> TimeZone? {
        let t = text.trimmingCharacters(in: .whitespaces)
        guard !t.isEmpty else { return nil }
        return TimeZone(identifier: t)
    }

    private static func fold(_ s: String) -> String {
        s.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "de_DE"))
    }
}
