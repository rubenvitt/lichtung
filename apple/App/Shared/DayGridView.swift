// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// Ansicht B/C: Zeitraster über einen oder mehrere Tage.
struct DayGridView: View {
    @Environment(AppModel.self) private var model
    let start: LocalDate
    let dayCount: Int
    var hourHeight: CGFloat = 48
    var visibleAllDayLanes = 3

    private let gutter: CGFloat = 44

    /// Ganztägige und mindestens 24 Stunden lange Termine stehen oben in der Zeile.
    static func isAllDayRow(_ o: Occurrence) -> Bool {
        o.isAllDay || o.end.timeIntervalSince(o.start) >= 86_400
    }

    var body: some View {
        let days = (0..<dayCount).map { start.adding(days: $0) }
        let occurrences = model.occurrences(from: start, days: dayCount)
        let timed = occurrences.filter { !Self.isAllDayRow($0) }
        let bars = LaneLayout.bars(occurrences.filter(Self.isAllDayRow), days: days, calendar: model.calendar)
        VStack(spacing: 0) {
            header(days)
            if !bars.isEmpty {
                AllDayRow(bars: bars, columns: dayCount, visibleLanes: visibleAllDayLanes, leadingInset: gutter)
            }
            Divider()
            ScrollViewReader { proxy in
                ScrollView(.vertical) {
                    HStack(alignment: .top, spacing: 0) {
                        hourLabels
                        ForEach(days, id: \.self) { day in
                            DayColumn(day: day, blocks: TimelineLayout.blocks(timed, day: day, calendar: model.calendar), hourHeight: hourHeight)
                        }
                    }
                    .frame(height: hourHeight * 24)
                }
                .onAppear { proxy.scrollTo(7, anchor: .top) }
            }
        }
    }

    private func header(_ days: [LocalDate]) -> some View {
        HStack(spacing: 0) {
            Text("KW \(MonthGrid.isoWeek(start))")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .frame(width: gutter)
            ForEach(days, id: \.self) { day in
                let isToday = day == model.today
                VStack(spacing: 0) {
                    Text(QuickEntryPreview.weekdayShort[day.weekday.rawValue - 1]).font(.caption)
                    Text("\(day.day)").font(.headline)
                }
                .foregroundStyle(isToday ? Color.accentColor : Color.primary)
                .frame(maxWidth: .infinity)
                .contentShape(Rectangle())
                .onTapGesture { model.selectedDay = day }
            }
        }
        .padding(.vertical, 4)
    }

    private var hourLabels: some View {
        VStack(spacing: 0) {
            ForEach(0..<24, id: \.self) { hour in
                Text(String(format: "%02d", hour))
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(width: gutter, height: hourHeight, alignment: .topTrailing)
                    .padding(.trailing, 4)
                    .offset(y: -6)
                    .id(hour)
            }
        }
        .frame(width: gutter)
    }
}

struct DayColumn: View {
    @Environment(AppModel.self) private var model
    let day: LocalDate
    let blocks: [TimelineBlock]
    let hourHeight: CGFloat

    var body: some View {
        GeometryReader { geo in
            let width = geo.size.width
            let height = geo.size.height
            ZStack(alignment: .topLeading) {
                ForEach(0..<24, id: \.self) { hour in
                    Rectangle()
                        .fill(Color.secondary.opacity(0.15))
                        .frame(width: width, height: 0.5)
                        .offset(y: CGFloat(hour) * hourHeight)
                }
                ForEach(blocks) { block in
                    EventBlockView(occurrence: block.occurrence)
                        .frame(width: max(0, width * block.width - 2), height: max(14, height * block.height - 1))
                        .offset(x: width * block.leading + 1, y: height * block.top)
                        .onTapGesture { model.selectedOccurrence = block.occurrence }
                        .contextMenu { OccurrenceMenu(occurrence: block.occurrence) }
                }
                if let line = TimelineLayout.nowLine(Date(), day: day, calendar: model.calendar) {
                    Rectangle()
                        .fill(Color.red)
                        .frame(width: width, height: 1.5)
                        .offset(y: height * line)
                        .accessibilityHidden(true)
                }
            }
            .frame(width: width, height: height, alignment: .topLeading)
        }
        .overlay(alignment: .leading) {
            Rectangle().fill(Color.secondary.opacity(0.15)).frame(width: 0.5)
        }
    }
}

struct EventBlockView: View {
    @Environment(AppModel.self) private var model
    let occurrence: Occurrence

    var body: some View {
        let color = model.color(for: occurrence.calendarId)
        HStack(spacing: 0) {
            Rectangle().fill(color).frame(width: 3)
            VStack(alignment: .leading, spacing: 0) {
                Text(occurrence.title).font(.caption.weight(.semibold)).lineLimit(2)
                if let location = occurrence.details.location, !location.isEmpty {
                    Text(location).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            .padding(.horizontal, 4)
            .padding(.vertical, 2)
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(color.opacity(0.18))
        .clipShape(RoundedRectangle(cornerRadius: 4))
        .accessibilityElement(children: .combine)
    }
}

/// Ganztägige Zeile über dem Raster (und Balken im Monatsraster).
struct AllDayRow: View {
    @Environment(AppModel.self) private var model
    let bars: [LaneBar]
    let columns: Int
    let visibleLanes: Int
    var leadingInset: CGFloat = 0
    var laneHeight: CGFloat = 18

    var body: some View {
        let lanes = min(visibleLanes, (bars.map(\.lane).max() ?? -1) + 1)
        let hidden = LaneLayout.overflow(bars, columns: columns, visibleLanes: visibleLanes)
        GeometryReader { geo in
            let columnWidth = (geo.size.width - leadingInset) / CGFloat(max(1, columns))
            ZStack(alignment: .topLeading) {
                ForEach(bars.filter { $0.lane < visibleLanes }) { bar in
                    let color = model.color(for: bar.occurrence.calendarId)
                    // Getrennt berechnet: Als ein Ausdruck sprengt das den Type-Checker (Xcode 26.6, Release).
                    let span = CGFloat(bar.lastColumn - bar.firstColumn + 1)
                    let width: CGFloat = max(0, columnWidth * span - 4)
                    let x: CGFloat = leadingInset + columnWidth * CGFloat(bar.firstColumn) + 2
                    let y: CGFloat = CGFloat(bar.lane) * laneHeight
                    Text(bar.occurrence.title)
                        .font(.caption2.weight(.semibold))
                        .lineLimit(1)
                        .padding(.horizontal, 4)
                        .frame(width: width, height: laneHeight - 2, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 4).fill(color.opacity(0.3)))
                        .offset(x: x, y: y)
                        .onTapGesture { model.selectedOccurrence = bar.occurrence }
                }
                ForEach(0..<columns, id: \.self) { column in
                    if hidden[column] > 0 {
                        Text("+\(hidden[column])")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .offset(x: leadingInset + columnWidth * CGFloat(column) + 4, y: CGFloat(lanes) * laneHeight)
                    }
                }
            }
        }
        .frame(height: CGFloat(lanes) * laneHeight + (hidden.contains { $0 > 0 } ? 14 : 0) + 4)
    }
}
