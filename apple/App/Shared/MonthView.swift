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
                ForEach(Array(QuickEntryPreview.weekdayShort.enumerated()), id: \.offset) { index, wd in
                    Text(wd)
                        .font(.caption)
                        .foregroundStyle(Theme.text2)
                        .frame(maxWidth: .infinity)
                }
            }
            .padding(.vertical, 6)
            Rectangle().fill(Theme.line).frame(height: 0.5)
            ForEach(weeks, id: \.first) { week in
                MonthWeekRow(days: week, month: month, occurrences: occurrences, visibleLanes: visibleLanes)
                Rectangle().fill(Theme.line).frame(height: 0.5)
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
                    Text(verbatim: "\(day.day)")
                        .font(Theme.digits(.caption, weight: isToday ? .bold : .medium))
                        .foregroundStyle(numberColor(day, isToday: isToday))
                        .frame(width: 24, height: 24)
                        .background(Circle().fill(numberFill(day, isToday: isToday)))
                        .overlay {
                            // Gewählter Tag (Ziel für Einfügen, LIC-24): Ring.
                            if day == model.selectedDay && !isToday {
                                Circle().strokeBorder(Theme.light, lineWidth: 1.5)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .topLeading)
                        .padding(.leading, 4)
                        .padding(.top, 3)
                        .contentShape(Rectangle())
                        .onTapGesture { model.selectedDay = day }
                        .modifier(DaySlotContextMenu(day: day)) // LIC-24
                }
            }
            AllDayRow(bars: bars, columns: 7, visibleLanes: visibleLanes, laneHeight: 16)
            Spacer(minLength: 0)
        }
        .frame(maxHeight: .infinity)
        .background { dayWash }
        .background { emptyDayAreas }
    }

    /// Der gewählte Tag (Ziel für Einfügen, LIC-24) blass hinterlegt.
    private func numberFill(_ day: LocalDate, isToday: Bool) -> Color {
        if day == model.selectedDay && !isToday { return Theme.light.opacity(0.12) }
        return Color.clear
    }

    /// Heute in Licht, Wochenende und fremde Monate in Text 2.
    private func numberColor(_ day: LocalDate, isToday: Bool) -> Color {
        if isToday { return Theme.light }
        if day.month != month || day.weekday.rawValue >= 6 { return Theme.text2 }
        return Theme.text1
    }

    /// Tage außerhalb des Monats gedämpft, dazu feine Spaltenlinien.
    private var dayWash: some View {
        HStack(spacing: 0) {
            ForEach(days, id: \.self) { day in
                Rectangle()
                    .fill(day.month != month ? Theme.line : Color.clear)
                    .overlay(alignment: .leading) {
                        Rectangle().fill(Theme.line).frame(width: 0.5)
                    }
            }
        }
        .allowsHitTesting(false)
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
                MonthTitle(year: year, month: month, style: .subheadline)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.bottom, 4)
            }
            Grid(horizontalSpacing: 2, verticalSpacing: 2) {
                GridRow {
                    ForEach(QuickEntryPreview.weekdayShort, id: \.self) { wd in
                        Text(String(wd.prefix(1)))
                            .font(.system(size: 9, weight: .bold))
                            .foregroundStyle(Theme.text2)
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
            Text(verbatim: "\(day.day)")
                .font(Theme.digits(.caption2, weight: isToday || selected ? .bold : .regular))
                .foregroundStyle(selected ? Theme.onLight : (inMonth ? (isToday ? Theme.light : Theme.text1) : Theme.text2))
                .frame(width: 22, height: 20)
                .background(Circle().fill(selected ? Theme.light : Color.clear))
            Circle()
                .fill(selected ? Theme.light : Theme.text2)
                .frame(width: 3, height: 3)
                .opacity(events > 0 && inMonth ? 1 : 0)
        }
        .contentShape(Rectangle())
        .onTapGesture { model.selectedDay = day }
        .accessibilityLabel(isToday ? "Heute, \(day.day). \(events) Termine" : "\(day.day). \(events) Termine")
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
