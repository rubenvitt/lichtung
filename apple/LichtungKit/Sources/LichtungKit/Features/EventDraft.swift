import Foundation

/// Entwurf im Termin-Editor (Spec §5 „Termin anlegen und bearbeiten“, LIC-20).
///
/// Entsteht leer an einem Tag (neuer Termin) oder aus einem angezeigten
/// Vorkommen. Die Oberfläche bindet direkt an die Felder; erst beim Sichern
/// wird daraus eine Absicht (`EditIntent`): beim Anlegen `create`, beim
/// Ändern ein `update` mit genau den Feldern, die sich gegenüber dem Stand
/// beim Öffnen geändert haben. Teilnehmende kommen mit E3 dazu.
///
/// Beginn und Ende sind Wanduhrzeiten in `timeZoneID` (`nil` = floating).
/// Bei ganztägigen Terminen zählen nur die Tage, und `end` ist der **letzte**
/// Tag (inklusive), wie ihn die Oberfläche zeigt; die Uhrzeiten bleiben
/// erhalten, falls „ganztägig“ wieder ausgeschaltet wird.
public struct EventDraft: Identifiable, Hashable, Sendable {
    /// Wiederholung, wie der Editor sie kennt.
    public enum Repetition: Hashable, Sendable {
        case none
        case rule(RecurrenceRule)
        /// Regel vom Anbieter, die der Editor nicht abbildet (z. B. „jeden
        /// zweiten Dienstag im Monat“). Bleibt unverändert, solange sie nicht
        /// durch eine andere ersetzt wird.
        case custom(String)
    }

    /// Auswahl „Wiederholen“ in der Oberfläche.
    public enum RepeatFrequency: String, CaseIterable, Hashable, Sendable, Identifiable {
        case none, daily, weekly, monthly, yearly, custom

        public var id: String { rawValue }

        public var label: String {
            switch self {
            case .none: "Nie"
            case .daily: "Täglich"
            case .weekly: "Wöchentlich"
            case .monthly: "Monatlich"
            case .yearly: "Jährlich"
            case .custom: "Benutzerdefiniert"
            }
        }

        /// „Jede Woche“, „Alle 2 Wochen“.
        public func intervalText(_ n: Int) -> String {
            switch self {
            case .daily: n == 1 ? "Jeden Tag" : "Alle \(n) Tage"
            case .weekly: n == 1 ? "Jede Woche" : "Alle \(n) Wochen"
            case .monthly: n == 1 ? "Jeden Monat" : "Alle \(n) Monate"
            case .yearly: n == 1 ? "Jedes Jahr" : "Alle \(n) Jahre"
            case .none, .custom: ""
            }
        }

        var frequency: RecurrenceRule.Frequency? {
            switch self {
            case .daily: .daily
            case .weekly: .weekly
            case .monthly: .monthly
            case .yearly: .yearly
            case .none, .custom: nil
            }
        }
    }

    /// Auswahl „Endet“ einer Wiederholung.
    public enum RepeatEnd: String, CaseIterable, Hashable, Sendable, Identifiable {
        case never, until, count

        public var id: String { rawValue }

        public var label: String {
            switch self {
            case .never: "Nie"
            case .until: "Am Datum"
            case .count: "Nach Anzahl"
            }
        }
    }

    /// Alle Felder, die der Editor ändern kann (Vergleich mit dem Stand beim Öffnen).
    public struct Fields: Hashable, Sendable {
        public var title: String
        public var isAllDay: Bool
        public var start: LocalDateTime
        public var end: LocalDateTime
        public var timeZoneID: String?
        public var endTimeZoneID: String?
        public var location: String
        public var notes: String
        public var alarms: [Int]
        public var repetition: Repetition
        public var calendarId: String
    }

    /// Der bestehende Termin, aus dem der Entwurf entstand.
    public struct Original: Hashable, Sendable {
        public var eventId: String
        public var calendarId: String
        /// Ursprünglicher Beginn des Vorkommens, `nil` bei Einzelterminen.
        public var recurrenceId: DateValue?
        /// Beginn der Serie (Master), für „Alle“.
        public var seriesStart: DateValue?
        public var recurrence: Recurrence?
        public var fields: Fields
    }

    public let id: String
    public let deviceTimeZone: TimeZone
    public let original: Original?

