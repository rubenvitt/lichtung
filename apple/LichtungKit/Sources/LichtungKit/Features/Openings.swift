import Foundation

// Buchungslinks in der App (Spec §6, LIC-35): offene Einmal-Links zum
// Kopieren, Teilen und Löschen, feste Links (Vorlagen) anlegen und ändern,
// neuer Einmal-Link mit Dauer, Zielkalender und optional Vorschlägen.
// Regeln und Grenzen wie im Vertrag (docs/api/openings.md); der Server
// prüft trotzdem selbst.

// MARK: - Zielkalender

public enum OpeningTargets {
    /// Kalender, die als Ziel eines Links taugen: für Lichtung beschreibbar
    /// und mit Einladungen (`can_invite`, Spec §6). Ausgeblendete auch, sie
    /// buchen genauso. Sortiert wie in den Einstellungen.
    public static func allowed(_ calendars: [String: CalendarInfo]) -> [CalendarInfo] {
        calendars.values.filter(isAllowed).sorted { a, b in
            if a.position != b.position { return a.position < b.position }
            return a.name.localizedStandardCompare(b.name) == .orderedAscending
        }
    }

    public static func isAllowed(_ calendar: CalendarInfo?) -> Bool {
        guard let calendar else { return false }
        return calendar.writable && calendar.canInvite
    }

    /// Vorauswahl: der zuletzt gewählte, wenn er noch taugt, sonst der erste.
    public static func preferred(_ calendars: [String: CalendarInfo], last: String?) -> String? {
        let list = allowed(calendars)
        if let last, list.contains(where: { $0.id == last }) { return last }
        return list.first?.id
    }

    /// Hinweis, wenn kein Kalender taugt.
    public static let noTargetHint = "Kein Kalender kann Einladungen verschicken. Ziel eines Buchungslinks ist ein beschreibbarer Kalender mit Einladungen, z. B. von Fastmail."
}

// MARK: - Texte

public enum OpeningText {
    /// Wochentage in der Reihenfolge des Vertrags (`mo` … `su`).
    public static let weekdayCodes = ["mo", "tu", "we", "th", "fr", "sa", "su"]

    public static func code(_ weekday: Weekday) -> String { weekdayCodes[weekday.rawValue - 1] }

    public static func weekday(code: String) -> Weekday? {
        weekdayCodes.firstIndex(of: code.lowercased()).flatMap { Weekday(rawValue: $0 + 1) }
    }

    /// „Mo“ … „So“.
    public static func shortName(_ weekday: Weekday) -> String {
        QuickEntryPreview.weekdayShort[weekday.rawValue - 1]
    }

    /// „30 Min.“, „1 Std.“, „1 Std. 30 Min.“.
    public static func duration(_ minutes: Int) -> String {
        let h = minutes / 60, m = minutes % 60
        switch (h, m) {
        case (0, _): return "\(m) Min."
        case (_, 0): return "\(h) Std."
        default: return "\(h) Std. \(m) Min."
        }
    }

    /// „1 Vorschlag“, „3 Vorschläge“.
    public static func suggestionCount(_ n: Int) -> String {
        n == 1 ? "1 Vorschlag" : "\(n) Vorschläge"
    }

    /// Minuten seit Mitternacht als „09:00“.
    public static func clock(_ minutes: Int) -> String {
        String(format: "%02d:%02d", minutes / 60, minutes % 60)
    }

    /// „09:00“ → 540; `nil` bei anderer Form.
    public static func minutes(clock text: String) -> Int? {
        let parts = text.split(separator: ":").map { Int($0) }
        guard parts.count == 2, let h = parts[0], let m = parts[1], (0...23).contains(h), (0...59).contains(m) else { return nil }
        return h * 60 + m
    }

    /// Wochenfenster knapp: „Mo–Fr 09:00–17:00“, „Mo, Mi 09:00–12:00 · Di 14:00–16:00“.
    /// Tage mit gleichen Zeiten stehen zusammen, aufeinanderfolgende ab drei
    /// Tagen als Spanne.
    public static func windows(_ windows: [Opening.Window]) -> String {
        var byDay: [Weekday: [String]] = [:]
        for w in windows {
            guard let day = weekday(code: w.weekday) else { continue }
            byDay[day, default: []].append("\(w.start)–\(w.end)")
        }
        guard !byDay.isEmpty else { return "keine Zeitfenster" }
        // Gleiche Zeiten zusammenfassen, in der Reihenfolge des ersten Tags.
        var groups: [(times: String, days: [Weekday])] = []
        for day in Weekday.allCases {
            guard let times = byDay[day]?.sorted().joined(separator: ", ") else { continue }
            if let i = groups.firstIndex(where: { $0.times == times }) {
                groups[i].days.append(day)
            } else {
                groups.append((times, [day]))
            }
        }
        return groups.map { "\(days($0.days)) \($0.times)" }.joined(separator: " · ")
    }

