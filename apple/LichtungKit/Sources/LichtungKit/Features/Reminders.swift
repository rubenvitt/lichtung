import Foundation

/// Eine Erinnerung, die die App als lokale Benachrichtigung einplant (LIC-68).
public struct PlannedReminder: Hashable, Sendable, Identifiable {
    /// Stabil über Neuberechnungen: Vorkommen plus Erinnerung. Die App plant
    /// damit nur die Differenz neu ein.
    public var id: String
    public var fireDate: Date
    public var title: String
    /// „14:00–15:00, Raum 2“, „Morgen, 9:00–9:30“, „Ganztägig“.
    public var body: String
    public var eventId: String
    public var occurrenceId: String

    public init(id: String, fireDate: Date, title: String, body: String, eventId: String, occurrenceId: String) {
        self.id = id
        self.fireDate = fireDate
        self.title = title
        self.body = body
        self.eventId = eventId
        self.occurrenceId = occurrenceId
    }
}

/// Rechnet aus dem lokalen Stand die Erinnerungen der nächsten Tage aus
/// (VALARM bzw. Graph-Erinnerungen, Spec §5). Reine Werte, ohne
/// UserNotifications, damit es unter Linux testbar bleibt.
///
/// Regeln:
/// - nur sichtbare Kalender (ausgeblendete erinnern nicht)
/// - keine abgesagten Termine, keine abgelehnten Einladungen
/// - relative Erinnerungen zählen ab Beginn (ganztägig: Mitternacht in der
///   Gerätezone), feste Zeitpunkte gelten einmal pro Termin
/// - nur Zeitpunkte ab `now` bis `now + horizon`, früheste zuerst,
///   höchstens `limit` (iOS behält nur 64 ausstehende Benachrichtigungen)
public enum ReminderPlanner {
    public static let defaultHorizon: TimeInterval = 7 * 24 * 3600
    public static let defaultLimit = 64
    /// Wie weit eine relative Erinnerung höchstens vor dem Beginn liegt, damit
    /// das Fenster der Serienexpansion begrenzt bleibt.
    static let maxLead: TimeInterval = 30 * 24 * 3600

    public static func plan(
        _ query: CalendarQuery,
        now: Date,
        horizon: TimeInterval = defaultHorizon,
        limit: Int = defaultLimit,
        calendar: Calendar,
        merge: MergeRules = .off
    ) -> [PlannedReminder] {
        let end = now.addingTimeInterval(horizon)
        // Relative Erinnerungen liegen vor dem Beginn (oder kurz danach, bei
        // negativen Minuten): Vorkommen bis `maxLead` nach dem Horizont und ab
        // einem Tag vor jetzt reichen.
        let window = DateInterval(start: now.addingTimeInterval(-24 * 3600), end: end.addingTimeInterval(maxLead))
        let today = LocalDate(now, calendar: calendar)
        var result: [PlannedReminder] = []
        // Fester Zeitpunkt: gilt für den Termin, nicht für jedes Vorkommen einer
        // Serie. Beschrieben wird das erste Vorkommen, das dann noch nicht vorbei ist.
        var fixed: [String: (Date, Occurrence)] = [:]

        func reminder(_ key: String, _ fire: Date, _ occ: Occurrence) -> PlannedReminder {
            PlannedReminder(
                id: key,
                fireDate: fire,
                title: occ.title.isEmpty ? "Termin" : occ.title,
                body: body(for: occ, firingOn: LocalDate(fire, calendar: calendar), today: today, calendar: calendar),
                eventId: occ.eventId,
                occurrenceId: occ.id
            )
        }

        let occurrences = query.occurrences(in: window)
        for occ in occurrences where remindable(occ.details) {
            for alarm in occ.details.alarms {
                switch alarm {
                case let .minutesBefore(m):
                    guard Double(m) * 60 <= maxLead else { continue }
                    let fire = occ.start.addingTimeInterval(-Double(m) * 60)
                    guard fire >= now, fire < end else { continue }
                    result.append(reminder("\(occ.id)#\(m)", fire, occ))
                case let .at(fire):
                    guard fire >= now, fire < end else { continue }
                    let key = "\(occ.eventId)#at\(Int(fire.timeIntervalSince1970))"
                    if let (_, current) = fixed[key], !describesBetter(occ, than: current, at: fire) { continue }
                    fixed[key] = (fire, occ)
                }
            }
        }
        result += fixed.map { key, value in reminder(key, value.0, value.1) }
        // Mehrere Erinnerungen mit gleichem Schlüssel (doppelte VALARMs) nur einmal.
        var seen = Set<String>()
        let sorted = result
            .sorted { $0.fireDate != $1.fireDate ? $0.fireDate < $1.fireDate : $0.id < $1.id }
            .filter { seen.insert($0.id).inserted }
        return Array(collapse(sorted, occurrences: occurrences, query: query, rules: merge, today: today, calendar: calendar).prefix(max(0, limit)))
    }

