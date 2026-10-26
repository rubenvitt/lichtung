// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI
#if os(macOS)
import AppKit
#elseif os(iOS)
import UIKit
#endif

/// Ansicht B/C: Zeitraster über einen oder mehrere Tage.
///
/// Ziehen (LIC-22): In leerer Fläche ziehen legt einen Termin an, einen Block
/// ziehen verschiebt ihn (in der Woche auch in andere Tage), der Griff unten
/// ändert die Länge. Am Mac direkt, am iPhone erst lange drücken, damit das
/// Scrollen frei bleibt. Gerechnet wird in `GridInteraction`.
struct DayGridView: View {
    @Environment(AppModel.self) private var model
    let start: LocalDate
    let dayCount: Int
    var hourHeight: CGFloat = 48
    var visibleAllDayLanes = 3

    private let gutter: CGFloat = 44
    /// Breite der Spalte für die zweite Zeitzone (LIC-42).
    private let zoneGutter: CGFloat = 36

    /// Breite links vor den Tagesspalten.
    private var leadingWidth: CGFloat { gutter + (model.secondaryTimeZone == nil ? 0 : zoneGutter) }

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
                AllDayRow(bars: bars, columns: dayCount, visibleLanes: visibleAllDayLanes, leadingInset: leadingWidth)
            }
            Divider()
            ScrollViewReader { proxy in
                ScrollView(.vertical) {
                    HStack(alignment: .top, spacing: 0) {
                        if let zone = model.secondaryTimeZone {
                            secondaryLabels(zone)
                        }
                        hourLabels
                        ForEach(days, id: \.self) { day in
                            DayColumn(
                                day: day,
                                index: start.days(until: day),
                                dayCount: dayCount,
                                blocks: TimelineLayout.blocks(timed, day: day, calendar: model.calendar),
                                hourHeight: hourHeight
                            )
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
            if let zone = model.secondaryTimeZone {
                Text(TimeZoneDisplay.cityName(for: zone))
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                    .frame(width: zoneGutter)
            }
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

    /// Immer 24 Zeilen: Das Raster hat eine Wanduhr-Achse (`WallClockAxis`),
    /// auch an Umstellungstagen (LIC-41).
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

    /// Zweite Zeitzone (LIC-42), gedämpft links neben den Stunden. Gilt für
    /// den ersten sichtbaren Tag; in der Woche kann sich der Abstand an einem
    /// Umstellungstag einer der Zonen ändern.
    private func secondaryLabels(_ zone: TimeZone) -> some View {
        let labels = TimelineLayout.secondaryHourLabels(for: zone, home: model.timeZone, day: start)
        return VStack(spacing: 0) {
            ForEach(0..<24, id: \.self) { hour in
                Text(labels[hour] ?? "")
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                    .frame(width: zoneGutter, height: hourHeight, alignment: .topTrailing)
                    .offset(y: -6)
            }
        }
        .frame(width: zoneGutter)
        .accessibilityHidden(true)
    }
}

/// Laufender Zug in einer Tagesspalte. Punkte und Strecken in der Spalte.
enum GridDrag: Equatable {
    case create(fromY: CGFloat, toY: CGFloat)
    case move(id: String, translation: CGSize)
    case resize(id: String, dy: CGFloat)

    var occurrenceId: String? {
        switch self {
        case .create: nil
        case let .move(id, _), let .resize(id, _): id
        }
    }
}

struct DayColumn: View {
    @Environment(AppModel.self) private var model
    let day: LocalDate
    /// Lage der Spalte im sichtbaren Bereich, für Verschieben in Nachbartage.
    var index = 0
    var dayCount = 1
    let blocks: [TimelineBlock]
    let hourHeight: CGFloat

    @GestureState private var drag: GridDrag? = nil

    var body: some View {
        GeometryReader { geo in
            let width = geo.size.width
            let height = geo.size.height
            ZStack(alignment: .topLeading) {
                // Leere Fläche: hier beginnt das Anlegen per Ziehen.
                Color.clear
                    .frame(width: width, height: height)
                    .contentShape(Rectangle())
                    .gesture(createGesture(height: height))
                ForEach(0..<24, id: \.self) { hour in
                    Rectangle()
                        .fill(Color.secondary.opacity(0.15))
                        .frame(width: width, height: 0.5)
                        .offset(y: CGFloat(hour) * hourHeight)
                        .allowsHitTesting(false)
                }
                // Reisezeit (LIC-42) unter den Terminen.
                ForEach(blocks) { block in
                    if let travel = TravelTime.travelBlock(for: block.occurrence),
                       let frame = GridInteraction.frame(of: travel, day: day, calendar: model.calendar),
                       frame.height > 0 {
                        TravelBlockView(
                            color: model.color(for: block.occurrence.calendarId),
                            minutes: block.occurrence.details.travelTimeMinutes ?? 0
                        )
                        .frame(width: max(0, width * block.width - 2), height: height * frame.height)
                        .offset(x: width * block.leading + 1, y: height * frame.top)
                        .allowsHitTesting(false)
                    }
                }
                ForEach(blocks) { block in
                    blockView(block, width: width, height: height)
                }
                preview(width: width, height: height)
                if let line = TimelineLayout.nowLine(Date(), day: day, calendar: model.calendar) {
                    Rectangle()
                        .fill(Color.red)
                        .frame(width: width, height: 1.5)
                        .offset(y: height * line)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .frame(width: width, height: height, alignment: .topLeading)
        }
        .overlay(alignment: .leading) {
            Rectangle().fill(Color.secondary.opacity(0.15)).frame(width: 0.5)
        }
        // Beim Ziehen in einen Nachbartag liegt die Vorschau über den anderen Spalten.
        .zIndex(drag == nil ? 0 : 1)
        #if os(iOS)
        .sensoryFeedback(.selection, trigger: drag == nil)
        #endif
    }

    // MARK: Blöcke

    private func blockView(_ block: TimelineBlock, width: CGFloat, height: CGFloat) -> some View {
        let occ = block.occurrence
        let draggable = model.canDrag(occ)
        let isDragged = drag?.occurrenceId == occ.id
        return EventBlockView(occurrence: occ)
            .overlay(alignment: .bottom) {
                if draggable && !block.continuesAfter {
                    ResizeHandle()
                        .gesture(resizeGesture(occ, height: height))
                }
            }
            .opacity(isDragged ? 0.35 : 1)
            .frame(width: max(0, width * block.width - 2), height: max(14, height * block.height - 1))
            .offset(x: width * block.leading + 1, y: height * block.top)
            .onTapGesture { model.selectedOccurrence = occ }
            .gesture(moveGesture(occ, width: width, height: height), including: draggable ? .all : .subviews)
            .modifier(BlockMenu(occurrence: occ, draggable: draggable))
    }

    // MARK: Gesten

    /// Anlegen: Mac direkt ziehen, iPhone lange drücken und dann ziehen.
    private func createGesture(height: CGFloat) -> some Gesture {
        #if os(iOS)
        return LongPressGesture(minimumDuration: 0.4)
            .sequenced(before: DragGesture(minimumDistance: 0))
            .updating($drag) { value, state, _ in
                if case .second(true, let d?) = value {
                    state = .create(fromY: d.startLocation.y, toY: d.location.y)
                }
            }
            .onEnded { value in
                if case .second(true, let d?) = value {
                    commitCreate(fromY: d.startLocation.y, toY: d.location.y, height: height)
                }
            }
        #else
        return DragGesture(minimumDistance: 4)
            .updating($drag) { value, state, _ in
                state = .create(fromY: value.startLocation.y, toY: value.location.y)
            }
            .onEnded { value in
                commitCreate(fromY: value.startLocation.y, toY: value.location.y, height: height)
            }
        #endif
    }

    /// Verschieben: Mac direkt ziehen, iPhone lange drücken und dann ziehen.
    /// Ein kurzer Tap bleibt beim `onTapGesture` und öffnet das Detail.
    private func moveGesture(_ occ: Occurrence, width: CGFloat, height: CGFloat) -> some Gesture {
        let id = occ.id
        #if os(iOS)
        return LongPressGesture(minimumDuration: 0.3)
            .sequenced(before: DragGesture(minimumDistance: 0))
            .updating($drag) { value, state, _ in
                if case .second(true, let d) = value {
                    state = .move(id: id, translation: d?.translation ?? .zero)
                }
            }
            .onEnded { value in
                if case .second(true, let d?) = value {
                    commitMove(occ, translation: d.translation, width: width, height: height)
                }
            }
        #else
        return DragGesture(minimumDistance: 4)
            .updating($drag) { value, state, _ in
                state = .move(id: id, translation: value.translation)
            }
            .onEnded { value in
                commitMove(occ, translation: value.translation, width: width, height: height)
            }
        #endif
    }

    /// Länge ändern am Griff: auf beiden Plattformen direkt ziehen.
    private func resizeGesture(_ occ: Occurrence, height: CGFloat) -> some Gesture {
        let id = occ.id
        return DragGesture(minimumDistance: 2)
            .updating($drag) { value, state, _ in
                state = .resize(id: id, dy: value.translation.height)
            }
            .onEnded { value in
                let target = resizedInterval(occ, dy: value.translation.height, height: height)
                Task { await model.move(occ, start: target.start, end: target.end) }
            }
    }

    // MARK: Rechnen (GridInteraction)

    private func creationInterval(fromY: CGFloat, toY: CGFloat, height: CGFloat) -> DateInterval {
        GridInteraction.creation(day: day, fromY: Double(fromY), toY: Double(toY), columnHeight: Double(height), calendar: model.calendar)
    }

    private func movedInterval(_ occ: Occurrence, translation: CGSize, width: CGFloat, height: CGFloat) -> DateInterval {
        let offset = GridInteraction.dayOffset(forDistance: Double(translation.width), columnWidth: Double(width), column: index, columns: dayCount)
        let delta = GridInteraction.minutes(forDistance: Double(translation.height), columnHeight: Double(height))
        return GridInteraction.moved(start: occ.start, end: occ.end, deltaMinutes: delta, dayOffset: offset, calendar: model.calendar)
    }

    private func resizedInterval(_ occ: Occurrence, dy: CGFloat, height: CGFloat) -> DateInterval {
        let delta = GridInteraction.minutes(forDistance: Double(dy), columnHeight: Double(height))
        return GridInteraction.resized(start: occ.start, end: occ.end, day: day, deltaMinutes: delta, calendar: model.calendar)
    }

    private func commitCreate(fromY: CGFloat, toY: CGFloat, height: CGFloat) {
        let target = creationInterval(fromY: fromY, toY: toY, height: height)
        Task { await model.createTimed(start: target.start, end: target.end) }
    }

    private func commitMove(_ occ: Occurrence, translation: CGSize, width: CGFloat, height: CGFloat) {
        let target = movedInterval(occ, translation: translation, width: width, height: height)
        Task { await model.move(occ, start: target.start, end: target.end) }
    }

    // MARK: Vorschau

    /// Vorschau zum laufenden Zug: Zeitraum, Spaltenversatz, Kalender (für die Farbe).
    private struct PreviewTarget {
        var interval: DateInterval
        var columnOffset: Int
        var calendarId: String?
    }

    private func previewTarget(width: CGFloat, height: CGFloat) -> PreviewTarget? {
        guard let drag else { return nil }
        switch drag {
        case let .create(fromY, toY):
            return PreviewTarget(interval: creationInterval(fromY: fromY, toY: toY, height: height), columnOffset: 0, calendarId: nil)
        case let .move(id, translation):
            guard let occ = blocks.first(where: { $0.occurrence.id == id })?.occurrence else { return nil }
            let target = movedInterval(occ, translation: translation, width: width, height: height)
            // Gezeichnet wird in der Spalte, in der der neue Beginn liegt.
            let startDay = LocalDate(max(target.start, day.startDate(in: model.calendar)), calendar: model.calendar)
            let offset = day.days(until: startDay)
            guard (0..<dayCount).contains(index + offset) else { return nil }
            return PreviewTarget(interval: target, columnOffset: offset, calendarId: occ.calendarId)
        case let .resize(id, dy):
            guard let occ = blocks.first(where: { $0.occurrence.id == id })?.occurrence else { return nil }
            return PreviewTarget(interval: resizedInterval(occ, dy: dy, height: height), columnOffset: 0, calendarId: occ.calendarId)
        }
    }

    @ViewBuilder
    private func preview(width: CGFloat, height: CGFloat) -> some View {
        if let target = previewTarget(width: width, height: height),
           let frame = GridInteraction.frame(of: target.interval, day: day.adding(days: target.columnOffset), calendar: model.calendar) {
            let color = target.calendarId.map { model.color(for: $0) } ?? Color.accentColor
            RoundedRectangle(cornerRadius: 4)
                .fill(color.opacity(0.3))
                .overlay {
                    RoundedRectangle(cornerRadius: 4).strokeBorder(color, lineWidth: 1)
                }
                .overlay(alignment: .topLeading) {
                    Text(GridInteraction.timeText(target.interval, calendar: model.calendar))
                        .font(.caption2.monospacedDigit().weight(.semibold))
                        .lineLimit(1)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 2)
                }
                .frame(width: max(0, width - 2), height: max(14, height * frame.height))
                .offset(x: CGFloat(target.columnOffset) * width + 1, y: height * frame.top)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }
}

/// Griff am unteren Rand eines Blocks zum Länge-Ändern.
private struct ResizeHandle: View {
    var body: some View {
        Color.clear
            .frame(maxWidth: .infinity)
            .frame(height: 8)
            .contentShape(Rectangle())
            .overlay {
                Capsule()
                    .fill(Color.secondary.opacity(0.5))
                    .frame(width: 16, height: 3)
            }
            #if os(macOS)
            .onHover { inside in
                if inside { NSCursor.resizeUpDown.push() } else { NSCursor.pop() }
            }
            #endif
            .accessibilityHidden(true)
    }
}

/// Kontextmenü am Block. Am iPhone ist langes Drücken bei ziehbaren Terminen
/// das Aufnehmen zum Verschieben, dort gibt es das Menü nur bei den übrigen.
private struct BlockMenu: ViewModifier {
    let occurrence: Occurrence
    let draggable: Bool

    @ViewBuilder
    func body(content: Content) -> some View {
        #if os(iOS)
        if draggable {
            content
        } else {
            content.contextMenu { OccurrenceMenu(occurrence: occurrence) }
        }
        #else
        content.contextMenu { OccurrenceMenu(occurrence: occurrence) }
        #endif
    }
}

/// Reisezeit vor einem Termin: gedämpft und schraffiert (LIC-42).
private struct TravelBlockView: View {
    let color: Color
    let minutes: Int

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                RoundedRectangle(cornerRadius: 4).fill(color.opacity(0.08))
                Canvas { context, size in
                    var path = Path()
                    var x = -size.height
                    while x < size.width {
                        path.move(to: CGPoint(x: x, y: size.height))
                        path.addLine(to: CGPoint(x: x + size.height, y: 0))
                        x += 6
                    }
                    context.stroke(path, with: .color(color.opacity(0.3)), lineWidth: 1)
                }
                if geo.size.height >= 16 {
                    Label(TravelTime.label(minutes: minutes), systemImage: "car.fill")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 1)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 4))
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Reisezeit \(TravelTime.label(minutes: minutes))")
    }
}

struct EventBlockView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL
    let occurrence: Occurrence

    var body: some View {
        let color = model.color(for: occurrence.calendarId)
        HStack(alignment: .top, spacing: 0) {
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
            if let link = VideoLinkDetector.detect(in: occurrence.details) {
                joinButton(link)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(color.opacity(0.18))
        .clipShape(RoundedRectangle(cornerRadius: 4))
        .accessibilityElement(children: .combine)
    }

    // MARK: Beitreten (LIC-42)

    /// Kleiner Knopf, solange `VideoLinkDetector.isJoinable` gilt (ab zehn
    /// Minuten vorher bis zum Ende). Jede Minute neu geprüft.
    private func joinButton(_ link: VideoLink) -> some View {
        TimelineView(.everyMinute) { context in
            if VideoLinkDetector.isJoinable(occurrence, now: context.date) {
                Button {
                    join(link)
                } label: {
                    Image(systemName: "video.fill")
                        .font(.caption2)
                        .padding(3)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
                .help("Beitreten (\(link.provider.displayName))")
                .accessibilityLabel("Beitreten (\(link.provider.displayName))")
            }
        }
    }

    /// Erst die Anbieter-App (`appURL`), wenn sie sich nicht öffnen lässt, der Link.
    /// Die App-Adresse geht direkt an das System, weil nur dessen Antwort
    /// sagt, ob das Schema (`zoommtg:`, `msteams:`) einen Abnehmer hat.
    private func join(_ link: VideoLink) {
        let web = link.joinURL
        guard let app = link.appURL else {
            openURL(web)
            return
        }
        #if os(macOS)
        if !NSWorkspace.shared.open(app) { openURL(web) }
        #else
        Task {
            let opened = await UIApplication.shared.open(app)
            if !opened { openURL(web) }
        }
        #endif
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
                    Text(bar.occurrence.title)
                        .font(.caption2.weight(.semibold))
                        .lineLimit(1)
                        .padding(.horizontal, 4)
                        .frame(width: max(0, columnWidth * CGFloat(bar.lastColumn - bar.firstColumn + 1) - 4), height: laneHeight - 2, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 4).fill(color.opacity(0.3)))
                        .offset(x: leadingInset + columnWidth * CGFloat(bar.firstColumn) + 2, y: CGFloat(bar.lane) * laneHeight)
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