    /// „Mo–Fr“, „Mo, Mi“, „Sa, So“.
    static func days(_ list: [Weekday]) -> String {
        var runs: [[Weekday]] = []
        for d in list {
            if let last = runs.last?.last, last.rawValue + 1 == d.rawValue {
                runs[runs.count - 1].append(d)
            } else {
                runs.append([d])
            }
        }
        return runs.map { run in
            run.count >= 3 ? "\(shortName(run[0]))–\(shortName(run[run.count - 1]))" : run.map(shortName).joined(separator: ", ")
        }.joined(separator: ", ")
    }

    /// „4 Std. Vorlauf“, „2 Tage Vorlauf“, „ohne Vorlauf“.
    public static func notice(_ minutes: Int) -> String {
        if minutes == 0 { return "ohne Vorlauf" }
        if minutes % 1440 == 0 {
            let d = minutes / 1440
            return d == 1 ? "1 Tag Vorlauf" : "\(d) Tage Vorlauf"
        }
        return "\(duration(minutes)) Vorlauf"
    }

    /// „21 Tage voraus“.
    public static func horizon(_ days: Int) -> String {
        days == 1 ? "1 Tag voraus" : "\(days) Tage voraus"
    }

    /// „10 Min. Puffer“, „Puffer 10 Min. vorher, 5 Min. nachher“; `nil` ohne Puffer.
    public static func buffers(before: Int, after: Int) -> String? {
        switch (before, after) {
        case (0, 0): return nil
        case let (b, a) where b == a: return "\(duration(b)) Puffer"
        case let (b, 0): return "\(duration(b)) Puffer vorher"
        case let (0, a): return "\(duration(a)) Puffer nachher"
        case let (b, a): return "Puffer \(duration(b)) vorher, \(duration(a)) nachher"
        }
    }

    /// Ein Vorschlag in der Zeitzone des Geräts: „Morgen · Di 3. · 10:00–10:30“.
    public static func slot(start: Date, end: Date, today: LocalDate, calendar cal: Calendar) -> String {
        let s = LocalDateTime(start, calendar: cal), e = LocalDateTime(end, calendar: cal)
        let heading = AgendaBuilder.heading(s.date, today: today)
        return "\(heading) · \(clock(s.hour * 60 + s.minute))–\(clock(e.hour * 60 + e.minute))"
    }

    /// Auswahl für Dauer, Puffer, Vorlauf und Horizont; ein abweichender
    /// aktueller Wert kommt dazu, damit die Auswahl ihn zeigen kann.
    public static func choices(_ base: [Int], including current: Int) -> [Int] {
        base.contains(current) ? base : (base + [current]).sorted()
    }

    public static let durationChoices = [15, 20, 30, 45, 60, 90, 120]
    public static let bufferChoices = [0, 5, 10, 15, 30, 60]
    public static let noticeChoices = [0, 60, 120, 240, 480, 1440, 2880]
    public static let horizonChoices = [7, 14, 21, 28, 42, 60, 90]

    /// Text zum Teilen eines Links (iOS-Teilen-Menü, Zwischenablage).
    public static func shareText(_ opening: Opening) -> String {
        opening.url
    }
}

extension Opening {
    /// Zeile unter dem Titel: „30 Min. · Privat · 2 Vorschläge“,
    /// „30 Min. · Privat · freie Zeiten“, bei Vorlagen „… · Mo–Fr 09:00–17:00“.
    public func summary(calendars: [String: CalendarInfo]) -> String {
        var parts = [OpeningText.duration(durationMinutes)]
        parts.append(calendars[calendarId]?.name ?? "Unbekannter Kalender")
        switch kind {
        case .once where suggestionsOnly:
            parts.append(suggestions.isEmpty ? "keine Vorschläge mehr" : OpeningText.suggestionCount(suggestions.count))
        case .once:
            parts.append("freie Zeiten")
        case .template, .unknown:
            parts.append(OpeningText.windows(windows))
        }
        return parts.joined(separator: " · ")
    }

