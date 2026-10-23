import Foundation

/// Bezieht den Zwischenstand des Parsers auf „jetzt“: aus „Di 14 Uhr“ wird
/// ein konkreter Zeitpunkt.
extension Draft {
    func resolve(
        title: String,
        now: Date,
        calendar cal: Calendar,
        defaultDuration: Int,
        calendars: [CalendarChoice],
        highlights: [Highlight]
    ) -> QuickEntry {
        let today = LocalDate(now, calendar: cal)
        let nowClock = Clock(
            hour: cal.component(.hour, from: now),
            minute: cal.component(.minute, from: now)
        )

        var clock = start
        var endClock = end
        if let part = daypart {
            clock = clock.map(part.adjust) ?? part.defaultClock
            endClock = endClock.map { e in
                // „8-10 abends“: das Ende folgt dem Anfang in den Abend.
                guard let s = clock, e.hour < 12, s.hour >= 12 else { return e }
                return Clock(hour: e.hour + 12, minute: e.minute)
            }
        }
        let isTimed = clock != nil && !allDay
        /// Ist die Uhrzeit heute schon vorbei?
        let passedToday = isTimed && clock!.minutes < nowClock.minutes

        // Tag bestimmen
        var first: LocalDate?
        switch day {
        case .absolute(let p)?:
            first = Self.resolve(p, after: today, allowToday: true)
        case .relativeDays(let n)?:
            first = today.adding(days: n)
        case .relativeMonths(let n)?:
            first = today.adding(months: n)
        case .weekday(let wd, let strict)?:
            first = Self.next(wd, from: today, allowToday: !strict && !passedToday)
        case .week(let offset, let wd)?:
            let monday = today.adding(days: -(today.weekday.rawValue - 1))
            first = monday.adding(days: 7 * offset + ((wd?.rawValue ?? 1) - 1))
        case nil:
            if let rule = recurrence {
                if rule.frequency == .weekly, !rule.weekdays.isEmpty {
                    first = rule.weekdays
                        .map { Self.next($0, from: today, allowToday: !passedToday) }
                        .min()
                } else {
                    first = passedToday ? today.adding(days: 1) : today
                }
            } else if isTimed {
                first = passedToday ? today.adding(days: 1) : today
            } else if allDay {
                first = today
            }
        }

        var last = first
        if let first, let lastDay {
            last = Self.resolve(lastDay, after: first, allowToday: true)
        }

        // Zeitform
        let timing: QuickEntry.Timing
        if let first, let last {
            if isTimed, let clock {
                let startDate = first.date(hour: clock.hour, minute: clock.minute, in: cal)
                var endDate: Date
                if let e = endClock {
                    // Wanduhr: 9–9:30 an einem Tag mit Zeitumstellung bleibt 9–9:30.
                    endDate = Self.wallClock(last, e, cal)
                    if endDate <= startDate {
                        endDate = Self.wallClock(last.adding(days: 1), e, cal)
                    }
                } else {
                    let minutes = durationMinutes ?? defaultDuration
                    let base = last == first ? startDate : last.date(hour: clock.hour, minute: clock.minute, in: cal)
                    endDate = base.addingTimeInterval(TimeInterval(minutes * 60))
                }
                timing = .timed(start: startDate, end: endDate)
            } else {
                timing = .allDay(first: first, last: max(first, last))
            }
        } else {
            timing = .unspecified
        }

        // Wiederholung
        var rule = recurrence
        if let u = until, rule != nil {
            rule!.until = Self.resolve(u, after: first ?? today, allowToday: true)
        }

        // Kalender
        var calendarID: String?
        if let q = calendarQuery?.lowercased() {
            let exact = calendars.filter { $0.name.lowercased() == q }
            let prefix = calendars.filter { $0.name.lowercased().hasPrefix(q) }
            if exact.count == 1 {
                calendarID = exact[0].id
            } else if prefix.count == 1 {
                calendarID = prefix[0].id
            }
        }

        return QuickEntry(
            title: title,
            timing: timing,
            recurrence: rule,
            location: location,
            calendarQuery: calendarQuery,
            calendarID: calendarID,
            highlights: highlights
        )
    }

    static func wallClock(_ day: LocalDate, _ c: Clock, _ cal: Calendar) -> Date {
        if c.hour == 24 { return day.adding(days: 1).startDate(in: cal) }
        return day.date(hour: c.hour, minute: c.minute, in: cal)
    }

    /// Nächster Tag `wd` ab `from` (heute nur mit `allowToday`).
    static func next(_ wd: Weekday, from: LocalDate, allowToday: Bool) -> LocalDate {
        var delta = (wd.rawValue - from.weekday.rawValue + 7) % 7
        if delta == 0, !allowToday { delta = 7 }
        return from.adding(days: delta)
    }

    /// Ergänzt Monat und Jahr so, dass der Tag nicht vor `after` liegt.
    /// Ungültige Tage (31.4.) rutschen auf den Monatsletzten.
    static func resolve(_ p: PartialDate, after: LocalDate, allowToday: Bool) -> LocalDate {
        func clamp(_ y: Int, _ m: Int, _ d: Int) -> LocalDate {
            LocalDate(year: y, month: m, day: min(d, LocalDate.daysInMonth(year: y, month: m)))
        }
        func isAfter(_ c: LocalDate) -> Bool { allowToday ? c >= after : c > after }

        if let m = p.month, let y = p.year { return clamp(y, m, p.day) }
        if let m = p.month {
            let c = clamp(after.year, m, p.day)
            return isAfter(c) ? c : clamp(after.year + 1, m, p.day)
        }
        // nur der Tag: dieser oder nächster Monat
        var probe = LocalDate(year: after.year, month: after.month, day: 1)
        for _ in 0..<13 {
            if LocalDate.isValid(year: probe.year, month: probe.month, day: p.day) {
                let c = LocalDate(year: probe.year, month: probe.month, day: p.day)
                if isAfter(c) { return c }
            }
            probe = probe.adding(months: 1)
        }
        return clamp(after.year, after.month, p.day)
    }
}