    public var title: String
    public private(set) var isAllDay: Bool
    public private(set) var start: LocalDateTime
    public var end: LocalDateTime
    public private(set) var timeZoneID: String?
    public private(set) var endTimeZoneID: String?
    public var location: String
    public var notes: String
    /// Erinnerungen als Vorlauf in Minuten vor Beginn (negativ = danach), sortiert.
    public private(set) var alarms: [Int]
    /// Erinnerungen zu festen Zeitpunkten. Der Editor zeigt sie nur an und
    /// schickt sie unverändert mit, wenn sich die Erinnerungen ändern.
    public let fixedAlarms: [Date]
    public var repetition: Repetition
    public var calendarId: String

    public static let defaultTitle = "Neuer Termin"

    // MARK: - Entstehen

    /// Neuer Termin an `day`: heute zur nächsten vollen Stunde, sonst um 9 Uhr,
    /// eine Stunde lang, in der Zeitzone des Geräts.
    public static func new(
        on day: LocalDate, now: Date, calendarId: String?, deviceTimeZone tz: TimeZone,
        id: String = UUID().uuidString
    ) -> EventDraft {
        let cal = Calendar.lichtung(timeZone: tz)
        let hour: Int
        if day == LocalDate(now, calendar: cal) {
            hour = min(23, cal.component(.hour, from: now) + 1)
        } else {
            hour = 9
        }
        let start = LocalDateTime(date: day, hour: hour, minute: 0)
        return new(startingAt: start, calendarId: calendarId, deviceTimeZone: tz, id: id)
    }

    /// Neuer Termin mit festem Beginn, z. B. „Neuer Termin hier“ im Raster
    /// (LIC-24). `minutes` lang, in der Zeitzone des Geräts.
    public static func new(
        startingAt start: LocalDateTime, minutes: Int = 60, calendarId: String?, deviceTimeZone tz: TimeZone,
        id: String = UUID().uuidString
    ) -> EventDraft {
        EventDraft(
            id: id, deviceTimeZone: tz, original: nil,
            fields: Fields(
                title: "", isAllDay: false, start: start, end: start.addingSeconds(max(1, minutes) * 60),
                timeZoneID: tz.identifier, endTimeZoneID: tz.identifier, location: "", notes: "",
                alarms: [], repetition: .none, calendarId: calendarId ?? ""
            ),
            fixedAlarms: []
        )
    }

    /// Entwurf aus einem angezeigten Vorkommen. `event` ist der (sichtbare)
    /// Stand des Events dazu; ohne ihn gilt das Vorkommen als Einzeltermin.
    public init(editing occ: Occurrence, event: Event?, deviceTimeZone tz: TimeZone, id: String = UUID().uuidString) {
        let d = occ.details
        let (start, zone) = Self.wallClock(d.start, deviceTimeZone: tz, defaultHour: 9)
        var (end, endZone) = Self.wallClock(d.end, deviceTimeZone: tz, defaultHour: 10)
        if d.start.isAllDay {
            // Ende ist exklusiv; der Editor zeigt den letzten Tag.
            end.date = max(start.date, end.date.adding(days: -1))
        }
        var minutes: [Int] = []
        var fixed: [Date] = []
        for a in d.alarms {
            switch a {
            case let .minutesBefore(m): minutes.append(m)
            case let .at(date): fixed.append(date)
            }
        }
        let repetition: Repetition
        if let r = event?.recurrence {
            let seriesZone = event?.details.start.timeZoneID.flatMap(TimeZone.init(identifier:)) ?? tz
            if r.incomplete != true, let rule = RecurrenceRule(rrule: r.rrule, timeZone: seriesZone) {
                repetition = .rule(rule)
            } else {
                repetition = .custom(r.rrule)
            }
        } else {
            repetition = .none
        }
        let fields = Fields(
            title: d.title, isAllDay: d.start.isAllDay, start: start, end: end,
            timeZoneID: zone, endTimeZoneID: endZone, location: d.location ?? "", notes: d.notes ?? "",
            alarms: Array(Set(minutes)).sorted(), repetition: repetition, calendarId: occ.calendarId
        )
        let original = Original(
            eventId: occ.eventId, calendarId: occ.calendarId,
            recurrenceId: event?.recurrence == nil ? nil : occ.recurrenceId,
            seriesStart: event?.details.start, recurrence: event?.recurrence, fields: fields
        )
        self.init(id: id, deviceTimeZone: tz, original: original, fields: fields, fixedAlarms: fixed)
    }