    /// Regeln für freie Zeiten (Vorlagen und Einmal-Links ohne Vorschläge):
    /// „4 Std. Vorlauf · 21 Tage voraus · 10 Min. Puffer“.
    public var rulesText: String {
        [OpeningText.notice(minNoticeMinutes), OpeningText.horizon(horizonDays),
         OpeningText.buffers(before: bufferBeforeMinutes, after: bufferAfterMinutes)]
            .compactMap { $0 }
            .joined(separator: " · ")
    }

    /// Hinweis zum Zustand, `nil` wenn alles in Ordnung ist: gebucht, alle
    /// Vorschläge vorbei, Zielkalender taugt nicht mehr, Platzhalter fehlt.
    public func statusText(calendars: [String: CalendarInfo], today: LocalDate, calendar cal: Calendar) -> String? {
        if let bookedAt {
            return "Gebucht · \(AgendaBuilder.heading(LocalDate(bookedAt, calendar: cal), today: today))"
        }
        if isExhausted { return "Alle Vorschläge sind vorbei, der Link bietet nichts mehr an." }
        if !OpeningTargets.isAllowed(calendars[calendarId]) {
            return "Der Zielkalender nimmt keine Buchungen mehr an. Bitte einen anderen wählen."
        }
        let missing = suggestions.filter { $0.placeholderStatus == .missing }.count
        if missing > 0 {
            return missing == 1
                ? "Ein Platzhalter wurde im Kalender gelöscht; der Vorschlag bleibt buchbar."
                : "\(missing) Platzhalter wurden im Kalender gelöscht; die Vorschläge bleiben buchbar."
        }
        return nil
    }

    /// Lässt sich der Zielkalender noch ändern? Nicht bei Einmal-Links mit
    /// Vorschlägen (die Platzhalter liegen fest).
    public var canChangeCalendarAndDuration: Bool { !suggestionsOnly }
}

// MARK: - Liste

/// Die Buchungslinks für die Verwaltung: offene Einmal-Links (zum Löschen),
/// Vorlagen und kürzlich gebuchte Einmal-Links.
public struct OpeningList: Hashable, Sendable {
    /// Einmal-Links, die noch nicht gebucht sind, neueste zuerst.
    public var open: [Opening]
    /// Feste Links nach Titel.
    public var templates: [Opening]
    /// Gebuchte Einmal-Links der letzten Tage, zuletzt gebuchter zuerst.
    public var booked: [Opening]

    /// Wie lange gebuchte Einmal-Links in der Liste bleiben.
    public static let recentDays = 30

    public init(_ openings: some Sequence<Opening>, now: Date, recentDays: Int = OpeningList.recentDays) {
        let all = Array(openings)
        let newestFirst: (Opening, Opening) -> Bool = { a, b in
            let (x, y) = (a.createdAt ?? .distantPast, b.createdAt ?? .distantPast)
            return x != y ? x > y : a.id > b.id
        }
        open = all.filter { $0.kind == .once && !$0.isBooked }.sorted(by: newestFirst)
        templates = all.filter { $0.kind == .template }.sorted { a, b in
            let c = a.title.localizedStandardCompare(b.title)
            return c != .orderedSame ? c == .orderedAscending : a.id < b.id
        }
        let since = now.addingTimeInterval(-TimeInterval(recentDays) * 86_400)
        booked = all.filter { $0.kind == .once && ($0.bookedAt.map { $0 >= since } ?? false) }
            .sorted { ($0.bookedAt ?? .distantPast) > ($1.bookedAt ?? .distantPast) }
    }

    public var isEmpty: Bool { open.isEmpty && templates.isEmpty && booked.isEmpty }
}

extension SyncState {
    /// Buchungslinks für die Verwaltung.
    public func openingList(now: Date) -> OpeningList {
        OpeningList(openings.values, now: now)
    }
}

// MARK: - Neuer Einmal-Link

/// Eingaben für einen neuen Einmal-Link: Dauer, Zielkalender, optional
/// Titel und konkrete Vorschläge (Beginn; das Ende ergibt sich aus der Dauer).
public struct OneOffLinkDraft: Hashable, Sendable {
    public var durationMinutes: Int
    public var calendarId: String?
    public var title: String
    /// Beginn der Vorschläge, in der Reihenfolge der Eingabe.
    public var suggestions: [Date]

