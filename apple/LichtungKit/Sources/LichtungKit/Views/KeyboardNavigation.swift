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

    /// Titel der Menüpunkte für ⌘← und ⌘→ (LIC-50), z. B. „Nächste Woche“.
    public func stepTitle(forward: Bool) -> String {
        switch self {
        case .day: forward ? "Nächster Tag" : "Vorheriger Tag"
        case .week: forward ? "Nächste Woche" : "Vorherige Woche"
        case .month: forward ? "Nächster Monat" : "Vorheriger Monat"
        case .year: forward ? "Nächstes Jahr" : "Vorheriges Jahr"
        }
    }

    /// Ausgewählter Tag nach ← bzw. → im Kalenderfeld (LIC-50): in jeder
    /// Ansicht ein Tag. In Woche und Monat wandert so die Markierung des Tages
    /// von Spalte zu Spalte; geblättert wird erst, wenn der Tag den sichtbaren
    /// Bereich verlässt (`pages(from:to:)`), weil der Bereich dem Tag folgt.
    /// Die Pfeile bewegen also die Auswahl, ganze Seiten blättern ⌘← und ⌘→
    /// (`stepped`). So lässt sich jeder Tag der Woche ansteuern, etwa als Ziel
    /// für Einfügen (⌘V) und „Neuer Termin“ (⌘N).
    public func movedSelection(_ day: LocalDate, forward: Bool) -> LocalDate {
        day.adding(days: forward ? 1 : -1)
    }

    /// Ausgewählter Tag, nachdem die Markierung mit ↑/↓ zu `occ` gewandert ist
    /// (LIC-50): sein erster Tag, bei früher beginnenden der erste sichtbare.
    /// So setzen ←/→ dort an, wo die Markierung steht. Würde die Ansicht dabei
    /// blättern (Monat: Tag aus dem Nachbarmonat im Raster), bleibt `day`.
    public func selection(following occ: Occurrence, from day: LocalDate, calendar: Calendar) -> LocalDate {
        guard let span = visibleDays(around: day) else { return day }
        var target = LocalDate(occ.start, calendar: calendar)
        if target < span.start { target = span.start }
        if target >= span.end { target = span.end.adding(days: -1) }
        return pages(from: day, to: target) ? day : target
    }

    /// Ob die Ansicht beim Wechsel des ausgewählten Tags von `old` nach `new`
    /// blättert, also einen anderen Bereich zeigt.
    public func pages(from old: LocalDate, to new: LocalDate) -> Bool {
        switch self {
        case .year: old.year != new.year
        default: visibleDays(around: old) != visibleDays(around: new)
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

    /// Einstieg ohne Markierung (LIC-50): ↓ beginnt beim ersten Termin, der
    /// am ausgewählten Tag `day` liegt oder später endet, ↑ beim letzten, der
    /// spätestens an `day` beginnt. Gibt es dort keinen, der letzte bzw. erste
    /// der Liste. `nil` nur bei leerer Liste.
    public static func entry(in list: [Occurrence], forward: Bool, day: LocalDate, calendar: Calendar) -> Occurrence? {
        let sorted = ordered(list)
        let dayStart = day.startDate(in: calendar)
        let dayEnd = day.adding(days: 1).startDate(in: calendar)
        if forward {
            return sorted.first { $0.end > dayStart || $0.start >= dayStart } ?? sorted.last
        }
        return sorted.last { $0.start < dayEnd } ?? sorted.first
    }

    /// Markierung nach ←/→ (LIC-50), wenn der ausgewählte Tag zu `day` wechselt:
    /// Liegt `current` auch an `day` (mehrtägig), bleibt er markiert. Sonst
    /// der Termin an `day` mit der nächstliegenden Uhrzeit (bei gleichem
    /// Abstand der frühere), ein ganztägiger springt zum ersten ganztägigen.
    /// So wandert die Markierung in der Woche seitwärts wie in einer Tabelle.
    /// `nil`, wenn an `day` nichts liegt.
    public static func counterpart(of current: Occurrence, on day: LocalDate, in list: [Occurrence], calendar: Calendar) -> Occurrence? {
        let dayStart = day.startDate(in: calendar)
        let dayEnd = day.adding(days: 1).startDate(in: calendar)
        func touches(_ o: Occurrence) -> Bool {
            o.start < dayEnd && (o.end > dayStart || o.start >= dayStart)
        }
        let onDay = ordered(list).filter(touches)
        if touches(current) {
            return onDay.first { $0.id == current.id } ?? current
        }
        if current.isAllDay {
            return onDay.first(where: \.isAllDay) ?? onDay.first
        }
        let timed = onDay.filter { !$0.isAllDay }
        guard !timed.isEmpty else { return onDay.first }
        let target = LocalDateTime(current.start, calendar: calendar).secondsOfDay
        func distance(_ o: Occurrence) -> Int {
            // Begann der Termin am Vortag, zählt er ab Mitternacht.
            let start = max(o.start, dayStart)
            return abs(LocalDateTime(start, calendar: calendar).secondsOfDay - target)
        }
        // `min` liefert bei gleichem Abstand das erste Element, also den früheren.
        return timed.min { distance($0) < distance($1) }
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
    /// Ausgewählten Tag einen Tag zurück oder weiter (LIC-50, `movedSelection`).
    /// Ganze Seiten blättern ⌘←/⌘→ über das Menü.
    case previousDay, nextDay

    /// Belegung: ↓/Tab weiter, ↑/⇧Tab zurück, ←/→ Tag für Tag, Return/Leertaste
    /// öffnen, ⌫ löschen, Esc hebt die Markierung auf.
    public init?(key: CalendarKey, shift: Bool = false) {
        switch key {
        case .down: self = .highlightNext
        case .up: self = .highlightPrevious
        case .tab: self = shift ? .highlightPrevious : .highlightNext
        case .backTab: self = .highlightPrevious
        case .left: self = .previousDay
        case .right: self = .nextDay
        case .enter, .space: self = .open
        case .delete: self = .delete
        case .escape: self = .clearHighlight
        }
    }
}