    private init(id: String, deviceTimeZone: TimeZone, original: Original?, fields f: Fields, fixedAlarms: [Date]) {
        self.id = id
        self.deviceTimeZone = deviceTimeZone
        self.original = original
        self.fixedAlarms = fixedAlarms
        title = f.title
        isAllDay = f.isAllDay
        start = f.start
        end = f.end
        timeZoneID = f.timeZoneID
        endTimeZoneID = f.endTimeZoneID
        location = f.location
        notes = f.notes
        alarms = f.alarms
        repetition = f.repetition
        calendarId = f.calendarId
    }

    /// Wanduhrzeit und Zone eines Datumswerts. Ganztägig: der Tag um
    /// `defaultHour` in der Gerätezone. Ein Zeitpunkt ohne übersetzbare Zone
    /// (`time_zone: null`, `utc` gesetzt) gilt in der Gerätezone.
    static func wallClock(_ v: DateValue, deviceTimeZone tz: TimeZone, defaultHour: Int) -> (LocalDateTime, String?) {
        switch v {
        case let .date(d):
            return (LocalDateTime(date: d, hour: defaultHour, minute: 0), tz.identifier)
        case let .dateTime(local, zone, utc):
            if zone == nil, let utc {
                return (LocalDateTime(utc, calendar: .lichtung(timeZone: tz)), tz.identifier)
            }
            return (local, zone)
        }
    }

    // MARK: - Zustand

    public var isNew: Bool { original == nil }

    /// Vorkommen einer Serie: Beim Sichern und Löschen wird nach dem
    /// Geltungsbereich gefragt.
    public var isSeriesOccurrence: Bool { original?.recurrenceId != nil }

    public var fields: Fields {
        Fields(
            title: title, isAllDay: isAllDay, start: start, end: end, timeZoneID: timeZoneID,
            endTimeZoneID: endTimeZoneID, location: location, notes: notes, alarms: alarms,
            repetition: repetition, calendarId: calendarId
        )
    }

    /// Neu oder gegenüber dem Stand beim Öffnen geändert.
    public var hasChanges: Bool {
        guard let original else { return true }
        return Self.differs(fields, original.fields)
    }

    /// Titel, wie er gespeichert wird: leer wird „Neuer Termin“.
    public var effectiveTitle: String {
        let t = title.trimmed
        return t.isEmpty ? Self.defaultTitle : t
    }

    public var startValue: DateValue { Self.startValue(fields) }
    public var endValue: DateValue { Self.endValue(fields) }

    public var startInstant: Date { startValue.instant(deviceTimeZone: deviceTimeZone) }
    public var endInstant: Date { endValue.instant(deviceTimeZone: deviceTimeZone) }

    static func startValue(_ f: Fields) -> DateValue {
        f.isAllDay ? .date(f.start.date) : .dateTime(f.start, timeZone: f.timeZoneID)
    }

    static func endValue(_ f: Fields) -> DateValue {
        f.isAllDay ? .date(f.end.date.adding(days: 1)) : .dateTime(f.end, timeZone: f.endTimeZoneID)
    }

    /// Vergleich ohne Felder, die gerade keine Rolle spielen (Uhrzeiten und
    /// Zonen bei ganztägigen Terminen, Leerzeichen um Titel und Ort).
    static func differs(_ a: Fields, _ b: Fields) -> Bool {
        a.title.trimmed != b.title.trimmed
            || startValue(a) != startValue(b) || endValue(a) != endValue(b)
            || a.location.trimmed != b.location.trimmed || a.notes != b.notes
            || a.alarms != b.alarms || a.repetition != b.repetition || a.calendarId != b.calendarId
    }

    // MARK: - Beginn, Ende, ganztägig

    /// Neuer Beginn; das Ende wandert mit, die Dauer bleibt.
    public mutating func moveStart(to new: LocalDateTime) {
        let delta = new.secondsSince(start)
        start = new
        end = end.addingSeconds(delta)
    }