    /// Zusammengefasste Termine erinnern einmal pro Zeitpunkt. Die übrig
    /// bleibende Erinnerung trägt Titel, Text und Kennungen des sichtbaren Termins.
    static func collapse(_ reminders: [PlannedReminder], occurrences: [Occurrence], query: CalendarQuery, rules: MergeRules, today: LocalDate, calendar cal: Calendar) -> [PlannedReminder] {
        guard rules != .off else { return reminders }
        let groups = OccurrenceMerge.merge(occurrences, calendars: query.calendars, rules: rules).filter { !$0.others.isEmpty }
        var shownOf: [String: Occurrence] = [:]
        for g in groups { for o in g.others { shownOf[o.id] = g.shown } }
        // Zeitpunkte, zu denen der sichtbare Termin selbst schon erinnert.
        var covered = Set(reminders.filter { shownOf[$0.occurrenceId] == nil }.map { "\($0.occurrenceId)|\($0.fireDate.timeIntervalSince1970)" })
        return reminders.compactMap { r in
            guard let shown = shownOf[r.occurrenceId] else { return r }
            guard covered.insert("\(shown.id)|\(r.fireDate.timeIntervalSince1970)").inserted else { return nil }
            var r = r
            r.title = shown.title.isEmpty ? "Termin" : shown.title
            r.occurrenceId = shown.id
            r.eventId = shown.eventId
            r.body = body(for: shown, firingOn: LocalDate(r.fireDate, calendar: cal), today: today, calendar: cal)
            return r
        }
    }

    /// Für einen festen Zeitpunkt: lieber ein Vorkommen, das dann noch nicht
    /// vorbei ist, davon das früheste; sonst das letzte vergangene.
    static func describesBetter(_ a: Occurrence, than b: Occurrence, at fire: Date) -> Bool {
        let aOpen = a.end > fire, bOpen = b.end > fire
        if aOpen != bOpen { return aOpen }
        return aOpen ? a.start < b.start : a.start > b.start
    }

    static func remindable(_ d: EventDetails) -> Bool {
        d.status != .cancelled && d.selfStatus != .declined
    }

    /// Text unter dem Titel. Bezieht sich auf den Tag, an dem die
    /// Benachrichtigung erscheint: „14:00–15:00“, „Morgen, 9:00–9:30“,
    /// „Di 10. Nov., 9:00–9:30“, ganztägig „Ganztägig“ bzw. „Morgen, ganztägig“.
    /// Mit Ort dahinter: „, Raum 2“.
    static func body(for occ: Occurrence, firingOn fireDay: LocalDate, today: LocalDate, calendar cal: Calendar) -> String {
        let startDay = LocalDate(occ.start, calendar: cal)
        let dayPrefix: String?
        switch fireDay.days(until: startDay) {
        case 0: dayPrefix = nil
        case 1: dayPrefix = "Morgen"
        default:
            let wd = QuickEntryPreview.weekdayShort[startDay.weekday.rawValue - 1]
            let month = QuickEntryPreview.monthShort[startDay.month - 1]
            let year = startDay.year == today.year ? "" : " \(startDay.year)"
            dayPrefix = "\(wd) \(startDay.day). \(month)\(year)"
        }
        var text: String
        if occ.isAllDay {
            text = dayPrefix.map { "\($0), ganztägig" } ?? "Ganztägig"
        } else {
            let range = "\(clock(occ.start, cal))–\(clock(occ.end, cal))"
            text = dayPrefix.map { "\($0), \(range)" } ?? range
        }
        if let location = occ.details.location?.trimmingCharacters(in: .whitespacesAndNewlines), !location.isEmpty {
            text += ", \(location)"
        }
        return text
    }

    static func clock(_ d: Date, _ cal: Calendar) -> String {
        let c = cal.dateComponents([.hour, .minute], from: d)
        return String(format: "%d:%02d", c.hour!, c.minute!)
    }

    /// Was die App ändern muss: Kennungen zum Entfernen und Erinnerungen zum
    /// (Neu-)Einplanen. Geänderte Erinnerungen (andere Zeit oder Text) stehen
    /// in beiden Listen.
    public static func diff(scheduled: [PlannedReminder], wanted: [PlannedReminder]) -> (remove: [String], add: [PlannedReminder]) {
        let old = Dictionary(scheduled.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let new = Dictionary(wanted.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let remove = old.filter { id, r in new[id] != r }.keys.sorted()
        let add = wanted.filter { old[$0.id] != $0 }
        return (remove, add)
    }
}
