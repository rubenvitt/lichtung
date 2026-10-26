import Foundation

/// Eine zusätzliche Uhr im Raster und in der Liste (Spec §1, Kriterium 3:
/// „mehrere Zeitzonen“).
public struct ZoneClock: Hashable, Sendable {
    public var timeZone: TimeZone
    /// Kurzname über der Spalte, z. B. „NYC“ aus den Einstellungen.
    public var label: String

    /// Ohne eigenen Namen gilt der Stadtname aus der IANA-ID.
    public init(timeZone: TimeZone, label: String? = nil) {
        self.timeZone = timeZone
        let trimmed = label?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        self.label = trimmed.isEmpty ? TimeZoneDisplay.cityName(for: timeZone) : trimmed
    }
}

/// Anzeige-Logik für fremde Zeitzonen. Formatiert selbst, ohne
/// `DateFormatter`: gleiche Eingabe, gleiche Ausgabe, unabhängig von der
/// Locale des Geräts. Uhrzeiten deutsch ohne führende Null („8:00“), außer in
/// der Stundenleiste („08“).
public enum TimeZoneDisplay {
    /// Deutsche Namen für Städte, deren IANA-Name englisch ist.
    private static let germanCityNames: [String: String] = [
        "Vienna": "Wien", "Zurich": "Zürich", "Rome": "Rom", "Moscow": "Moskau",
        "Copenhagen": "Kopenhagen", "Brussels": "Brüssel", "Prague": "Prag",
        "Warsaw": "Warschau", "Lisbon": "Lissabon", "Athens": "Athen",
        "Bucharest": "Bukarest", "Belgrade": "Belgrad", "Kiev": "Kiew", "Kyiv": "Kiew",
        "Mexico_City": "Mexiko-Stadt", "Sao_Paulo": "São Paulo",
    ]

    /// „America/New_York“ → „New York“, „Europe/Vienna“ → „Wien“, „UTC“ → „UTC“.
    public static func cityName(for zone: TimeZone) -> String {
        let id = zone.identifier
        // Foundation meldet `UTC` je nach Plattform als `GMT`.
        if ["UTC", "GMT", "Etc/UTC", "Etc/GMT", "Etc/Universal", "Universal", "Zulu"].contains(id) { return "UTC" }
        guard !id.hasPrefix("Etc/"), let last = id.split(separator: "/").last.map(String.init), id.contains("/") else { return id }
        return germanCityNames[last] ?? last.replacingOccurrences(of: "_", with: " ")
    }

    /// Abstand von `zone` zu `home` zum Zeitpunkt `at`: „−6 Std.“, „+5,5 Std.“,
    /// „+3,75 Std.“, „±0“. Der Abstand hängt von der Sommerzeit beider Zonen ab
    /// (Berlin–New York ist Ende Oktober eine Woche lang −5 statt −6).
    public static func offsetLabel(_ zone: TimeZone, relativeTo home: TimeZone, at: Date) -> String {
        let diff = zone.secondsFromGMT(for: at) - home.secondsFromGMT(for: at)
        guard diff != 0 else { return "±0" }
        let minutes = abs(diff) / 60
        return (diff < 0 ? "−" : "+") + hoursText(minutes) + " Std."
    }

    /// Je Stunde des Heimat-Tages die Uhrzeit in `zone`: „03“, bei halben
    /// Abständen „03:30“.
    ///
    /// Eintrag `i` gehört zur `i`-ten echten Stunde ab Mitternacht (Heimatzone).
    /// An Umstellungstagen hat der Heimat-Tag 23 oder 25 Stunden, die Liste hat
    /// dann ebenso viele Einträge. Das Raster hat immer 24 Wanduhr-Zeilen
    /// (LIC-41); `TimelineLayout.secondaryHourLabels` ordnet die Einträge
    /// diesen Zeilen zu.
    public static func hourLabels(for zone: TimeZone, home: TimeZone, day: LocalDate) -> [String] {
        let homeCal = Calendar.lichtung(timeZone: home)
        let zoneCal = Calendar.lichtung(timeZone: zone)
        let start = day.startDate(in: homeCal)
        let end = day.adding(days: 1).startDate(in: homeCal)
        var labels: [String] = []
        var t = start
        while t < end {
            let c = zoneCal.dateComponents([.hour, .minute], from: t)
            let h = String(format: "%02d", c.hour!)
            labels.append(c.minute! == 0 ? h : h + String(format: ":%02d", c.minute!))
            t = t.addingTimeInterval(3600)
        }
        return labels
    }

    /// „14:00 (8:00 New York)“ oder „3:00 (21:00 New York, Vortag)“.
    /// Hat `zone` gerade denselben Abstand wie `home`, nur die Heimat-Uhrzeit.
    public static func describe(_ instant: Date, in zone: TimeZone, home: TimeZone) -> String {
        describe(instant, in: ZoneClock(timeZone: zone), home: home)
    }

    /// Wie oben, mit dem eingestellten Kurznamen der Uhr.
    public static func describe(_ instant: Date, in clock: ZoneClock, home: TimeZone) -> String {
        let homeTime = LocalDateTime(instant, calendar: .lichtung(timeZone: home))
        let homeText = timeText(homeTime)
        guard clock.timeZone.secondsFromGMT(for: instant) != home.secondsFromGMT(for: instant) else { return homeText }
        let other = LocalDateTime(instant, calendar: .lichtung(timeZone: clock.timeZone))
        var inner = "\(timeText(other)) \(clock.label)"
        switch homeTime.date.days(until: other.date) {
        case ..<0: inner += ", Vortag"
        case 1...: inner += ", Folgetag"
        default: break
        }
        return "\(homeText) (\(inner))"
    }

    /// Hinweis für Termine, die in einer anderen Zone angelegt wurden:
    /// „9:00 in Los Angeles“ (Wanduhrzeit des Originals). `nil` bei
    /// ganztägigen und floating Terminen und wenn die Zone zum Zeitpunkt des
    /// Termins denselben Abstand wie `home` hat (Paris bei Heimat Berlin).
    public static func originalZoneHint(for details: EventDetails, home: TimeZone) -> String? {
        guard case let .dateTime(local, tzID?, _) = details.start,
              let zone = TimeZone(identifier: tzID), zone.identifier != home.identifier
        else { return nil }
        let instant = details.start.instant(deviceTimeZone: home)
        guard zone.secondsFromGMT(for: instant) != home.secondsFromGMT(for: instant) else { return nil }
        return "\(timeText(local)) in \(cityName(for: zone))"
    }

    // MARK: - Formatierung

    static func timeText(_ t: LocalDateTime) -> String {
        "\(t.hour):" + String(format: "%02d", t.minute)
    }

    /// Minuten als Stunden mit Dezimalkomma: 360 → „6“, 330 → „5,5“, 225 → „3,75“.
    static func hoursText(_ minutes: Int) -> String {
        let h = minutes / 60, rem = minutes % 60
        guard rem != 0 else { return "\(h)" }
        var frac = String(format: "%02d", rem * 100 / 60)
        while frac.hasSuffix("0") { frac.removeLast() }
        return "\(h),\(frac)"
    }
}