    /// Ganztägig ein- oder ausschalten. Die Tage bleiben; ohne „ganztägig“
    /// gelten wieder die gemerkten Uhrzeiten (ein Ende vor dem Beginn wird
    /// zu Beginn plus eine Stunde).
    public mutating func setAllDay(_ on: Bool) {
        guard on != isAllDay else { return }
        isAllDay = on
        if on {
            // 23:00–0:00 bleibt ein Tag.
            if end.date > start.date, end.secondsOfDay == 0 { end.date = end.date.adding(days: -1) }
            if end.date < start.date { end.date = start.date }
        } else {
            let endZone = endTimeZoneID.flatMap(TimeZone.init(identifier:)) ?? deviceTimeZone
            let startZone = timeZoneID.flatMap(TimeZone.init(identifier:)) ?? deviceTimeZone
            if end.instant(in: .lichtung(timeZone: endZone)) <= start.instant(in: .lichtung(timeZone: startZone)) {
                end = start.addingSeconds(3600)
            }
        }
    }

    // Für die Oberfläche: Datumsauswahlen zeigen ein `Date` in der Zone des
    // Geräts. Sie bekommen die Wanduhrzeit des Termins als Zeitpunkt in dieser
    // Zone, so steht dort „9:00“, auch wenn der Termin in New York liegt.

    private var pickerCalendar: Calendar { .lichtung(timeZone: deviceTimeZone) }

    /// Beginn für die Datumsauswahl; Setzen verschiebt das Ende mit.
    public var startPickerDate: Date {
        get { start.instant(in: pickerCalendar) }
        set { moveStart(to: LocalDateTime(newValue, calendar: pickerCalendar)) }
    }

    /// Ende für die Datumsauswahl (ganztägig: der letzte Tag).
    public var endPickerDate: Date {
        get { end.instant(in: pickerCalendar) }
        set { end = LocalDateTime(newValue, calendar: pickerCalendar) }
    }

    /// Schalter „Ganztägig“ (siehe `setAllDay`).
    public var allDay: Bool {
        get { isAllDay }
        set { setAllDay(newValue) }
    }

    /// Auswahl der Zeitzone; `""` steht für floating.
    public var timeZoneSelection: String {
        get { timeZoneID ?? "" }
        set { setTimeZone(newValue.isEmpty ? nil : newValue) }
    }

    /// Letzter Tag der Serie für die Datumsauswahl.
    public var repeatUntilPickerDate: Date {
        get { repeatUntil.startDate(in: pickerCalendar) }
        set { repeatUntil = LocalDate(newValue, calendar: pickerCalendar) }
    }

    /// Zeitzone für Beginn und Ende; die Wanduhrzeit bleibt. `nil` = floating.
    public mutating func setTimeZone(_ id: String?) {
        timeZoneID = id
        endTimeZoneID = id
    }

    /// Zonen zur Auswahl: die des Termins, die des Geräts und einige häufige.
    public var timeZoneChoices: [String] {
        let candidates = [timeZoneID, endTimeZoneID, original?.fields.timeZoneID, deviceTimeZone.identifier]
            + ["Europe/Berlin", "Europe/London", "America/New_York", "America/Los_Angeles", "Asia/Tokyo", "UTC"]
        var seen: Set<String> = []
        return candidates.compactMap { $0 }.filter { seen.insert($0).inserted }
    }

    /// „Berlin“, „New York“; floating heißt „Ohne Zeitzone“.
    public static func timeZoneLabel(_ id: String?) -> String {
        guard let id, let zone = TimeZone(identifier: id) else { return "Ohne Zeitzone" }
        return TimeZoneDisplay.cityName(for: zone)
    }

    // MARK: - Erinnerungen

    public mutating func addAlarm(_ minutesBefore: Int) {
        alarms = Array(Set(alarms + [minutesBefore])).sorted()
    }

    public mutating func removeAlarm(_ minutesBefore: Int) {
        alarms.removeAll { $0 == minutesBefore }
    }

    /// Vorlagen für neue Erinnerungen. Bei ganztägigen Terminen zählt der
    /// Vorlauf ab Mitternacht des ersten Tages: −540 ist 9:00 am Tag selbst.
    public static func alarmPresets(allDay: Bool) -> [Int] {
        allDay ? [-540, 900, 2340, 9540] : [0, 5, 10, 15, 30, 60, 120, 1440, 2880]
    }

    /// Vorlagen, die noch nicht gewählt sind.
    public var alarmChoices: [Int] {
        Self.alarmPresets(allDay: isAllDay).filter { !alarms.contains($0) }
    }

