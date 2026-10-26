import Foundation

/// Reihenfolge der Kalender und Sets in den Einstellungen (LIC-43).
///
/// `position` ist eine Zahl je Kalender über alle Konten. Die Einstellungen
/// zeigen die Kalender nach Konto gruppiert; verschoben wird innerhalb eines
/// Kontos, die übrigen Kalender behalten ihre Plätze.
public enum CalendarOrder {
    /// Nach `position`, dann Name, dann ID (stabil, auch wenn alle 0 haben).
    public static func sorted(_ calendars: some Sequence<CalendarInfo>) -> [CalendarInfo] {
        calendars.sorted { a, b in
            if a.position != b.position { return a.position < b.position }
            let byName = a.name.localizedStandardCompare(b.name)
            if byName != .orderedSame { return byName == .orderedAscending }
            return a.id < b.id
        }
    }

    public static func sortedSets(_ sets: some Sequence<CalendarSet>) -> [CalendarSet] {
        sets.sorted { a, b in
            if a.position != b.position { return a.position < b.position }
            let byName = a.name.localizedStandardCompare(b.name)
            if byName != .orderedSame { return byName == .orderedAscending }
            return a.id < b.id
        }
    }

    /// Ein Konto mit seinen Kalendern.
    public struct Section: Hashable, Sendable, Identifiable {
        public var id: String { accountId }
        public var accountId: String
        public var account: Account?
        public var calendars: [CalendarInfo]

        public var title: String { account?.name ?? "Weitere Kalender" }
    }

    /// Konten in der Reihenfolge ihres ersten Kalenders, Konten ohne Kalender
    /// am Ende (nach Name).
    public static func sections(calendars: [String: CalendarInfo], accounts: [String: Account]) -> [Section] {
        var byAccount: [String: [CalendarInfo]] = [:]
        for c in sorted(calendars.values) { byAccount[c.accountId, default: []].append(c) }
        var result = byAccount.map { Section(accountId: $0.key, account: accounts[$0.key], calendars: $0.value) }
        result.sort { a, b in
            let pa = a.calendars.first?.position ?? .max
            let pb = b.calendars.first?.position ?? .max
            if pa != pb { return pa < pb }
            return a.title.localizedStandardCompare(b.title) == .orderedAscending
        }
        let empty = accounts.values
            .filter { byAccount[$0.id] == nil }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            .map { Section(accountId: $0.id, account: $0, calendars: []) }
        return result + empty
    }

    /// Wie `move(fromOffsets:toOffset:)` in SwiftUI: `toOffset` zählt in der
    /// Liste vor dem Verschieben.
    public static func moved<T>(_ items: [T], fromOffsets offsets: IndexSet, toOffset: Int) -> [T] {
        let valid = offsets.filter { $0 >= 0 && $0 < items.count }
        guard !valid.isEmpty else { return items }
        let moving = valid.map { items[$0] }
        var rest: [T] = []
        for (i, item) in items.enumerated() where !valid.contains(i) { rest.append(item) }
        let target = min(max(0, toOffset), items.count)
        let insertAt = target - valid.filter { $0 < target }.count
        rest.insert(contentsOf: moving, at: insertAt)
        return rest
    }

    /// Neue Positionen nach dem Verschieben innerhalb von `section` (IDs in
    /// angezeigter Reihenfolge). Die Kalender des Abschnitts tauschen nur
    /// untereinander ihre Plätze in der Gesamtliste. Ergebnis: nur die
    /// Kalender, deren `position` sich ändert (je ein `PATCH`).
    public static func positionsAfterMove(section: [String], fromOffsets: IndexSet, toOffset: Int, all: [CalendarInfo]) -> [String: Int] {
        let global = sorted(all).map(\.id)
        let members = Set(section)
        let reordered = moved(section, fromOffsets: fromOffsets, toOffset: toOffset)
        var next = reordered.makeIterator()
        let newOrder = global.map { id in members.contains(id) ? (next.next() ?? id) : id }
        let current = Dictionary(all.map { ($0.id, $0.position) }, uniquingKeysWith: { a, _ in a })
        return changes(order: newOrder, current: current)
    }

    /// Dasselbe für Kalender-Sets (eine Liste).
    public static func setPositionsAfterMove(_ sets: [CalendarSet], fromOffsets: IndexSet, toOffset: Int) -> [String: Int] {
        let order = moved(sortedSets(sets).map(\.id), fromOffsets: fromOffsets, toOffset: toOffset)
        let current = Dictionary(sets.map { ($0.id, $0.position) }, uniquingKeysWith: { a, _ in a })
        return changes(order: order, current: current)
    }

    /// Position = Platz in `order`; nur Abweichungen.
    public static func changes(order: [String], current: [String: Int]) -> [String: Int] {
        var result: [String: Int] = [:]
        for (i, id) in order.enumerated() where current[id] != i { result[id] = i }
        return result
    }

    /// Position für ein neues Set: hinter allen bestehenden.
    public static func nextSetPosition(_ sets: some Sequence<CalendarSet>) -> Int {
        (sets.map(\.position).max() ?? -1) + 1
    }
}
