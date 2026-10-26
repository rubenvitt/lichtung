import Foundation

/// Expandiert Termine in Vorkommen für die Anzeige (Spec §4 „Serien“).
///
/// Regeln:
/// - Expandiert wird in Wanduhrzeit der Zone des Serienbeginns: „Mo 9:00
///   Europe/Berlin“ bleibt auch nach der Sommerzeit-Umstellung 9:00 in Berlin.
///   Floating und ganztägige Serien gelten in `deviceTimeZone`.
/// - Dauer jedes Vorkommens = Dauer des Masters. Ganztägig in Tagen, zeitgebunden
///   in Sekunden (absolut, wie Apple Kalender und Google): Ein einstündiger
///   Termin bleibt eine Stunde, auch wenn er über eine Umstellung läuft.
/// - DTSTART ist immer das erste Vorkommen (RFC 5545), RDATE fügt hinzu, EXDATE
///   nimmt weg (auch DTSTART und RDATE).
/// - Overrides ersetzen ihr Vorkommen vollständig. Sie erscheinen nach ihrer
///   eigenen Zeit, egal wo das ursprüngliche Vorkommen lag, und auch dann,
///   wenn ihre `recurrence_id` nicht (mehr) auf die Regel passt.
/// - Eine kaputte oder nicht unterstützte Regel (z. B. `FREQ=HOURLY`,
///   `BYWEEKNO`) ergibt nur DTSTART (plus RDATE und Overrides), keinen Absturz.
public struct SeriesExpander: Sendable {
    public var deviceTimeZone: TimeZone

    /// Schutz gegen Regeln, die nie (oder nur selten) ein Vorkommen ergeben.
    static let maxPeriods = 50_000

    public init(deviceTimeZone: TimeZone) {
        self.deviceTimeZone = deviceTimeZone
    }

    /// Alle Vorkommen von `event`, die sich mit `window` überschneiden,
    /// sortiert nach Beginn. Abgesagte Vorkommen fehlen.
    public func occurrences(of event: Event, in window: DateInterval) -> [Occurrence] {
        guard event.details.status != .cancelled else { return [] }
        guard let recurrence = event.recurrence else {
            let start = event.details.start.instant(deviceTimeZone: deviceTimeZone)
            let end = event.details.end.instant(deviceTimeZone: deviceTimeZone)
            guard Self.overlaps(start, end, window) else { return [] }
            return [Occurrence(eventId: event.id, calendarId: event.calendarId, recurrenceId: nil, details: event.details, start: start, end: end, isModified: false)]
        }

        let series = SeriesFrame(master: event.details, deviceTimeZone: deviceTimeZone)

        // Tagesbereich in Wanduhrzeit der Serie, mit einem Tag Puffer. Vorkommen,
        // die vorher beginnen, aber ins Fenster ragen, deckt die Dauer ab.
        let reach = max(series.duration, Double(series.durationDays) * 86_400)
        let lower = LocalDate(window.start.addingTimeInterval(-reach), calendar: series.calendar).adding(days: -1)
        let upper = LocalDate(window.end, calendar: series.calendar).adding(days: 1)

        var starts: Set<LocalDateTime> = [series.start]
        if let rule = recurrence.rrule.flatMap({ RRule($0) }) {
            starts.formUnion(rule.instances(
                after: series.start, allDay: series.allDay, from: lower, through: upper,
                until: rule.until.map(series.limit), maxPeriods: Self.maxPeriods
            ))
        }
        for r in recurrence.rdates { starts.insert(series.key(r)) }

        // EXDATE: über die Wanduhrzeit der Serie. Ein reines Datum bei einer
        // zeitgebundenen Serie nimmt alle Vorkommen dieses Tages weg.
        let exKeys = Set(recurrence.exdates.map(series.key))
        let exDays: Set<LocalDate> = series.allDay ? [] : Set(recurrence.exdates.compactMap {
            if case let .date(d) = $0 { return d }
            return nil
        })
        starts = starts.filter { !exKeys.contains($0) && !exDays.contains($0.date) }

        // Overrides, bei doppelter `recurrence_id` gilt der letzte.
        var overrides: [LocalDateTime: EventOverride] = [:]
        for o in event.overrides { overrides[series.key(o.recurrenceId)] = o }

        var result: [Occurrence] = []
        for (key, o) in overrides where o.details.status != .cancelled {
            let start = o.details.start.instant(deviceTimeZone: deviceTimeZone)
            let end = o.details.end.instant(deviceTimeZone: deviceTimeZone)
            guard Self.overlaps(start, end, window) else { continue }
            result.append(Occurrence(
                eventId: event.id, calendarId: event.calendarId, recurrenceId: series.value(key),
                details: o.details, start: start, end: end, isModified: true
            ))
        }
        for local in starts where overrides[local] == nil {
            let (details, start, end) = series.occurrence(at: local)
            guard Self.overlaps(start, end, window) else { continue }
            result.append(Occurrence(
                eventId: event.id, calendarId: event.calendarId, recurrenceId: series.value(local),
                details: details, start: start, end: end, isModified: false
            ))
        }
        return result.sorted { ($0.start, $0.end, $0.id) < ($1.start, $1.end, $1.id) }
    }