    /// „Zur Startzeit“, „15 Minuten vorher“, „1 Tag vorher“; ganztägig
    /// „Am Tag des Termins um 9:00“, „1 Tag vorher um 9:00“.
    public static func alarmLabel(_ minutes: Int, allDay: Bool) -> String {
        if allDay {
            // Tage vor dem ersten Tag (aufgerundet) und Uhrzeit an jenem Tag.
            let days = Int((Double(minutes) / 1440).rounded(.up))
            let clock = days * 1440 - minutes
            let time = "\(clock / 60):" + String(format: "%02d", clock % 60)
            switch days {
            case ..<0: return "\(-days) \(-days == 1 ? "Tag" : "Tage") danach um \(time)"
            case 0: return "Am Tag des Termins um \(time)"
            default: return days % 7 == 0
                ? "\(days / 7) \(days == 7 ? "Woche" : "Wochen") vorher um \(time)"
                : "\(days) \(days == 1 ? "Tag" : "Tage") vorher um \(time)"
            }
        }
        if minutes == 0 { return "Zur Startzeit" }
        let m = abs(minutes)
        let amount: String
        if m % 1440 == 0 {
            amount = "\(m / 1440) \(m == 1440 ? "Tag" : "Tage")"
        } else if m % 60 == 0 {
            amount = "\(m / 60) \(m == 60 ? "Stunde" : "Stunden")"
        } else {
            amount = "\(m) \(m == 1 ? "Minute" : "Minuten")"
        }
        return amount + (minutes > 0 ? " vorher" : " nach Beginn")
    }

    // MARK: - Wiederholung

    /// Angebotene Auswahl; „Benutzerdefiniert“ nur, wenn der Termin so eine Regel hat.
    public var frequencyChoices: [RepeatFrequency] {
        if case .custom? = original?.fields.repetition { return RepeatFrequency.allCases }
        return RepeatFrequency.allCases.filter { $0 != .custom }
    }

    public var repeatFrequency: RepeatFrequency {
        get {
            switch repetition {
            case .none: .none
            case .custom: .custom
            case let .rule(r):
                switch r.frequency {
                case .daily: .daily
                case .weekly: .weekly
                case .monthly: .monthly
                case .yearly: .yearly
                }
            }
        }
        set {
            switch newValue {
            case .none:
                repetition = .none
            case .custom:
                if case let .custom(text)? = original?.fields.repetition { repetition = .custom(text) }
            case .daily, .weekly, .monthly, .yearly:
                guard let f = newValue.frequency, f != rule?.frequency else { return }
                // Das Ende der Serie bleibt beim Wechsel der Frequenz.
                repetition = .rule(RecurrenceRule(frequency: f, until: rule?.until, count: rule?.count))
            }
        }
    }

    private var rule: RecurrenceRule? {
        if case let .rule(r) = repetition { return r }
        return nil
    }

    private mutating func updateRule(_ change: (inout RecurrenceRule) -> Void) {
        guard var r = rule else { return }
        change(&r)
        repetition = .rule(RecurrenceRule(frequency: r.frequency, interval: r.interval, weekdays: r.weekdays, until: r.until, count: r.count))
    }

    /// „Alle n Tage/Wochen/…“, mindestens 1.
    public var repeatInterval: Int {
        get { rule?.interval ?? 1 }
        set { updateRule { $0.interval = max(1, newValue) } }
    }

    public var repeatEnd: RepeatEnd {
        get {
            if rule?.until != nil { return .until }
            if rule?.count != nil { return .count }
            return .never
        }
        set {
            let day = start.date
            updateRule { r in
                switch newValue {
                case .never:
                    r.until = nil
                    r.count = nil
                case .until:
                    r.until = r.until ?? day.adding(months: 3)
                    r.count = nil
                case .count:
                    r.count = r.count ?? 10
                    r.until = nil
                }
            }
        }
    }

    /// Letzter Tag der Serie (bei „Am Datum“).
    public var repeatUntil: LocalDate {
        get { rule?.until ?? start.date.adding(months: 3) }
        set { updateRule { $0.until = newValue; $0.count = nil } }
    }

    /// Anzahl der Termine (bei „Nach Anzahl“).
    public var repeatCount: Int {
        get { rule?.count ?? 10 }
        set { updateRule { $0.count = max(1, newValue); $0.until = nil } }
    }

    /// Wochentage einer wöchentlichen Regel; ohne eigene Auswahl der des Beginns.
    public var effectiveWeekdays: [Weekday] {
        guard let r = rule, r.frequency == .weekly else { return [] }
        return r.weekdays.isEmpty ? [start.date.weekday] : r.weekdays
    }

