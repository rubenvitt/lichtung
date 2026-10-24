import Foundation

/// Terminsuche (Spec §1 Kriterium 3).
///
/// Sucht im lokalen Stand, also auch offline. Jedes Suchwort muss irgendwo
/// vorkommen (Titel, Ort, Notiz, Teilnehmende); Groß-/Kleinschreibung und
/// Akzente zählen nicht („cafe“ findet „Café“, „strasse“ findet „Straße“).
public struct EventSearch: Sendable {
    public struct Hit: Hashable, Sendable, Identifiable {
        public var event: Event
        /// Nächstes Vorkommen ab jetzt, sonst das letzte davor.
        public var occurrence: Occurrence?
        public var score: Int

        public var id: String { event.id }
    }

    public var expander: SeriesExpander
    /// Wie weit nach vorn und hinten Vorkommen von Serien gesucht werden.
    public var horizon: TimeInterval = 2 * 365 * 86_400

    public init(deviceTimeZone: TimeZone) {
        expander = SeriesExpander(deviceTimeZone: deviceTimeZone)
    }

    /// Treffer, sortiert: zuerst kommende Termine (nächster zuerst), dann
    /// vergangene (jüngster zuerst); bei gleicher Lage der bessere Treffer.
    public func search(_ query: String, in events: some Collection<Event>, now: Date, limit: Int = 100) -> [Hit] {
        let terms = Self.fold(query).split(whereSeparator: \.isWhitespace).map(String.init)
        guard !terms.isEmpty else { return [] }
        var hits: [Hit] = []
        for e in events {
            guard let score = Self.score(e, terms: terms) else { continue }
            hits.append(Hit(event: e, occurrence: anchor(of: e, now: now), score: score))
        }
        return hits.sorted { a, b in
            let ta = a.occurrence?.start ?? .distantPast
            let tb = b.occurrence?.start ?? .distantPast
            let fa = a.occurrence.map { $0.end > now } ?? false
            let fb = b.occurrence.map { $0.end > now } ?? false
            if fa != fb { return fa }
            if ta != tb { return fa ? ta < tb : ta > tb }
            if a.score != b.score { return a.score > b.score }
            return a.event.id < b.event.id
        }
        .prefix(limit)
        .map { $0 }
    }

    /// Vorkommen, das in der Trefferliste steht.
    func anchor(of e: Event, now: Date) -> Occurrence? {
        if !e.isRecurring {
            return expander.occurrences(of: e, in: DateInterval(start: .distantPast, end: .distantFuture)).first
        }
        if let next = expander.occurrences(of: e, in: DateInterval(start: now, duration: horizon)).first(where: { $0.end > now }) {
            return next
        }
        return expander.occurrences(of: e, in: DateInterval(start: now.addingTimeInterval(-horizon), end: now)).last
    }

    /// `nil`, wenn ein Wort nirgends vorkommt. Höher ist besser.
    static func score(_ e: Event, terms: [String]) -> Int? {
        let d = e.details
        let title = fold(d.title)
        let titleWords = title.split(whereSeparator: { !$0.isLetter && !$0.isNumber }).map(String.init)
        var fields: [String] = [d.location, d.notes, d.url].compactMap { $0 }
        for a in d.attendees { fields += [a.email, a.name ?? ""] }
        if let o = d.organizer { fields += [o.email, o.name ?? ""] }
        fields += e.overrides.map(\.details.title)
        fields = fields.map(fold)
        var total = 0
        for t in terms {
            if titleWords.contains(where: { $0.hasPrefix(t) }) { total += 10 }
            else if title.contains(t) { total += 6 }
            else if fields.contains(where: { $0.contains(t) }) { total += 2 }
            else { return nil }
        }
        if title == terms.joined(separator: " ") { total += 20 }
        return total
    }

    /// Kleinbuchstaben ohne Akzente; ß wird zu ss.
    static func fold(_ s: String) -> String {
        s.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "de_DE"))
            .lowercased()
            .replacingOccurrences(of: "ß", with: "ss")
    }
}

/// Termin duplizieren (Spec §1 Kriterium 3).
public enum EventDuplicate {
    /// Eingabe für eine Kopie von `occurrence`, optional um `days` Tage
    /// verschoben (Wanduhrzeit bleibt). Die Kopie ist ein Einzeltermin ohne
    /// UID. Teilnehmende kommen nur mit `includeAttendees` mit, weil sonst
    /// ungefragt Einladungen verschickt würden.
    public static func input(from occurrence: Occurrence, shiftedByDays days: Int = 0, includeAttendees: Bool = false) -> EventInput {
        var d = occurrence.details
        d.start = shift(d.start, days: days)
        d.end = shift(d.end, days: days)
        if !includeAttendees { d.attendees = [] }
        return EventInput(copying: d, recurrence: nil)
    }

    /// Kopie an einen neuen Beginn, Dauer bleibt (z. B. per Ziehen mit ⌥).
    public static func input(from occurrence: Occurrence, movedTo newStart: DateValue, deviceTimeZone tz: TimeZone, includeAttendees: Bool = false) -> EventInput {
        var d = occurrence.details
        d.end = OptimisticApply.shift(d.end, by: d.start, to: newStart, tz: tz)
        d.start = newStart
        if !includeAttendees { d.attendees = [] }
        return EventInput(copying: d, recurrence: nil)
    }

    static func shift(_ v: DateValue, days: Int) -> DateValue {
        switch v {
        case let .date(d): .date(d.adding(days: days))
        case let .dateTime(l, tz, _): .dateTime(l.adding(days: days), timeZone: tz)
        }
    }
}
