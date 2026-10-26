import Foundation

/// Ansicht im Mac-Hauptfenster (Spec §5, C): Tag, Woche, Monat, Jahr.
/// Umschalten mit ⌘1–⌘4 (LIC-24).
public enum CalendarViewMode: String, CaseIterable, Identifiable, Hashable, Sendable {
    case day, week, month, year

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .day: "Tag"
        case .week: "Woche"
        case .month: "Monat"
        case .year: "Jahr"
        }
    }

    /// Ziffer des Tastenkürzels (⌘1 = Tag … ⌘4 = Jahr).
    public var shortcutDigit: Character {
        switch self {
        case .day: "1"
        case .week: "2"
        case .month: "3"
        case .year: "4"
        }
    }

    /// Ausgewählter Tag nach `steps` Schritten vor (positiv) oder zurück:
    /// ein Tag, eine Woche (gleicher Wochentag), ein Monat bzw. ein Jahr
    /// (jeweils der Erste).
    public func stepped(_ day: LocalDate, by steps: Int) -> LocalDate {
        switch self {
        case .day: day.adding(days: steps)
        case .week: day.adding(days: 7 * steps)
        case .month: LocalDate(year: day.year, month: day.month, day: 1).adding(months: steps)
        case .year: LocalDate(year: day.year + steps, month: day.month, day: 1)
        }
    }

    /// Tage, die die Ansicht um `day` zeigt. Das Jahr zeigt keine Termine,
    /// dort gibt es nichts zu markieren (`nil`).
    public func visibleDays(around day: LocalDate) -> DaySpan? {
        switch self {
        case .day:
            return DaySpan(start: day, count: 1)
        case .week:
            return DaySpan(start: day.adding(days: -(day.weekday.rawValue - 1)), count: 7)
        case .month:
            let weeks = MonthGrid.weeks(year: day.year, month: day.month, fixedRows: true)
            guard let first = weeks.first?.first else { return nil }
            return DaySpan(start: first, count: weeks.count * 7)
        case .year:
            return nil
        }
    }
}

/// Zusammenhängende Tage, z. B. der sichtbare Bereich einer Ansicht.
public struct DaySpan: Hashable, Sendable {
    public var start: LocalDate
    public var count: Int

    public init(start: LocalDate, count: Int) {
        self.start = start
        self.count = count
    }

    public var end: LocalDate { start.adding(days: count) }

    /// Zeitraum in `calendar` (Ende exklusiv).
    public func interval(in calendar: Calendar) -> DateInterval {
        DateInterval(start: start.startDate(in: calendar), end: end.startDate(in: calendar))
    }

    public func contains(_ day: LocalDate) -> Bool { start <= day && day < end }
}

/// Markierung im Raster und in der Liste per Tastatur (LIC-24): Tab/⇧Tab
/// oder ↓/↑ wandern zum nächsten bzw. vorherigen Termin, ohne das Detail zu öffnen.
public enum OccurrenceNavigation {
    /// Reihenfolge beim Wandern: nach Beginn, ganztägige vor Terminen mit
    /// Uhrzeit, dann nach Ende, Titel und ID (stabil).
    public static func ordered(_ list: [Occurrence]) -> [Occurrence] {
        list.sorted(by: precedes)
    }

    static func precedes(_ a: Occurrence, _ b: Occurrence) -> Bool {
        if a.start != b.start { return a.start < b.start }
        if a.isAllDay != b.isAllDay { return a.isAllDay }
        if a.end != b.end { return a.end < b.end }
        if a.title != b.title { return a.title < b.title }
        return a.id < b.id
    }

    /// Nachbar von `current` in `list` (beliebig sortiert). Ohne Markierung
    /// der erste bzw. letzte Termin. Am Ende bleibt es beim letzten bzw.
    /// ersten (kein Umlauf). Liegt `current` nicht in der Liste (z. B. in der
    /// Seitenleiste markiert), zählt sein Platz in der Reihenfolge.
    /// `nil` nur bei leerer Liste.
    public static func neighbor(of current: Occurrence?, in list: [Occurrence], forward: Bool) -> Occurrence? {
        let sorted = ordered(list)
        guard let first = sorted.first, let last = sorted.last else { return nil }
        guard let current else { return forward ? first : last }
        if let index = sorted.firstIndex(where: { $0.id == current.id }) {
            let next = forward ? min(sorted.count - 1, index + 1) : max(0, index - 1)
            return sorted[next]
        }
        if forward {
            return sorted.first { precedes(current, $0) } ?? last
        }
        return sorted.last { precedes($0, current) } ?? first
    }

    /// Frischer Stand einer Markierung nach einer Änderung (gleiche ID);
    /// `nil`, wenn der Termin nicht mehr da ist.
    public static func refreshed(_ old: Occurrence, in list: [Occurrence]) -> Occurrence? {
        list.first { $0.id == old.id }
    }
}

/// Tasten, die das Kalenderfeld ohne ⌘ auswertet, solange es den Fokus hat
/// (also kein Textfeld). Die Oberfläche übersetzt ihre Tastencodes hierher.
public enum CalendarKey: Hashable, Sendable {
    case up, down, left, right, tab, backTab, enter, space, escape, delete
}

/// Was eine Taste im Kalenderfeld auslöst (LIC-24).
public enum CalendarKeyCommand: Hashable, Sendable {
    /// Markierung zum nächsten bzw. vorherigen Termin.
    case highlightNext, highlightPrevious
    /// Markierten Termin öffnen (Detail).
    case open
    /// Markierten Termin löschen (bei Serien mit Rückfrage).
    case delete
    /// Markierung aufheben.
    case clearHighlight
    /// Einen Tag bzw. eine Woche (je nach Ansicht) zurück oder weiter.
    case stepBackward, stepForward

    /// Belegung: ↓/Tab weiter, ↑/⇧Tab zurück, ←/→ blättern, Return/Leertaste
    /// öffnen, ⌫ löschen, Esc hebt die Markierung auf.
    public init?(key: CalendarKey, shift: Bool = false) {
        switch key {
        case .down: self = .highlightNext
        case .up: self = .highlightPrevious
        case .tab: self = shift ? .highlightPrevious : .highlightNext
        case .backTab: self = .highlightPrevious
        case .left: self = .stepBackward
        case .right: self = .stepForward
        case .enter, .space: self = .open
        case .delete: self = .delete
        case .escape: self = .clearHighlight
        }
    }
}