    /// Wochentag an- oder abwählen. Der letzte bleibt stehen.
    public mutating func toggleWeekday(_ day: Weekday) {
        var days = effectiveWeekdays
        guard rule?.frequency == .weekly else { return }
        if let i = days.firstIndex(of: day) {
            guard days.count > 1 else { return }
            days.remove(at: i)
        } else {
            days.append(day)
        }
        updateRule { $0.weekdays = days.sortedUnique() }
    }

    /// Kurzer Text, z. B. „jeden Montag und Mittwoch“, „monatlich, 10 Mal“.
    public var repeatSummary: String? {
        switch repetition {
        case .none: nil
        case .custom: "eigene Regel des Anbieters"
        case let .rule(r): QuickEntryPreview.describe(r)
        }
    }

    // MARK: - Prüfen

    /// Was das Sichern verhindert, als Satz für die Oberfläche; `nil` = in Ordnung.
    public var validationMessage: String? {
        if calendarId.isEmpty { return "Kein beschreibbarer Kalender ausgewählt." }
        if isAllDay {
            if end.date < start.date { return "Das Ende liegt vor dem Beginn." }
        } else if endInstant <= startInstant {
            return "Das Ende muss nach dem Beginn liegen."
        }
        if let r = rule {
            if let u = r.until, u < start.date { return "Die Wiederholung endet vor dem Beginn." }
            // Der Beginn ist das erste Vorkommen (RFC 5545), auch auf einem
            // anderen Wochentag. Nur prüfen, wo der Beginn der Serienbeginn wird.
            let startsSeries = isNew || original?.recurrence == nil
            if startsSeries, r.frequency == .weekly, !r.weekdays.isEmpty, !r.weekdays.contains(start.date.weekday) {
                return "Der Beginn muss auf einen der gewählten Wochentage fallen."
            }
        }
        return nil
    }

    public var canSave: Bool { validationMessage == nil }

    // MARK: - Geltungsbereich

    /// Geltungsbereiche, nach denen beim Sichern gefragt wird; leer heißt:
    /// nicht fragen. Nur, was der Server zulässt (Vertrag §6): Eine neue
    /// Wiederholung geht nicht für „nur dieser“, ein Kalenderwechsel nur für alle.
    public var saveScopes: [EditScope] {
        guard isSeriesOccurrence, hasChanges, let original else { return [] }
        if calendarId != original.fields.calendarId { return [.all] }
        if repetition != original.fields.repetition { return [.thisAndFollowing, .all] }
        return [.this, .thisAndFollowing, .all]
    }

    public var deleteScopes: [EditScope] {
        isSeriesOccurrence ? [.this, .thisAndFollowing, .all] : []
    }

    public var saveLabel: String { isNew ? "Termin anlegen" : "Termin ändern" }

    // MARK: - Absicht

    /// Was beim Sichern passiert. `nil`, wenn nichts zu tun ist, der Entwurf
    /// ungültig ist oder `scope` hier nicht erlaubt ist. Bei Einzelterminen
    /// gilt immer `all`.
    public func saveIntent(scope requested: EditScope = .all) -> EditIntent? {
        guard canSave else { return nil }
        guard let original else {
            let input = EventInput(
                title: effectiveTitle, location: location.trimmed.nilIfEmpty, notes: notes.trimmed.isEmpty ? nil : notes,
                start: startValue, end: endValue, recurrence: recurrenceValue(base: nil),
                alarms: alarmValues.isEmpty ? nil : alarmValues
            )
            return .create(calendarId: calendarId, event: input)
        }
        let scope: EditScope = isSeriesOccurrence ? requested : .all
        if isSeriesOccurrence, !saveScopes.contains(scope) { return nil }
        let before = original.fields
        var c = EventChanges()
        if title.trimmed != before.title.trimmed { c.title = effectiveTitle }
        if location.trimmed != before.location.trimmed {
            c.location = location.trimmed.isEmpty ? .clear : .set(location.trimmed)
        }
        if notes != before.notes { c.notes = notes.trimmed.isEmpty ? .clear : .set(notes) }
        if startValue != Self.startValue(before) || endValue != Self.endValue(before) {
            if scope == .all, isSeriesOccurrence, let seriesStart = original.seriesStart {
                // „Alle“: Die Serie wandert um so viele Tage wie dieses
                // Vorkommen und bekommt dessen Uhrzeit und Dauer. Geschickt
                // wird der Beginn der Serie selbst (ohne `occurrence`).
                let times = Self.seriesTimes(
                    seriesStart: seriesStart, occurrenceDay: before.start.date,
                    newStart: startValue, newEnd: endValue, deviceTimeZone: deviceTimeZone
                )
                c.start = times.start
                c.end = times.end
            } else {
                c.start = startValue
                c.end = endValue
            }
        }
        if alarms != before.alarms { c.alarms = alarmValues }
        if repetition != before.repetition {
            c.recurrence = recurrenceValue(base: original.recurrence).map { .set($0) } ?? .clear
        } else if isAllDay != before.isAllDay, scope != .this, case .rule = repetition {
            // `UNTIL` muss dieselbe Form haben wie der Beginn (Tag oder Zeitpunkt).
            c.recurrence = recurrenceValue(base: original.recurrence).map { .set($0) }
        }
        if calendarId != before.calendarId { c.calendarId = calendarId }
        guard !c.isEmpty else { return nil }
        return .update(
            eventId: original.eventId, scope: scope,
            occurrence: scope == .all ? nil : original.recurrenceId, changes: c
        )
    }