    public init(durationMinutes: Int = 30, calendarId: String? = nil, title: String = "", suggestions: [Date] = []) {
        self.durationMinutes = durationMinutes
        self.calendarId = calendarId
        self.title = title
        self.suggestions = suggestions
    }

    /// Vorschlag für einen weiteren Vorschlag: einen Tag nach dem letzten,
    /// sonst morgen 10:00 in `calendar`s Zeitzone.
    public func nextSuggestion(now: Date, calendar cal: Calendar) -> Date {
        if let last = suggestions.max() {
            return cal.date(byAdding: .day, value: 1, to: last) ?? last.addingTimeInterval(86_400)
        }
        let tomorrow = LocalDate(now, calendar: cal).adding(days: 1)
        return LocalDateTime(date: tomorrow, hour: 10, minute: 0).instant(in: cal)
    }

    public mutating func addSuggestion(now: Date, calendar cal: Calendar) {
        suggestions.append(nextSuggestion(now: now, calendar: cal))
    }

    /// Was fehlt oder nicht passt, als Satz; `nil` wenn sich der Link anlegen lässt.
    public func problem(now: Date, calendars: [String: CalendarInfo]) -> String? {
        if !(Opening.minDurationMinutes...Opening.maxDurationMinutes).contains(durationMinutes) {
            return "Die Dauer muss zwischen 5 Minuten und 12 Stunden liegen."
        }
        guard let calendarId else { return "Bitte einen Zielkalender wählen." }
        guard OpeningTargets.isAllowed(calendars[calendarId]) else {
            return "Der Zielkalender muss beschreibbar sein und Einladungen verschicken können."
        }
        if let t = OpeningDraftRules.titleProblem(title, required: false) { return t }
        if suggestions.count > Opening.maxSuggestions {
            return "Ein Einmal-Link hat höchstens \(Opening.maxSuggestions) Vorschläge."
        }
        let latest = now.addingTimeInterval(TimeInterval(Opening.maxHorizonDays) * 86_400)
        for s in suggestions {
            if s <= now { return "Ein Vorschlag liegt in der Vergangenheit." }
            if s > latest { return "Vorschläge liegen höchstens \(Opening.maxHorizonDays) Tage in der Zukunft." }
        }
        if Set(suggestions.map(\.timeIntervalSinceReferenceDate)).count != suggestions.count {
            return "Ein Vorschlag steht doppelt in der Liste."
        }
        return nil
    }

    /// Der Aufruf für `POST /api/openings`; `nil` solange `problem` etwas meldet.
    public func newOpening(now: Date, calendars: [String: CalendarInfo], timeZone: TimeZone) -> NewOpening? {
        guard problem(now: now, calendars: calendars) == nil, let calendarId else { return nil }
        let t = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return NewOpening(
            kind: .once, title: t.isEmpty ? nil : t, durationMinutes: durationMinutes, calendarId: calendarId,
            timeZone: timeZone.identifier,
            suggestions: suggestions.isEmpty ? nil : suggestions.sorted().map { NewOpening.SuggestionInput(start: $0) }
        )
    }

    /// Kurzfassung vor dem Anlegen: „30 Min. · Privat · 2 Vorschläge“.
    public func summary(calendars: [String: CalendarInfo]) -> String {
        [OpeningText.duration(durationMinutes),
         calendarId.flatMap { calendars[$0]?.name } ?? "kein Kalender",
         suggestions.isEmpty ? "freie Zeiten" : OpeningText.suggestionCount(suggestions.count)]
            .joined(separator: " · ")
    }

    /// Erklärung unter den Vorschlägen.
    public var suggestionsHint: String {
        suggestions.isEmpty
            ? "Ohne Vorschläge bietet der Link freie Zeiten Mo–Fr 09:00–17:00 an, frühestens in 4 Stunden, höchstens 3 Wochen voraus."
            : "Jeder Vorschlag wird sofort als „Reserviert“ im Zielkalender eingetragen und blockiert die Zeit, bis gebucht oder der Link gelöscht ist."
    }
}

// MARK: - Vorlage

/// Eingaben einer Vorlage (fester Link) zum Anlegen oder Ändern.
public struct OpeningTemplateDraft: Hashable, Sendable {
    /// Ein Wochenfenster, Zeiten als Minuten seit Mitternacht.
    public struct WindowDraft: Hashable, Sendable, Identifiable {
        public var id: UUID
        public var weekday: Weekday
        public var start: Int
        public var end: Int

