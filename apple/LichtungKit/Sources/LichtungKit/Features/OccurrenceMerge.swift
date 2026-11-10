import Foundation

/// Wann Termine in der Anzeige zu einem zusammenfallen (Spec §6).
public struct MergeRules: Hashable, Sendable, Codable {
    /// Gleicher Beginn, gleiches Ende, gleicher Titel in verschiedenen Kalendern.
    public var mergeIdentical = true
    /// Blocker („Blockiert“) verschwinden hinter einem Termin zur selben Zeit.
    public var hideBlockers = true
    public var blockerTitles = ["Blockiert"]

    public init(mergeIdentical: Bool = true, hideBlockers: Bool = true, blockerTitles: [String] = ["Blockiert"]) {
        self.mergeIdentical = mergeIdentical
        self.hideBlockers = hideBlockers
        self.blockerTitles = blockerTitles
    }

    public static let `default` = MergeRules()
    public static let off = MergeRules(mergeIdentical: false, hideBlockers: false, blockerTitles: [])
}

/// Ein angezeigter Termin samt den Terminen, die er verdeckt.
public struct MergedOccurrence: Hashable, Sendable, Identifiable {
    public var shown: Occurrence
    /// Zusammengefasste, in Kalenderreihenfolge.
    public var others: [Occurrence]
    public var id: String { shown.id }
    /// Ohne Doppelte, ohne den Kalender von `shown`.
    public var alsoInCalendarIds: [String] {
        var seen: Set<String> = [shown.calendarId]
        return others.map(\.calendarId).filter { seen.insert($0).inserted }
    }

    public init(shown: Occurrence, others: [Occurrence] = []) {
        self.shown = shown
        self.others = others
    }
}

public enum OccurrenceMerge {
    public static func merge(_ occurrences: [Occurrence], calendars: [String: CalendarInfo], rules: MergeRules) -> [MergedOccurrence] {
        let order = Dictionary(uniqueKeysWithValues: CalendarOrder.sorted(calendars.values).enumerated().map { ($1.id, $0) })
        func calendarRank(_ o: Occurrence) -> Int { order[o.calendarId] ?? Int.max }
        // Rangfolge für `shown`: offene Einladung, schreibbarer Kalender, Kalenderreihenfolge.
        func isBetter(_ a: Occurrence, _ b: Occurrence) -> Bool {
            let pa = a.details.selfStatus == .needsAction, pb = b.details.selfStatus == .needsAction
            if pa != pb { return pa }
            let wa = calendars[a.calendarId]?.writable ?? false, wb = calendars[b.calendarId]?.writable ?? false
            if wa != wb { return wa }
            return calendarRank(a) < calendarRank(b)
        }
        func norm(_ s: String) -> String { s.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }

        // Gruppen; Mitglieder jeweils nach Rangfolge, `shown` ist das erste.
        var groups: [[Occurrence]] = []
        if rules.mergeIdentical {
            struct Key: Hashable { var start: Date, end: Date, allDay: Bool, title: String }
            var byKey: [Key: [Int]] = [:]
            for o in occurrences.sorted(by: isBetter) {
                let key = Key(start: o.start, end: o.end, allDay: o.isAllDay, title: norm(o.title))
                // Nur verschiedene Kalender: zwei gleiche im selben Kalender bleiben getrennt.
                if let i = byKey[key]?.first(where: { g in !groups[g].contains { $0.calendarId == o.calendarId } }) {
                    groups[i].append(o)
                } else {
                    byKey[key, default: []].append(groups.count)
                    groups.append([o])
                }
            }
        } else {
            groups = occurrences.sorted(by: isBetter).map { [$0] }
        }

        if rules.hideBlockers {
            let titles = Set(rules.blockerTitles.map(norm).filter { !$0.isEmpty })
            func isBlocker(_ g: [Occurrence]) -> Bool {
                titles.contains(norm(g[0].title)) && !g.contains { $0.details.selfStatus == .needsAction }
            }
            let partners = groups.filter { !titles.contains(norm($0[0].title)) }
            var kept = partners
            for g in groups where titles.contains(norm(g[0].title)) {
                let calendarIds = Set(g.map(\.calendarId))
                if isBlocker(g), let p = kept.prefix(partners.count).firstIndex(where: { cand in
                    cand[0].start == g[0].start && cand[0].end == g[0].end && !cand.contains { calendarIds.contains($0.calendarId) }
                }) {
                    kept[p] += g
                } else {
                    kept.append(g)
                }
            }
            groups = kept
        }

        // Ausgabe in Eingabereihenfolge (Position von `shown`).
        let position = Dictionary(occurrences.enumerated().map { ($1.id, $0) }, uniquingKeysWith: { a, _ in a })
        return groups
            .map { g in
                MergedOccurrence(shown: g[0], others: g.dropFirst().sorted { calendarRank($0) < calendarRank($1) })
            }
            .sorted { position[$0.shown.id, default: 0] < position[$1.shown.id, default: 0] }
    }
}