    /// Löschen des Termins (bei Serien im gewählten Geltungsbereich); `nil` bei neuen.
    public func deleteIntent(scope requested: EditScope = .all) -> EditIntent? {
        guard let original else { return nil }
        guard isSeriesOccurrence, requested != .all else { return .delete(eventId: original.eventId) }
        return .delete(eventId: original.eventId, scope: requested, occurrence: original.recurrenceId)
    }

    private var alarmValues: [Alarm] {
        fixedAlarms.map { .at($0) } + alarms.map { .minutesBefore($0) }
    }

    /// Wiederholung für die Mutation. RDATEs und EXDATEs der Serie bleiben,
    /// weil der Server sie mit der neuen Regel überschreibt.
    private func recurrenceValue(base: Recurrence?) -> Recurrence? {
        switch repetition {
        case .none:
            return nil
        case let .custom(text):
            return base ?? Recurrence(rrule: text)
        case let .rule(r):
            let zone = isAllDay ? deviceTimeZone : (timeZoneID.flatMap(TimeZone.init(identifier:)) ?? deviceTimeZone)
            return Recurrence(
                rrule: r.rruleString(allDay: isAllDay, timeZone: zone),
                rdates: base?.rdates ?? [], exdates: base?.exdates ?? []
            )
        }
    }

    /// Beginn und Ende der Serie, wenn ein Vorkommen vom Tag `occurrenceDay`
    /// nach `newStart`/`newEnd` verlegt wird: gleicher Tagesabstand, neue
    /// Uhrzeit, Zone und Dauer.
    static func seriesTimes(
        seriesStart: DateValue, occurrenceDay: LocalDate, newStart: DateValue, newEnd: DateValue,
        deviceTimeZone tz: TimeZone
    ) -> (start: DateValue, end: DateValue) {
        let seriesDay: LocalDate = switch seriesStart {
        case let .date(d): d
        case let .dateTime(l, _, _): l.date
        }
        let start: DateValue
        switch newStart {
        case let .date(d):
            start = .date(seriesDay.adding(days: occurrenceDay.days(until: d)))
        case let .dateTime(l, zone, _):
            var moved = l
            moved.date = seriesDay.adding(days: occurrenceDay.days(until: l.date))
            start = .dateTime(moved, timeZone: zone)
        }
        return (start, OptimisticApply.shift(newEnd, by: newStart, to: start, tz: tz))
    }
}

extension EditScope {
    /// Text im Dialog beim Sichern oder Löschen eines Serientermins.
    public var editorLabel: String {
        switch self {
        case .this: "Nur dieser Termin"
        case .thisAndFollowing: "Dieser und alle folgenden"
        case .all: "Alle Termine"
        }
    }
}

extension LocalDateTime {
    /// Wanduhrzeit plus Sekunden, ohne Zeitzone (Sommerzeit spielt keine Rolle).
    func addingSeconds(_ s: Int) -> LocalDateTime {
        LocalDateTime(instant(in: .gregorianUTC).addingTimeInterval(TimeInterval(s)), calendar: .gregorianUTC)
    }

    /// Abstand in Sekunden Wanduhrzeit.
    func secondsSince(_ other: LocalDateTime) -> Int {
        Int(instant(in: .gregorianUTC).timeIntervalSince(other.instant(in: .gregorianUTC)))
    }
}

private extension String {
    var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