        public init(id: UUID = UUID(), weekday: Weekday, start: Int, end: Int) {
            self.id = id
            self.weekday = weekday
            self.start = start
            self.end = end
        }

        var window: Opening.Window {
            Opening.Window(weekday: OpeningText.code(weekday), start: OpeningText.clock(start), end: OpeningText.clock(end))
        }
    }

    public var original: Opening?
    public var title: String
    public var durationMinutes: Int
    public var calendarId: String?
    public var timeZone: String
    public var windows: [WindowDraft]
    public var bufferBeforeMinutes: Int
    public var bufferAfterMinutes: Int
    public var minNoticeMinutes: Int
    public var horizonDays: Int
    public var videoURL: String

    /// Neue Vorlage mit den Standardwerten des Vertrags.
    public init(calendarId: String?, timeZone: TimeZone) {
        original = nil
        title = ""
        durationMinutes = 30
        self.calendarId = calendarId
        self.timeZone = timeZone.identifier
        windows = Self.drafts(Opening.defaultWindows)
        bufferBeforeMinutes = 0
        bufferAfterMinutes = 0
        minNoticeMinutes = Opening.defaultMinNoticeMinutes
        horizonDays = Opening.defaultHorizonDays
        videoURL = ""
    }

    /// Bestehenden Link bearbeiten.
    public init(editing opening: Opening) {
        original = opening
        title = opening.title
        durationMinutes = opening.durationMinutes
        calendarId = opening.calendarId
        timeZone = opening.timeZone
        windows = Self.drafts(opening.windows)
        bufferBeforeMinutes = opening.bufferBeforeMinutes
        bufferAfterMinutes = opening.bufferAfterMinutes
        minNoticeMinutes = opening.minNoticeMinutes
        horizonDays = opening.horizonDays
        videoURL = opening.videoUrl ?? ""
    }

    static func drafts(_ windows: [Opening.Window]) -> [WindowDraft] {
        windows.compactMap { w in
            guard let day = OpeningText.weekday(code: w.weekday),
                  let s = OpeningText.minutes(clock: w.start), let e = OpeningText.minutes(clock: w.end) else { return nil }
            return WindowDraft(weekday: day, start: s, end: e)
        }
    }

    public var isNew: Bool { original == nil }

    /// Neues Fenster: der nächste Wochentag nach dem letzten mit dessen Zeiten,
    /// sonst Montag 09:00–17:00.
    public mutating func addWindow() {
        guard let last = windows.last else {
            windows.append(WindowDraft(weekday: .monday, start: 9 * 60, end: 17 * 60))
            return
        }
        let next = Weekday(rawValue: last.weekday.rawValue % 7 + 1) ?? .monday
        windows.append(WindowDraft(weekday: next, start: last.start, end: last.end))
    }

    /// Fenster nach Wochentag und Beginn, wie sie gesendet werden.
    var sortedWindows: [Opening.Window] {
        windows.sorted { ($0.weekday.rawValue, $0.start, $0.end) < ($1.weekday.rawValue, $1.start, $1.end) }.map(\.window)
    }

