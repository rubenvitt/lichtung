// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Ansicht D: der ganze Monat, Termine als Balken.
struct MonthView: View {
    @Environment(AppModel.self) private var model
    let year: Int
    let month: Int
    var visibleLanes = 3

    var body: some View {
        let weeks = MonthGrid.weeks(year: year, month: month, fixedRows: true)
        let first = weeks.first!.first!
        let occurrences = model.occurrences(from: first, days: weeks.count * 7)
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                ForEach(QuickEntryPreview.weekdayShort, id: \.self) { wd in
                    Text(wd).font(.caption).foregroundStyle(.secondary).frame(maxWidth: .infinity)
                }
            }
            .padding(.vertical, 4)
            Divider()
            ForEach(weeks, id: \.first) { week in
                MonthWeekRow(days: week, month: month, occurrences: occurrences, visibleLanes: visibleLanes)
                Divider()
            }
        }
    }
}

struct MonthWeekRow: View {
    @Environment(AppModel.self) private var model
    let days: [LocalDate]
    let month: Int
    let occurrences: [Occurrence]
    let visibleLanes: Int

    var body: some View {
        let bars = LaneLayout.bars(occurrences, days: days, calendar: model.calendar)
        VStack(spacing: 2) {
            HStack(spacing: 0) {
                ForEach(days, id: \.self) { day in
                    let isToday = day == model.today
                    Text("\(day.day)")
                        .font(.caption.weight(isToday ? .bold : .regular))
                        .foregroundStyle(isToday ? Color.white : (day.month == month ? Color.primary : Color.secondary))
                        .frame(width: 22, height: 22)
                        .background(Circle().fill(numberFill(day, isToday: isToday)))
                        .frame(maxWidth: .infinity, alignment: .topLeading)
                        .padding(.leading, 2)
                        .contentShape(Rectangle())
                        .onTapGesture { model.selectedDay = day }
                        .modifier(DaySlotContextMenu(day: day)) // LIC-24
                }
            }
            AllDayRow(bars: bars, columns: 7, visibleLanes: visibleLanes, laneHeight: 16)
            Spacer(minLength: 0)
        }
        .frame(maxHeight: .infinity)
        .background { emptyDayAreas }
    }

    /// Heute gefüllt, der gewählte Tag (Ziel für Einfügen, LIC-24) blass.
    private func numberFill(_ day: LocalDate, isToday: Bool) -> Color {
        if isToday { return Color.accentColor }
        if day == model.selectedDay { return Color.accentColor.opacity(0.2) }
        return Color.clear
    }

    /// Mac: freie Fläche je Tag unter den Balken. Klick wählt den Tag und hebt
    /// die Markierung auf, Rechtsklick bietet „Neuer Termin hier“ (LIC-24).
    @ViewBuilder
    private var emptyDayAreas: some View {
        #if os(macOS)
        HStack(spacing: 0) {
            ForEach(days, id: \.self) { day in
                Color.clear
                    .contentShape(Rectangle())
                    .onTapGesture { model.tapEmptyArea(day: day) }
                    .modifier(DaySlotContextMenu(day: day))
            }
        }
        #endif
    }
}

/// Kleiner Monat für Seitenleiste und Jahresansicht; Punkte zeigen belegte Tage.
struct MiniMonthView: View {
    @Environment(AppModel.self) private var model
    let year: Int
    let month: Int
    var showsTitle = true

    var body: some View {
        let weeks = MonthGrid.weeks(year: year, month: month, fixedRows: true)
        let days = weeks.flatMap { $0 }
        let counts = MonthGrid.counts(model.occurrences(from: days[0], days: days.count), days: days, calendar: model.calendar)
        VStack(spacing: 2) {
            if showsTitle {
                Text(MonthGrid.title(year: year, month: month))
                    .font(.caption.weight(.semibold))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            Grid(horizontalSpacing: 2, verticalSpacing: 2) {
                GridRow {
                    ForEach(QuickEntryPreview.weekdayShort, id: \.self) { wd in
                        Text(String(wd.prefix(1))).font(.caption2).foregroundStyle(.secondary)
                    }
                }
                ForEach(weeks, id: \.first) { week in
                    GridRow {
                        ForEach(week, id: \.self) { day in
                            MiniDayCell(day: day, inMonth: day.month == month, events: counts[day] ?? 0)
                        }
                    }
                }
            }
        }
    }
}

struct MiniDayCell: View {
    @Environment(AppModel.self) private var model
    let day: LocalDate
    let inMonth: Bool
    let events: Int

    var body: some View {
        let selected = day == model.selectedDay
        let isToday = day == model.today
        VStack(spacing: 1) {
            Text("\(day.day)")
                .font(.caption2.monospacedDigit().weight(isToday ? .bold : .regular))
                .foregroundStyle(selected ? Color.white : (inMonth ? (isToday ? Color.accentColor : Color.primary) : Color.secondary.opacity(0.6)))
                .frame(width: 20, height: 18)
                .background(Circle().fill(selected ? Color.accentColor : Color.clear))
            Circle()
                .fill(Color.secondary)
                .frame(width: 3, height: 3)
                .opacity(events > 0 && inMonth ? 1 : 0)
        }
        .contentShape(Rectangle())
        .onTapGesture { model.selectedDay = day }
        .accessibilityLabel("\(day.day). \(events) Termine")
    }
}
