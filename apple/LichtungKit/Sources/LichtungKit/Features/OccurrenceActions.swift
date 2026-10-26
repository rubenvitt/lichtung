import Foundation

/// Aktionen an einem angezeigten Termin, wie Kontextmenü, Menü „Termin“ und
/// Tastatur sie anbieten (LIC-24): löschen, duplizieren, Kalender wechseln.
public enum OccurrenceActions {
    /// Vorkommen einer Serie (nicht der Einzeltermin).
    public static func isSeries(_ occ: Occurrence) -> Bool {
        occ.recurrenceId != nil
    }

    /// Geltungsbereiche, nach denen beim Löschen gefragt wird; leer heißt:
    /// ohne Rückfrage löschen (Rückgängig geht). Wie im Editor (`EventDraft.deleteScopes`).
    public static func deleteScopes(_ occ: Occurrence) -> [EditScope] {
        isSeries(occ) ? [.this, .thisAndFollowing, .all] : []
    }

    /// Löschen im Geltungsbereich `scope`; Einzeltermine immer ganz.
    public static func deleteIntent(_ occ: Occurrence, scope: EditScope = .all) -> EditIntent {
        guard let recurrenceId = occ.recurrenceId, scope != .all else {
            return .delete(eventId: occ.eventId)
        }
        return .delete(eventId: occ.eventId, scope: scope, occurrence: recurrenceId)
    }

    /// Kalender, in die sich der Termin verschieben lässt: beschreibbar,
    /// nicht ausgeblendet, nicht der eigene, in der Reihenfolge der Seitenleiste.
    /// Leer, wenn der eigene Kalender schreibgeschützt (oder unbekannt) ist.
    public static func moveTargets(for occ: Occurrence, calendars: [String: CalendarInfo]) -> [CalendarInfo] {
        guard calendars[occ.calendarId]?.writable == true else { return [] }
        return calendars.values
            .filter { $0.writable && !$0.hidden && $0.id != occ.calendarId }
            .sorted { ($0.position, $0.name) < ($1.position, $1.name) }
    }

    /// In einen anderen Kalender verschieben. Der Server erlaubt das nur für
    /// das ganze Event (Vertrag §6), bei Serien also für alle Termine.
    /// `nil`, wenn der Kalender schon der eigene ist.
    public static func moveIntent(_ occ: Occurrence, toCalendar calendarId: String) -> EditIntent? {
        guard calendarId != occ.calendarId else { return nil }
        return .update(eventId: occ.eventId, scope: .all, changes: EventChanges(calendarId: calendarId))
    }

    /// Kalender für eine Kopie: der eigene, wenn beschreibbar, sonst der erste
    /// beschreibbare (`writable` in der Reihenfolge der Seitenleiste).
    /// Ein Abo (ICS) lässt sich so trotzdem duplizieren.
    public static func duplicateCalendar(for occ: Occurrence, calendars: [String: CalendarInfo], writable: [CalendarInfo]) -> String? {
        if calendars[occ.calendarId]?.writable == true { return occ.calendarId }
        return writable.first?.id
    }

    /// Kopie um `days` Tage verschoben (0 = gleicher Tag) im Kalender `calendarId`.
    public static func duplicateIntent(_ occ: Occurrence, shiftedByDays days: Int = 0, calendarId: String) -> EditIntent {
        .create(calendarId: calendarId, event: EventDuplicate.input(from: occ, shiftedByDays: days))
    }
}

/// Kopieren und Einfügen eines Termins (⌘C/⌘V am Mac, LIC-24; iPhone über
/// Kontextmenü und Toolbar, LIC-50).
///
/// In die Zwischenablage des Systems geht ein lesbarer Text; eingefügt wird
/// aus dieser Kopie, solange die Zwischenablage noch genau diesen Text hält
/// (wer danach etwas anderes kopiert, fügt keinen alten Termin ein).
public struct EventClipboard: Hashable, Sendable {
    public var occurrence: Occurrence
    /// Text für die Zwischenablage, z. B. „Team-Sync\nFr 23.10.2026, 9:00–9:30“.
    public var text: String

    public init(_ occurrence: Occurrence, calendar: Calendar) {
        self.occurrence = occurrence
        self.text = Self.text(for: occurrence, calendar: calendar)
    }

    /// Ob `pasted` (Inhalt der Zwischenablage) noch von dieser Kopie stammt.
    public func matches(_ pasted: String?) -> Bool {
        pasted == text
    }

    /// Tage vom ersten Tag des Termins (in `calendar`, der Gerätezone) bis `day`.
    public func dayShift(to day: LocalDate, calendar: Calendar) -> Int {
        LocalDate(occurrence.start, calendar: calendar).days(until: day)
    }

    /// Einfügen am Tag `day`: gleiche Uhrzeit und Dauer, Einzeltermin ohne
    /// Teilnehmende (wie Duplizieren).
    public func intent(pastingOn day: LocalDate, calendarId: String, calendar: Calendar) -> EditIntent {
        OccurrenceActions.duplicateIntent(occurrence, shiftedByDays: dayShift(to: day, calendar: calendar), calendarId: calendarId)
    }

    /// Menüpunkt zum Einfügen am Tag `day`, z. B. „„Team-Sync“ einfügen am Di 27.10.“
    /// (iPhone, LIC-50).
    public func pasteTitle(on day: LocalDate) -> String {
        let weekday = QuickEntryPreview.weekdayShort[day.weekday.rawValue - 1]
        return "„\(occurrence.title)“ einfügen am \(weekday) \(day.day).\(day.month)."
    }

    static func text(for occ: Occurrence, calendar: Calendar) -> String {
        let day = LocalDate(occ.start, calendar: calendar)
        let weekday = QuickEntryPreview.weekdayShort[day.weekday.rawValue - 1]
        let date = "\(weekday) \(day.day).\(day.month).\(day.year)"
        let time: String
        if occ.isAllDay {
            let last = LocalDate(occ.end, calendar: calendar).adding(days: -1)
            time = last > day ? "ganztägig bis \(last.day).\(last.month).\(last.year)" : "ganztägig"
        } else {
            time = GridInteraction.timeText(DateInterval(start: occ.start, end: max(occ.start, occ.end)), calendar: calendar)
        }
        var lines = [occ.title, "\(date), \(time)"]
        if let location = occ.details.location, !location.isEmpty { lines.append(location) }
        return lines.joined(separator: "\n")
    }
}