    /// Vorkommen, die `rrule` vor `occurrence` erzeugt, DTSTART mitgezählt,
    /// RDATE und EXDATE nicht. So zählt `COUNT` (RFC 5545), und so rechnet der
    /// Server beim Teilen einer Serie die Restzahl (`split_series`, LIC-52).
    public func ruleInstances(of rrule: String, seriesStart: DateValue, before occurrence: DateValue) -> Int {
        let series = SeriesFrame(master: EventDetails(title: "", start: seriesStart, end: seriesStart), deviceTimeZone: deviceTimeZone)
        let cut = series.key(occurrence)
        guard series.start < cut else { return 0 }
        guard let rule = RRule(rrule) else { return 1 }
        let later = rule.instances(
            after: series.start, allDay: series.allDay, from: series.start.date, through: cut.date,
            until: rule.until.map(series.limit), maxPeriods: Self.maxPeriods
        )
        return 1 + later.filter { $0 < cut }.count
    }

    /// Überschneidung mit dem Fenster (Ende exklusiv). Ein Termin ohne Dauer
    /// zählt, wenn er in `[window.start, window.end)` beginnt.
    static func overlaps(_ start: Date, _ end: Date, _ window: DateInterval) -> Bool {
        guard start < window.end else { return false }
        return end > window.start || (end <= start && start >= window.start)
    }
}

/// Die festen Größen einer Serie: Zone, Beginn in Wanduhrzeit, Dauer.
private struct SeriesFrame {
    let master: EventDetails
    let allDay: Bool
    /// Zone des Beginns, `nil` bei floating und ganztägig.
    let zoneID: String?
    let calendar: Calendar
    let deviceCalendar: Calendar
    /// DTSTART in Wanduhrzeit (ganztägig: 00:00).
    let start: LocalDateTime
    /// Dauer zeitgebundener Vorkommen in Sekunden.
    let duration: TimeInterval
    /// Dauer ganztägiger Vorkommen in Tagen.
    let durationDays: Int
    /// Das Ende darf in einer anderen Zone liegen (z. B. Flüge).
    let endZoneID: String?
    let endCalendar: Calendar

    init(master: EventDetails, deviceTimeZone: TimeZone) {
        self.master = master
        deviceCalendar = .lichtung(timeZone: deviceTimeZone)
        func calendar(_ id: String?) -> Calendar {
            .lichtung(timeZone: id.flatMap(TimeZone.init(identifier:)) ?? deviceTimeZone)
        }
        switch master.start {
        case let .date(d):
            allDay = true
            zoneID = nil
            start = LocalDateTime(date: d, hour: 0, minute: 0)
        case let .dateTime(l, tz, _):
            allDay = false
            zoneID = tz
            start = l
        }
        self.calendar = calendar(zoneID)
        if case let .dateTime(_, tz, _) = master.end { endZoneID = tz } else { endZoneID = zoneID }
        endCalendar = calendar(endZoneID)

        let s = master.start.instant(deviceTimeZone: deviceTimeZone)
        let e = master.end.instant(deviceTimeZone: deviceTimeZone)
        duration = max(0, e.timeIntervalSince(s))
        if case let .date(sd) = master.start, case let .date(ed) = master.end {
            durationDays = max(0, sd.days(until: ed))
        } else {
            durationDays = Int((duration / 86_400).rounded())
        }
    }

    /// Ein Datumswert (EXDATE, RDATE, RECURRENCE-ID) als Wanduhrzeit der Serie.
    /// Gleiche Zone (oder keine): direkt; andere Zone: über den Zeitpunkt.
    func key(_ value: DateValue) -> LocalDateTime {
        switch value {
        case let .date(d):
            return allDay ? LocalDateTime(date: d, hour: 0, minute: 0)
                : LocalDateTime(date: d, hour: start.hour, minute: start.minute, second: start.second)
        case let .dateTime(l, tz, _):
            if allDay {
                let d = tz == nil ? l.date : LocalDate(value.instant(deviceTimeZone: deviceCalendar.timeZone), calendar: deviceCalendar)
                return LocalDateTime(date: d, hour: 0, minute: 0)
            }
            if tz == nil || tz == zoneID { return l }
            return LocalDateTime(value.instant(deviceTimeZone: deviceCalendar.timeZone), calendar: calendar)
        }
    }

    /// `UNTIL` in Wanduhrzeit der Serie (inklusive).
    func limit(_ until: RRule.Until) -> LocalDateTime {
        switch until {
        case let .date(d): return LocalDateTime(date: d, hour: 23, minute: 59, second: 59)
        case let .utc(t): return LocalDateTime(t, calendar: calendar)
        case let .local(l): return l
        }
    }

    /// Ursprünglicher Beginn in der Form von DTSTART (ohne `utc`).
    func value(_ local: LocalDateTime) -> DateValue {
        allDay ? .date(local.date) : .dateTime(local, timeZone: zoneID)
    }

    func occurrence(at local: LocalDateTime) -> (EventDetails, Date, Date) {
        var details = master
        if allDay {
            let endDate = local.date.adding(days: durationDays)
            details.start = .date(local.date)
            details.end = .date(endDate)
            return (details, local.date.startDate(in: deviceCalendar), endDate.startDate(in: deviceCalendar))
        }
        let s = local.instant(in: calendar)
        let e = s.addingTimeInterval(duration)
        details.start = .dateTime(local, timeZone: zoneID)
        details.end = .dateTime(LocalDateTime(e, calendar: endCalendar), timeZone: endZoneID)
        return (details, s, e)
    }
}