    var trimmedVideoURL: String { videoURL.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// Was fehlt oder nicht passt, als Satz; `nil` wenn sich die Vorlage sichern lässt.
    public func problem(calendars: [String: CalendarInfo]) -> String? {
        if let t = OpeningDraftRules.titleProblem(title, required: true) { return t }
        if !(Opening.minDurationMinutes...Opening.maxDurationMinutes).contains(durationMinutes) {
            return "Die Dauer muss zwischen 5 Minuten und 12 Stunden liegen."
        }
        guard let calendarId else { return "Bitte einen Zielkalender wählen." }
        if calendarId != original?.calendarId, !OpeningTargets.isAllowed(calendars[calendarId]) {
            return "Der Zielkalender muss beschreibbar sein und Einladungen verschicken können."
        }
        if TimeZone(identifier: timeZone) == nil { return "Unbekannte Zeitzone „\(timeZone)“." }
        if windows.isEmpty { return "Mindestens ein Zeitfenster angeben, z. B. Montag 09:00–17:00." }
        if windows.count > Opening.maxWindows { return "Höchstens \(Opening.maxWindows) Zeitfenster." }
        if let w = windows.first(where: { $0.end <= $0.start }) {
            return "Das Zeitfenster am \(OpeningText.shortName(w.weekday)) muss nach seinem Beginn enden (über Mitternacht geht nicht)."
        }
        for (name, value) in [("Puffer vorher", bufferBeforeMinutes), ("Puffer nachher", bufferAfterMinutes)]
        where !(0...Opening.maxBufferMinutes).contains(value) {
            return "\(name) darf höchstens 12 Stunden sein."
        }
        if !(0...Opening.maxNoticeMinutes).contains(minNoticeMinutes) { return "Der Vorlauf darf höchstens 30 Tage sein." }
        if !(1...Opening.maxHorizonDays).contains(horizonDays) { return "Der Horizont liegt zwischen 1 und 365 Tagen." }
        if !trimmedVideoURL.isEmpty, !OpeningDraftRules.isWebURL(trimmedVideoURL) {
            return "Der Videolink muss eine https-Adresse sein, z. B. https://meet.example.org/raum."
        }
        return nil
    }

    /// Aufruf für `POST /api/openings` (nur neue Vorlagen).
    public func newOpening(calendars: [String: CalendarInfo]) -> NewOpening? {
        guard isNew, problem(calendars: calendars) == nil, let calendarId else { return nil }
        return NewOpening(
            kind: .template, title: title.trimmingCharacters(in: .whitespacesAndNewlines), durationMinutes: durationMinutes,
            calendarId: calendarId, timeZone: timeZone, windows: sortedWindows,
            bufferBeforeMinutes: bufferBeforeMinutes, bufferAfterMinutes: bufferAfterMinutes,
            minNoticeMinutes: minNoticeMinutes, horizonDays: horizonDays,
            videoUrl: trimmedVideoURL.isEmpty ? nil : trimmedVideoURL
        )
    }

    /// Aufruf für `PATCH /api/openings/{id}` mit nur den geänderten Feldern;
    /// `nil` bei neuen Vorlagen oder wenn `problem` etwas meldet. Leer, wenn
    /// sich nichts geändert hat.
    public func patch(calendars: [String: CalendarInfo]) -> OpeningPatch? {
        guard let o = original, problem(calendars: calendars) == nil, let calendarId else { return nil }
        var p = OpeningPatch()
        let t = title.trimmingCharacters(in: .whitespacesAndNewlines)
        if t != o.title { p.title = t }
        if o.canChangeCalendarAndDuration {
            if durationMinutes != o.durationMinutes { p.durationMinutes = durationMinutes }
            if calendarId != o.calendarId { p.calendarId = calendarId }
        }
        if timeZone != o.timeZone { p.timeZone = timeZone }
        let sorted = sortedWindows
        if sorted != Self.drafts(o.windows).sorted(by: { ($0.weekday.rawValue, $0.start, $0.end) < ($1.weekday.rawValue, $1.start, $1.end) }).map(\.window) {
            p.windows = sorted
        }
        if bufferBeforeMinutes != o.bufferBeforeMinutes { p.bufferBeforeMinutes = bufferBeforeMinutes }
        if bufferAfterMinutes != o.bufferAfterMinutes { p.bufferAfterMinutes = bufferAfterMinutes }
        if minNoticeMinutes != o.minNoticeMinutes { p.minNoticeMinutes = minNoticeMinutes }
        if horizonDays != o.horizonDays { p.horizonDays = horizonDays }
        let video = trimmedVideoURL
        if video != (o.videoUrl ?? "") { p.videoUrl = video.isEmpty ? .clear : .set(video) }
        return p
    }
}

/// Gemeinsame Prüfungen der Entwürfe.
enum OpeningDraftRules {
    static func titleProblem(_ raw: String, required: Bool) -> String? {
        let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if t.isEmpty { return required ? "Die Vorlage braucht einen Titel, z. B. „30 Minuten Kennenlernen“." : nil }
        if t.count > Opening.maxTitleLength { return "Der Titel hat höchstens \(Opening.maxTitleLength) Zeichen." }
        if t.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) {
            return "Der Titel darf keine Steuerzeichen enthalten."
        }
        return nil
    }

    static func isWebURL(_ text: String) -> Bool {
        guard text.count <= Opening.maxVideoURLLength, let url = URL(string: text),
              let scheme = url.scheme?.lowercased(), scheme == "https" || scheme == "http",
              url.host?.isEmpty == false else { return false }
        return true
    }
}
