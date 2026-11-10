import LichtungKit
import SwiftUI
#if os(macOS)
import AppKit // NSCursor am Griff
#endif

/// Ansicht B/C: Zeitraster über einen oder mehrere Tage.
///
/// Ziehen (LIC-22): In leerer Fläche ziehen legt einen Termin an, einen Block
/// ziehen verschiebt ihn (in der Woche auch in andere Tage), der Griff unten
/// ändert die Länge. Am Mac direkt, am iPhone erst lange drücken, damit das
/// Scrollen frei bleibt. Gerechnet wird in `GridInteraction`.
///
/// Menü am iPhone (LIC-54): Ein aufgenommener Block, der ohne Bewegung
/// losgelassen wird, öffnet das Menü des Termins als Popover
/// (`OccurrenceMenuPopover`); bewegt ihn der Finger, wird verschoben.
/// Auf leerer Fläche öffnet Halten ohne Bewegung ein Menü mit „Hier
/// einfügen“ (`SlotMenuPopover`, LIC-58), aber nur, wenn eine Kopie vorliegt;
/// sonst und beim Ziehen bleibt es beim Anlegen.
///
/// Feinschliff (LIC-47): Nach dem Aufziehen und nach einem Doppelklick (Mac)
/// öffnet der Editor mit dem neuen Termin. Vorkommen einer Serie fragen vor
/// dem Verschieben nach dem Geltungsbereich. Am oberen und unteren Rand
/// scrollt das Raster beim Ziehen mit. Die Vorschau bleibt stehen, bis der
/// neue Stand da ist.
///
/// Fokus-Tag (Spec §5): In der Woche ist `model.focusDay` breiter
/// (`ColumnWidths`). Kopf, ganztägige Zeile, Tagesspalten und das Ziehen in
/// andere Tage rechnen mit derselben Instanz.
struct DayGridView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let start: LocalDate
    let dayCount: Int
    /// Darunter ergeben 15 Minuten keine lesbare Zeile mehr (Spec §5).
    static let minimumHourHeight: CGFloat = 56
    var hourHeight: CGFloat = DayGridView.minimumHourHeight
    var visibleAllDayLanes = 3

    private let gutter: CGFloat = 48
    /// Breite der Spalte für die zweite Zeitzone (LIC-42).
    private let zoneGutter: CGFloat = 36
    /// Sichtbarer Ausschnitt und automatisches Scrollen beim Ziehen (LIC-47).
    @State private var scroller = GridScroller()

    /// Breite links vor den Tagesspalten.
    private var leadingWidth: CGFloat { gutter + (model.secondaryTimeZone == nil ? 0 : zoneGutter) }

    /// Spaltenbreiten; in der Tagesansicht eine Spalte ohne Fokus.
    private var widths: ColumnWidths {
        ColumnWidths(columns: dayCount, focus: dayCount > 1 ? start.days(until: model.focusDay) : nil)
    }

    /// In der Tagesansicht ist der eine Tag der Fokus-Tag.
    private func isFocus(_ day: LocalDate) -> Bool {
        dayCount == 1 || day == model.focusDay
    }

    /// Ganztägige und mindestens 24 Stunden lange Termine stehen oben in der Zeile.
    static func isAllDayRow(_ o: Occurrence) -> Bool {
        o.isAllDay || o.end.timeIntervalSince(o.start) >= 86_400
    }

    var body: some View {
        let days = (0..<dayCount).map { start.adding(days: $0) }
        let occurrences = model.occurrences(from: start, days: dayCount)
        let timed = occurrences.filter { !Self.isAllDayRow($0) }
        let bars = LaneLayout.bars(occurrences.filter(Self.isAllDayRow), days: days, calendar: model.calendar)
        // Zweite Zeitzone: links der erste Tag, abweichende Tage in ihrer Spalte (LIC-47).
        let zoneLabels: SecondaryZoneLabels? = model.secondaryTimeZone.map {
            TimelineLayout.secondaryHourLabels(for: $0, home: model.timeZone, days: days)
        }
        let widths = self.widths
        GeometryReader { geo in
            let daysWidth: CGFloat = max(0, geo.size.width - leadingWidth)
            VStack(spacing: 0) {
                header(days, widths: widths, daysWidth: daysWidth)
                if !bars.isEmpty {
                    AllDayRow(bars: bars, columns: dayCount, visibleLanes: visibleAllDayLanes, leadingInset: leadingWidth, widths: widths)
                }
                Divider()
                grid(days, timed: timed, zoneLabels: zoneLabels, widths: widths, daysWidth: daysWidth)
            }
            // Nur wenn sich die Breiten ändern; Blättern in den gleichen Wochentag gleitet nicht.
            .animation(reduceMotion ? nil : .snappy(duration: 0.3), value: widths)
        }
        .background(Theme.shade)
    }

    private func grid(_ days: [LocalDate], timed: [Occurrence], zoneLabels: SecondaryZoneLabels?,
                      widths: ColumnWidths, daysWidth: CGFloat) -> some View {
        ScrollView(.vertical) {
            HStack(alignment: .top, spacing: 0) {
                if let zoneLabels {
                    secondaryLabels(zoneLabels.gutter)
                }
                hourLabels(showsNow: days.contains(model.today))
                ForEach(days, id: \.self) { day in
                    let index = start.days(until: day)
                    let focus = isFocus(day)
                    DayColumn(
                        day: day,
                        index: index,
                        dayCount: dayCount,
                        layout: TimelineLayout.day(timed, day: day, calendar: model.calendar, maxColumns: focus ? 5 : 3),
                        isFocus: focus,
                        widths: widths,
                        daysX: leadingWidth,
                        hourHeight: hourHeight,
                        scroller: scroller,
                        zoneLabels: zoneLabels?.perDay[index]
                    )
                    .frame(width: daysWidth * CGFloat(widths.fractions[index]))
                }
            }
            .frame(height: hourHeight * 24)
        }
        .scrollPosition($scroller.position)
        .onScrollGeometryChange(for: GridViewport.self) { geometry in
            GridViewport(
                offset: geometry.contentOffset.y,
                height: geometry.containerSize.height,
                contentHeight: geometry.contentSize.height
            )
        } action: { _, viewport in
            scroller.viewport = viewport
        }
        // Gesten in den Spalten messen im sichtbaren Ausschnitt, damit
        // der Rand beim automatischen Scrollen fest bleibt.
        .coordinateSpace(.named(GridScroller.viewportSpace))
        // Über dieselbe `ScrollPosition` wie das Mitscrollen; `ScrollViewReader.scrollTo`
        // kam gegen die gebundene Position nicht an (Raster blieb bei 0 Uhr).
        .onAppear { scroller.position.scrollTo(y: hourHeight * 7) }
    }

    /// Kopf: Fokus-Tag ausgeschrieben und groß, übrige „Mo 9“. Die KW steht in der Toolbar.
    private func header(_ days: [LocalDate], widths: ColumnWidths, daysWidth: CGFloat) -> some View {
        HStack(alignment: .lastTextBaseline, spacing: 0) {
            if let zone = model.secondaryTimeZone {
                Text(TimeZoneDisplay.cityName(for: zone))
                    .font(.caption2)
                    .foregroundStyle(Theme.text3)
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                    .frame(width: zoneGutter)
            }
            Color.clear.frame(width: gutter, height: 0)
            ForEach(days, id: \.self) { day in
                DayHeaderCell(
                    day: day,
                    isToday: day == model.today,
                    isSelected: day == model.selectedDay,
                    isWeekend: day.weekday.rawValue >= 6,
                    isFocus: isFocus(day),
                    focusSize: focusHeaderSize
                )
                    .frame(width: daysWidth * CGFloat(widths.fractions[start.days(until: day)]), alignment: .leading)
                    .background { selectedDayMarker(day) }
                    .contentShape(Rectangle())
                    .onTapGesture { model.focus(day) }
            }
        }
        .padding(.top, Self.headerTopPadding)
        .padding(.bottom, 8)
    }

    /// Mac: Klicks im Band bis etwa 38 pt unter der Toolbar erreichen den Inhalt
    /// nicht (macOS 26, `NavigationSplitView`); so liegen die schmalen Tage darunter.
    private static var headerTopPadding: CGFloat {
        #if os(macOS)
        26
        #else
        4
        #endif
    }

    /// Fokus-Tag im Kopf: am iPhone in der Woche kleiner, sonst 26 pt.
    private var focusHeaderSize: CGFloat {
        #if os(iOS)
        dayCount > 1 ? 20 : 26
        #else
        26
        #endif
    }

    /// Mac: gewählter Tag der Woche (Ziel für Einfügen und „Neuer Termin“, LIC-24).
    /// Am Fokus-Tag sagt das schon der große Kopf.
    @ViewBuilder
    private func selectedDayMarker(_ day: LocalDate) -> some View {
        #if os(macOS)
        if dayCount > 1 && day == model.selectedDay && day != model.focusDay {
            RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Theme.light.opacity(0.08))
                .padding(.horizontal, 2)
        }
        #endif
    }

    /// Immer 24 Zeilen: Das Raster hat eine Wanduhr-Achse (`WallClockAxis`),
    /// auch an Umstellungstagen (LIC-41). Ist heute zu sehen, steht die
    /// Uhrzeit der Jetzt-Linie in Sonne auf ihrer Höhe und verdrängt die
    /// Stundenzahl im Abstand unter 20 Minuten.
    private func hourLabels(showsNow: Bool) -> some View {
        TimelineView(.everyMinute) { context in
            let now: Double? = showsNow ? TimelineLayout.nowLine(context.date, day: model.today, calendar: model.calendar) : nil
            let nowMinutes: Double? = now.map { $0 * WallClockAxis.minutesPerDay }
            ZStack(alignment: .topTrailing) {
                VStack(spacing: 0) {
                    ForEach(0..<24, id: \.self) { hour in
                        // 0 Uhr oben am Rand bliebe abgeschnitten.
                        let nearNow = nowMinutes.map { abs($0 - Double(hour * 60)) < 20 } ?? false
                        Text(hour == 0 || nearNow ? "" : String(format: "%02d:00", hour))
                            .font(Theme.digits(.caption2, weight: .medium))
                            .foregroundStyle(Theme.text3)
                            .lineLimit(1)
                            .minimumScaleFactor(0.7)
                            .frame(width: gutter - 6, height: hourHeight, alignment: .topTrailing)
                            .padding(.trailing, 6)
                            .offset(y: -7)
                    }
                }
                if let now {
                    Text(Self.clockText(context.date, calendar: model.calendar))
                        .font(Theme.digits(.caption2, weight: .semibold))
                        .foregroundStyle(Theme.sun)
                        .lineLimit(1)
                        .padding(.trailing, 6)
                        .offset(y: hourHeight * 24 * CGFloat(now) - 7)
                        .accessibilityHidden(true)
                }
            }
        }
        .frame(width: gutter)
    }

    /// „10:40“
    static func clockText(_ date: Date, calendar: Calendar) -> String {
        let c = calendar.dateComponents([.hour, .minute], from: date)
        return String(format: "%02d:%02d", c.hour ?? 0, c.minute ?? 0)
    }

    /// Zweite Zeitzone (LIC-42), gedämpft links neben den Stunden. Gilt für
    /// den ersten sichtbaren Tag; Tage, an denen sich der Abstand ändert,
    /// beschriften ihre abweichenden Zeilen selbst (`DayColumn.zoneLabels`, LIC-47).
    private func secondaryLabels(_ labels: [String?]) -> some View {
        VStack(spacing: 0) {
            ForEach(0..<24, id: \.self) { hour in
                Text(labels.indices.contains(hour) ? labels[hour] ?? "" : "")
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(Theme.text3)
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

/// Sichtbarer Ausschnitt des Rasters in Punkten (`onScrollGeometryChange`).
struct GridViewport: Equatable {
    /// Lage des oberen Rands des Ausschnitts im Inhalt.
    var offset: CGFloat = 0
    var height: CGFloat = 0
    var contentHeight: CGFloat = 0
}

/// Ausschnitt des Rasters und automatisches Scrollen beim Ziehen (LIC-47).
///
/// Gesten in den Spalten melden den Zeiger im sichtbaren Ausschnitt
/// (`viewportSpace`). Steht er in einer Randzone, scrollt ein Takt (etwa 60
/// Schritte pro Sekunde) weiter, auch wenn der Zeiger stillsteht. Die Spalten
/// rechnen Ausschnitt-Punkte über `contentY` und `scrolled` in Punkte der
/// Spalte um. Wie weit gescrollt wird, rechnet `GridInteraction`.
@MainActor
@Observable
final class GridScroller {
    /// Name des Koordinatenraums am `ScrollView` (oben links im Ausschnitt).
    nonisolated static let viewportSpace = "lichtung.grid.viewport"

    var position = ScrollPosition(idType: Int.self)
    var viewport = GridViewport()

    /// Versatz beim Beginn des laufenden (oder letzten) Zugs.
    @ObservationIgnored private var dragStartOffset: CGFloat?
    /// Zeiger im Ausschnitt während eines Zugs, sonst `nil`.
    @ObservationIgnored private var pointerY: CGFloat?
    @ObservationIgnored private var ticker: Task<Void, Never>?
    @ObservationIgnored private var tickerGeneration = 0

    /// Wie weit der Inhalt seit Beginn des Zugs gescrollt ist (nach unten positiv).
    var scrolled: CGFloat { viewport.offset - (dragStartOffset ?? viewport.offset) }

    /// Punkt im Ausschnitt als Lage in der Spalte (jetzt).
    func contentY(_ viewportY: CGFloat) -> CGFloat { viewportY + viewport.offset }

    /// Beginn des Zugs (im Ausschnitt gemessen) als Lage in der Spalte.
    func dragStartContentY(_ viewportY: CGFloat) -> CGFloat { viewportY + (dragStartOffset ?? viewport.offset) }

    /// Bei jeder Bewegung eines Zugs.
    func dragChanged(pointerY y: CGFloat) {
        if pointerY == nil { dragStartOffset = viewport.offset }
        pointerY = y
        startTickerIfNeeded()
    }

    /// Ende (oder Abbruch) eines Zugs. `scrolled` bleibt bis zum nächsten Zug gültig.
    func dragEnded() {
        pointerY = nil
        ticker?.cancel()
        ticker = nil
    }

    private var step: Double {
        guard let pointerY else { return 0 }
        return GridInteraction.autoScrollStep(pointerY: Double(pointerY), viewportHeight: Double(viewport.height))
    }

    private func startTickerIfNeeded() {
        guard ticker == nil, step != 0 else { return }
        tickerGeneration += 1
        let generation = tickerGeneration
        ticker = Task { [weak self] in
            while let self, !Task.isCancelled, self.tick() {
                try? await Task.sleep(for: .milliseconds(16))
            }
            if let self, self.tickerGeneration == generation { self.ticker = nil }
        }
    }

    /// Ein Schritt; `false`, wenn nicht (mehr) gescrollt wird.
    private func tick() -> Bool {
        let step = self.step
        guard step != 0 else { return false }
        let target = CGFloat(GridInteraction.autoScrolledOffset(
            Double(viewport.offset), step: step,
            contentHeight: Double(viewport.contentHeight), viewportHeight: Double(viewport.height)
        ))
        guard target != viewport.offset else { return false }
        viewport.offset = target
        position.scrollTo(y: target)
        return true
    }
}

/// Laufender Zug in einer Tagesspalte. Punkte im sichtbaren Ausschnitt
/// (`GridScroller.viewportSpace`), Strecken ohne das automatische Scrollen.
enum GridDrag: Equatable {
    /// `dx`: seitliche Strecke des Fingers, nur für die Vorschau beim Halten
    /// mit Kopie (iPhone, LIC-63).
    case create(fromY: CGFloat, toY: CGFloat, dx: CGFloat = 0)
    /// `x`: Zeiger im Ausschnitt, bestimmt den Zieltag; `nil` ohne Bewegung (iPhone).
    case move(id: String, translation: CGSize, x: CGFloat?)
    case resize(id: String, dy: CGFloat)

    var occurrenceId: String? {
        switch self {
        case .create: nil
        case let .move(id, _, _), let .resize(id, _): id
        }
    }
}

struct DayColumn: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme
    let day: LocalDate
    /// Lage der Spalte im sichtbaren Bereich, für Verschieben in Nachbartage.
    let index: Int
    let dayCount: Int
    /// Blöcke, Rahmen und „+n“ des Tages (`TimelineLayout.day`).
    let layout: TimelineDay
    /// Fokus-Tag: Zeit an Rahmen, Ort und „Beitreten“ an Blöcken, „+n“ als Liste.
    let isFocus: Bool
    /// Spaltenbreiten der sichtbaren Tage (Ziehen in andere Tage, Vorschau dort).
    let widths: ColumnWidths
    /// Linker Rand der Tagesspalten im Ausschnitt (`GridScroller.viewportSpace`).
    let daysX: CGFloat
    let hourHeight: CGFloat
    let scroller: GridScroller
    /// Zweite Zeitzone: Zeilen, die von der Spalte links abweichen (LIC-47).
    var zoneLabels: [String?]? = nil

    /// Abstand je Rahmen, um den Blöcke einrücken; Linien liegen je 5 pt auseinander.
    private static let frameInset: CGFloat = 10
    /// Zeilenhöhe der Rahmentitel (10 pt Schrift).
    private static let frameTitleLine: CGFloat = 13

    private var blocks: [TimelineBlock] { layout.blocks }

    @GestureState private var drag: GridDrag? = nil
    /// Offenes „+n“ im Fokus-Tag (Liste als Popover).
    @State private var shownOverflow: String?
    /// In der Liste gewählter Termin; öffnet, sobald das Popover zu ist (LIC-64).
    @State private var pendingOverflowOpen: Occurrence?
    /// Block unter dem Zeiger (Griff zum Längeändern zeigen).
    @State private var hoveredBlock: String?
    /// Mausposition für „Neuer Termin hier“ (Mac, LIC-24).
    @State private var hover = HoverPoint()
    /// Vorschau nach dem Loslassen, bis der neue Stand da ist (LIC-47).
    @State private var settling: Settling?
    /// Verschieben eines Serientermins, das auf die Wahl des Geltungsbereichs wartet.
    @State private var pendingSeriesMove: PendingMove?
    #if os(iOS)
    /// Block, dessen Menü gerade offen ist (LIC-54).
    @State private var menuOccurrenceId: String?
    /// Im Menü gewählte Aktion; sie läuft, sobald das Popover zu ist.
    @State private var pendingMenuAction: PendingMenuAction?
    /// Menü beim Halten auf leerer Fläche (LIC-58). Die Stelle bleibt nach
    /// dem Schließen stehen, damit das Popover nicht beim Ausblenden springt.
    @State private var slotMenu: SlotMenuTarget?
    @State private var showsSlotMenu = false
    #endif

    var body: some View {
        GeometryReader { geo in
            let width = geo.size.width
            let height = geo.size.height
            ZStack(alignment: .topLeading) {
                // Leere Fläche: hier beginnt das Anlegen per Ziehen. Heute auf der Licht-Fläche.
                columnFill
                    .frame(width: width, height: height)
                    .contentShape(Rectangle())
                    .gesture(createGesture(height: height))
                    .modifier(EmptyAreaInteraction(day: day, height: height, hover: hover) { y in
                        createAt(y: y, height: height)
                    })
                ForEach(0..<24, id: \.self) { hour in
                    Rectangle()
                        .fill(Theme.line)
                        .frame(width: width, height: 0.5)
                        .offset(y: CGFloat(hour) * hourHeight)
                        .allowsHitTesting(false)
                }
                if let zoneLabels {
                    zoneLabelOverlay(zoneLabels)
                }
                // Jetzt-Linie hinter den Terminen (Spec §5).
                TimelineView(.everyMinute) { context in
                    if let line = TimelineLayout.nowLine(context.date, day: day, calendar: model.calendar) {
                        let lineY: CGFloat = height * CGFloat(line) - NowLine.thickness / 2
                        NowLine(width: width)
                            .offset(y: lineY)
                    }
                }
                .allowsHitTesting(false)
                .accessibilityHidden(true)
                // Erst die Trefferflächen, darüber die Titel: Ein Klick auf einen Titel trifft dessen Rahmen.
                ForEach(TimelineLayout.frameHitAreas(layout.frames), id: \.self) { area in
                    frameHitView(area, height: height)
                }
                let titleRows = TimelineLayout.frameTitleRows(layout.frames, lineHeight: Double(Self.frameTitleLine / max(1, height)))
                ForEach(layout.frames) { frame in
                    frameView(frame, titleRow: titleRows[frame.id] ?? 0, width: width, height: height)
                }
                // Reisezeit (LIC-42) unter den Terminen.
                ForEach(blocks) { block in
                    if let travel = TravelTime.travelBlock(for: block.occurrence),
                       let frame = GridInteraction.frame(of: travel, day: day, calendar: model.calendar),
                       frame.height > 0 {
                        let rect = blockRect(block, width: width)
                        TravelBlockView(
                            color: model.color(for: block.occurrence.calendarId),
                            minutes: block.occurrence.details.travelTimeMinutes ?? 0
                        )
                        .frame(width: rect.width, height: height * frame.height)
                        .offset(x: rect.x, y: height * frame.top)
                        .allowsHitTesting(false)
                    }
                }
                ForEach(blocks) { block in
                    blockView(block, width: width, height: height)
                }
                ForEach(layout.overflows) { overflow in
                    overflowView(overflow, width: width, height: height)
                }
                preview(width: width, height: height)
                #if os(iOS)
                slotMenuAnchor(width: width, height: height)
                #endif
            }
            .frame(width: width, height: height, alignment: .topLeading)
        }
        .overlay(alignment: .leading) {
            Rectangle().fill(Theme.line).frame(width: 0.5)
        }
        // Beim Ziehen in einen Nachbartag liegt die Vorschau über den anderen Spalten.
        .zIndex(drag == nil && settling == nil ? 0 : 1)
        #if os(iOS)
        .sensoryFeedback(.selection, trigger: drag == nil)
        #endif
        // Abgebrochene Züge kommen nicht bis `onEnded`.
        .onChange(of: drag == nil) { _, ended in
            if ended { scroller.dragEnded() }
        }
        // Der neue Stand ist da: Die Blöcke zeigen ihn selbst.
        .onChange(of: layout) { _, newLayout in
            #if os(iOS)
            // Verschwindet der Block mit offenem Menü (z. B. gelöscht), ruft das
            // Popover den Setter nicht immer auf; sonst blieben Taps in der
            // ganzen Spalte ohne Detail (LIC-64).
            if let id = menuOccurrenceId, !newLayout.blocks.contains(where: { $0.occurrence.id == id }) {
                menuOccurrenceId = nil
            }
            #endif
            settling = nil
        }
        // Editor für einen neuen Termin zu (oder ein anderer offen).
        .onChange(of: model.editorDraft?.id) { _, id in
            if let draftId = settling?.draftId, draftId != id { settling = nil }
        }
        // Rückfrage ohne Wahl geschlossen: Block springt zurück.
        .onChange(of: pendingSeriesMove == nil) { _, closed in
            if closed, settling?.isCommitted == false, settling?.occurrenceId != nil { settling = nil }
        }
        .confirmationDialog(
            "Serientermin verschieben",
            isPresented: isConfirmingSeriesMove,
            titleVisibility: .visible,
            presenting: pendingSeriesMove
        ) { pending in
            ForEach(GridInteraction.moveScopes(pending.occurrence), id: \.self) { scope in
                Button(scope.editorLabel) {
                    perform(pending.occurrence, to: pending.interval, scope: scope)
                }
            }
            Button("Abbrechen", role: .cancel) { settling = nil }
        } message: { pending in
            Text("„\(pending.occurrence.title)“ gehört zu einer Serie. Welche Termine sollen sich ändern?")
        }
    }

    /// Grund der Spalte: heute die Licht-Fläche. Dunkel als senkrechter
    /// Verlauf, voll von 8 bis 17 Uhr und je 90 Minuten auslaufend; hell flächig.
    @ViewBuilder
    private var columnFill: some View {
        if day != model.today {
            Color.clear
        } else if colorScheme == .dark {
            // shortcut: feste Uhrzeiten statt Sonnenstand; mit Ort des Nutzers auf Sonnenauf- und -untergang umstellen
            LinearGradient(stops: [
                .init(color: Theme.lit.opacity(0), location: 6.5 / 24),
                .init(color: Theme.lit, location: 8 / 24),
                .init(color: Theme.lit, location: 17 / 24),
                .init(color: Theme.lit.opacity(0), location: 18.5 / 24),
            ], startPoint: .top, endPoint: .bottom)
        } else {
            Theme.lit
        }
    }

    // MARK: Blöcke

    /// Waagerechte Lage eines Blocks: rechts der Rahmen, die ihn überdecken, dann nach Spalte.
    private func blockRect(_ block: TimelineBlock, width: CGFloat) -> (x: CGFloat, width: CGFloat) {
        let inset: CGFloat = CGFloat(block.insetByFrames) * Self.frameInset
        let available: CGFloat = max(0, width - inset)
        let x: CGFloat = inset + available * CGFloat(block.leading) + 1
        let blockWidth: CGFloat = max(0, available * CGFloat(block.width) - 2)
        return (x, blockWidth)
    }

    private func blockView(_ block: TimelineBlock, width: CGFloat, height: CGFloat) -> some View {
        let occ = block.occurrence
        let draggable = model.canDrag(occ)
        let isDragged = drag?.occurrenceId == occ.id || settling?.occurrenceId == occ.id
        let rect = blockRect(block, width: width)
        let blockHeight: CGFloat = max(14, height * block.height - 1)
        // Griff nur bei Bedarf (überfahren, markiert, gezogen) und nie über einzeiligen Titeln.
        let wantsGrip = hoveredBlock == occ.id || model.highlightedOccurrence?.id == occ.id || drag?.occurrenceId == occ.id
        let showsGrip = wantsGrip && blockHeight >= EventBlockView.oneLineHeight
        return EventBlockView(occurrence: occ, compact: !isFocus)
            .overlay(alignment: .bottom) {
                if draggable && !block.continuesAfter {
                    ResizeHandle(showsBar: showsGrip)
                        .gesture(resizeGesture(occ, height: height))
                }
            }
            .opacity(isDragged ? 0.35 : 1)
            .frame(width: rect.width, height: blockHeight)
            .onHover { inside in
                if inside { hoveredBlock = occ.id } else if hoveredBlock == occ.id { hoveredBlock = nil }
            }
            #if os(iOS)
            // Vor `offset`, damit das Popover am gezeichneten Block hängt.
            .popover(isPresented: menuBinding(for: occ.id)) {
                OccurrenceMenuPopover(model: model, occurrence: occ) { action in
                    pendingMenuAction = PendingMenuAction(run: action)
                    menuOccurrenceId = nil
                }
                .onDisappear { runPendingMenuAction() }
            }
            #endif
            .offset(x: rect.x, y: height * block.top)
            #if os(iOS)
            // Das Loslassen nach langem Halten zählt auch als Tap. Ging dabei das
            // Menü auf, kollidiert das Detail-Blatt mit ihm: Das Menü schließt
            // sich, das Blatt erscheint nie, und `selectedOccurrence` bleibt
            // gesetzt und blockiert alle weiteren Blätter (LIC-64).
            .modifier(OccurrenceTap(occurrence: occ, opensDetail: { menuOccurrenceId == nil }))
            #else
            .modifier(OccurrenceTap(occurrence: occ)) // markieren, doppelt öffnen (LIC-24)
            #endif
            .gesture(moveGesture(occ, width: width, height: height), including: draggable ? .all : .subviews)
            .modifier(BlockMenu(occurrence: occ, draggable: draggable))
            .modifier(NudgeActions(enabled: draggable) { minutes in nudge(occ, by: minutes) })
    }

    // MARK: Rahmen und „+n“ (Spec §5)

    /// Langer Termin als Linie am linken Rand mit kleinem Titel oben. Der Titel
    /// ist anklickbar und hat das Menü wie ein Block; die Linie trifft man über
    /// `frameHitView`. Ziehen geht nicht.
    private func frameView(_ frame: TimelineFrame, titleRow: Int, width: CGFloat, height: CGFloat) -> some View {
        let occ = frame.occurrence
        let lineX: CGFloat = CGFloat(TimelineLayout.frameLineX(frame.index))
        let frameHeight: CGFloat = max(14, height * frame.height)
        // Rechts aller Linien des Tages, damit keine durch einen Titel läuft.
        let lastLine: CGFloat = CGFloat(TimelineLayout.frameLineX(max(0, layout.frames.count - 1)))
        let titleX: CGFloat = lastLine + 6
        let titleY: CGFloat = height * frame.top + 1 + CGFloat(titleRow) * Self.frameTitleLine
        let isHighlighted: Bool = model.highlightedOccurrence?.id == occ.id
        let time = AgendaBuilder.timeText(occ, calendar: model.calendar)
        return TimelineView(.everyMinute) { context in
            let state = Theme.state(of: occ, now: context.date)
            ZStack(alignment: .topLeading) {
                Rectangle()
                    .fill(model.color(for: occ.calendarId).opacity(state == .past ? 0.4 : 1))
                    .frame(width: 1.5, height: frameHeight)
                    .modifier(HighlightBorder(isOn: isHighlighted))
                    .offset(x: lineX, y: height * frame.top)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
                HStack(spacing: 4) {
                    Text(occ.title)
                    if isFocus {
                        Text(time).monospacedDigit()
                    }
                }
                .font(.system(size: 10, weight: isHighlighted ? .semibold : .regular))
                .fontWidth(isFocus ? .standard : .condensed)
                .foregroundStyle(state == .past ? Theme.text2 : Theme.text1)
                .lineLimit(1)
                // Vor `frame`: Nur der Titel selbst ist Trefferfläche, nicht die Zeile.
                .contentShape(Rectangle())
                .frame(maxWidth: max(0, width - titleX - 2), alignment: .leading)
                .offset(x: titleX, y: titleY)
                .modifier(OccurrenceTap(occurrence: occ))
                .modifier(BlockMenu(occurrence: occ, draggable: false))
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("\(occ.title), \(time)")
                .accessibilityValue(EventBlockView.accessibilityValue(state))
                .accessibilityAddTraits(isHighlighted ? [.isButton, .isSelected] : .isButton)
            }
        }
    }

    /// Trefferfläche einer Linie: Das Band links (mindestens 16 pt) teilt sich
    /// zwischen Rahmen, die zur selben Zeit laufen (`TimelineLayout.frameHitAreas`).
    @ViewBuilder
    private func frameHitView(_ area: FrameHitArea, height: CGFloat) -> some View {
        if let occ = layout.frames.first(where: { $0.id == area.frameId })?.occurrence {
            Color.clear
                .frame(width: CGFloat(area.width), height: max(14, height * CGFloat(area.height)))
                .contentShape(Rectangle())
                .offset(x: CGFloat(area.x), y: height * CGFloat(area.top))
                .modifier(OccurrenceTap(occurrence: occ))
                .modifier(BlockMenu(occurrence: occ, draggable: false))
                .accessibilityHidden(true)
        }
    }

    /// Feld „+n“ für Termine über die erlaubten Spalten hinaus. Im schmalen Tag
    /// macht es den Tag zum Fokus-Tag, im Fokus-Tag zeigt es die Termine als Liste.
    private func overflowView(_ overflow: TimelineOverflow, width: CGFloat, height: CGFloat) -> some View {
        let frames = layout.frames.filter { $0.top < overflow.top + overflow.height && overflow.top < $0.top + $0.height }.count
        let inset: CGFloat = CGFloat(frames) * Self.frameInset
        let columnWidth: CGFloat = max(0, width - inset) / CGFloat(max(1, overflow.columns))
        let x: CGFloat = inset + columnWidth * CGFloat(overflow.column) + 1
        let count = overflow.hidden.count
        return Button {
            if isFocus { shownOverflow = overflow.id } else { model.focus(day) }
        } label: {
            Text(verbatim: "+\(count)")
                .font(Theme.digits(.caption, weight: .semibold))
                .foregroundStyle(Theme.text1)
                .padding(.top, 3)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                .background(RoundedRectangle(cornerRadius: Theme.blockRadius, style: .continuous).fill(Theme.line))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(width: max(0, columnWidth - 2), height: max(14, height * overflow.height - 1))
        // Markierung (Tastatur) auf einem verborgenen Termin: dann am Feld zeigen.
        .modifier(HighlightBorder(isOn: overflow.hidden.contains { $0.id == model.highlightedOccurrence?.id }))
        .popover(isPresented: overflowBinding(overflow.id)) {
            OverflowList(model: model, occurrences: overflow.hidden) { occ in
                pendingOverflowOpen = occ
                shownOverflow = nil
            }
            .onDisappear {
                if let occ = pendingOverflowOpen {
                    pendingOverflowOpen = nil
                    model.open(occ)
                }
            }
        }
        .offset(x: x, y: height * overflow.top)
        .accessibilityLabel("\(count) weitere Termine")
        .accessibilityHint(isFocus ? "Zeigt sie als Liste" : "Macht den Tag breit")
    }

    private func overflowBinding(_ id: String) -> Binding<Bool> {
        Binding(
            get: { shownOverflow == id },
            set: { shown in if !shown, shownOverflow == id { shownOverflow = nil } }
        )
    }

    /// Zeilen der zweiten Zeitzone, die an diesem Tag anders lauten als links
    /// (Umstellung in dieser Woche, LIC-47). Klein am linken Rand der Spalte.
    private func zoneLabelOverlay(_ labels: [String?]) -> some View {
        ForEach(0..<min(24, labels.count), id: \.self) { hour in
            if let label = labels[hour] {
                Text(label)
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(Theme.text3)
                    .padding(.leading, 2)
                    .offset(y: CGFloat(hour) * hourHeight + 1)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
        }
    }

    // MARK: Gesten

    /// Anlegen: Mac direkt ziehen, iPhone lange drücken und dann ziehen. Am
    /// iPhone öffnet Loslassen ohne Bewegung das Menü „Hier einfügen“, wenn
    /// eine Kopie vorliegt (LIC-58); ohne Kopie bleibt alles wie bisher.
    private func createGesture(height: CGFloat) -> some Gesture {
        #if os(iOS)
        return LongPressGesture(minimumDuration: 0.4)
            .sequenced(before: DragGesture(minimumDistance: 0, coordinateSpace: .named(GridScroller.viewportSpace)))
            .updating($drag) { value, state, _ in
                if case .second(true, let d?) = value {
                    state = .create(fromY: d.startLocation.y, toY: d.location.y, dx: d.translation.width)
                }
            }
            .onChanged { value in
                if case .second(true, let d?) = value { scroller.dragChanged(pointerY: d.location.y) }
            }
            .onEnded { value in
                if case .second(true, let d) = value {
                    let translation: (dx: Double, dy: Double)? = d.map {
                        (dx: Double($0.translation.width), dy: Double($0.translation.height + scroller.scrolled))
                    }
                    switch GridInteraction.slotHoldRelease(translation: translation, canPaste: canPaste) {
                    case .create:
                        if let d { commitCreate(fromY: d.startLocation.y, toY: d.location.y, height: height) }
                    case .menu:
                        if let d { openSlotMenu(atViewportY: d.startLocation.y, height: height) }
                    case .nothing:
                        break
                    }
                }
                scroller.dragEnded()
            }
        #else
        return DragGesture(minimumDistance: 4, coordinateSpace: .named(GridScroller.viewportSpace))
            .updating($drag) { value, state, _ in
                state = .create(fromY: value.startLocation.y, toY: value.location.y)
            }
            .onChanged { value in scroller.dragChanged(pointerY: value.location.y) }
            .onEnded { value in
                commitCreate(fromY: value.startLocation.y, toY: value.location.y, height: height)
                scroller.dragEnded()
            }
        #endif
    }

    /// Verschieben: Mac direkt ziehen, iPhone lange drücken und dann ziehen.
    /// Ein kurzer Tap bleibt beim `onTapGesture` und öffnet das Detail. Am
    /// iPhone öffnet Loslassen ohne Bewegung das Menü (LIC-54).
    private func moveGesture(_ occ: Occurrence, width: CGFloat, height: CGFloat) -> some Gesture {
        let id = occ.id
        #if os(iOS)
        return LongPressGesture(minimumDuration: 0.3)
            .sequenced(before: DragGesture(minimumDistance: 0, coordinateSpace: .named(GridScroller.viewportSpace)))
            .updating($drag) { value, state, _ in
                if case .second(true, let d) = value {
                    state = .move(id: id, translation: d?.translation ?? .zero, x: d?.location.x)
                }
            }
            .onChanged { value in
                if case .second(true, let d?) = value { scroller.dragChanged(pointerY: d.location.y) }
            }
            .onEnded { value in
                if case .second(true, let d) = value {
                    let translation = d?.translation ?? .zero
                    if GridInteraction.opensMenuOnRelease(
                        dx: Double(translation.width), dy: Double(translation.height + scroller.scrolled), columnHeight: Double(height)
                    ) {
                        menuOccurrenceId = id
                    } else {
                        commitMove(occ, translation: translation, pointerX: d?.location.x, width: width, height: height)
                    }
                }
                scroller.dragEnded()
            }
        #else
        return DragGesture(minimumDistance: 4, coordinateSpace: .named(GridScroller.viewportSpace))
            .updating($drag) { value, state, _ in
                state = .move(id: id, translation: value.translation, x: value.location.x)
            }
            .onChanged { value in scroller.dragChanged(pointerY: value.location.y) }
            .onEnded { value in
                commitMove(occ, translation: value.translation, pointerX: value.location.x, width: width, height: height)
                scroller.dragEnded()
            }
        #endif
    }

    /// Länge ändern am Griff: auf beiden Plattformen direkt ziehen.
    private func resizeGesture(_ occ: Occurrence, height: CGFloat) -> some Gesture {
        let id = occ.id
        return DragGesture(minimumDistance: 2, coordinateSpace: .named(GridScroller.viewportSpace))
            .updating($drag) { value, state, _ in
                state = .resize(id: id, dy: value.translation.height)
            }
            .onChanged { value in scroller.dragChanged(pointerY: value.location.y) }
            .onEnded { value in
                let target = resizedInterval(occ, dy: value.translation.height, height: height)
                commit(occ, to: target, preview: PreviewTarget(interval: target, columnOffset: 0, calendarId: occ.calendarId))
                scroller.dragEnded()
            }
    }

    // MARK: Rechnen (GridInteraction)

    /// `fromY`/`toY` im Ausschnitt; umgerechnet mit dem Scrollen seit Beginn des Zugs.
    private func creationInterval(fromY: CGFloat, toY: CGFloat, height: CGFloat) -> DateInterval {
        GridInteraction.creation(
            day: day, fromY: Double(scroller.dragStartContentY(fromY)), toY: Double(scroller.contentY(toY)),
            columnHeight: Double(height), calendar: model.calendar
        )
    }

    /// Breite aller Tagesspalten, aus der eigenen Breite und ihrem Anteil.
    private func daysWidth(_ width: CGFloat) -> CGFloat {
        let fraction = widths.fractions.indices.contains(index) ? widths.fractions[index] : 1
        return fraction > 0 ? width / CGFloat(fraction) : width
    }

    /// Tage vom Ausgangstag zur Spalte unter dem Zeiger (ungleiche Breiten, `ColumnWidths`).
    private func moveDayOffset(pointerX: CGFloat?, width: CGFloat) -> Int {
        pointerX.map { x in
            widths.dayOffset(fromColumn: index, pointerFraction: Double((x - daysX) / max(1, daysWidth(width))))
        } ?? 0
    }

    private func movedInterval(_ occ: Occurrence, translation: CGSize, dayOffset offset: Int, height: CGFloat) -> DateInterval {
        let delta = GridInteraction.minutes(forDistance: Double(translation.height + scroller.scrolled), columnHeight: Double(height))
        return GridInteraction.moved(start: occ.start, end: occ.end, deltaMinutes: delta, dayOffset: offset, calendar: model.calendar)
    }

    private func resizedInterval(_ occ: Occurrence, dy: CGFloat, height: CGFloat) -> DateInterval {
        let delta = GridInteraction.minutes(forDistance: Double(dy + scroller.scrolled), columnHeight: Double(height))
        return GridInteraction.resized(start: occ.start, end: occ.end, day: day, deltaMinutes: delta, calendar: model.calendar)
    }

    // MARK: Anlegen (Editor, LIC-47)

    /// Zug in leerer Fläche: Editor mit dem Zeitraum öffnen.
    private func commitCreate(fromY: CGFloat, toY: CGFloat, height: CGFloat) {
        let target = creationInterval(fromY: fromY, toY: toY, height: height)
        openEditor(model.newEventDraft(spanning: target), interval: target)
    }

    /// Doppelklick in leere Fläche (Mac): Editor zur Zeit an der Stelle `y`
    /// der Spalte, gerundet auf das Raster, mit der Standarddauer.
    private func createAt(y: CGFloat, height: CGFloat) {
        let start = GridInteraction.slotStart(day: day, atY: Double(y), columnHeight: Double(height))
        let draft = model.newEventDraft(startingAt: start)
        openEditor(draft, interval: DateInterval(start: draft.startInstant, end: max(draft.startInstant, draft.endInstant)))
    }

    /// Die Vorschau bleibt stehen, solange der Editor diesen Entwurf zeigt.
    private func openEditor(_ draft: EventDraft, interval: DateInterval) {
        let calendarId: String? = draft.calendarId.isEmpty ? nil : draft.calendarId
        settling = Settling(target: PreviewTarget(interval: interval, columnOffset: 0, calendarId: calendarId), draftId: draft.id)
        model.editorDraft = draft
    }

    // MARK: Verschieben

    private func commitMove(_ occ: Occurrence, translation: CGSize, pointerX: CGFloat?, width: CGFloat, height: CGFloat) {
        let offset = moveDayOffset(pointerX: pointerX, width: width)
        let target = movedInterval(occ, translation: translation, dayOffset: offset, height: height)
        commit(occ, to: target, preview: targetPreview(target, from: occ, dayOffset: offset))
    }

    /// Bedienhilfen: 15 Minuten früher oder später (LIC-47).
    private func nudge(_ occ: Occurrence, by minutes: Int) {
        let target = GridInteraction.nudged(start: occ.start, end: occ.end, minutes: minutes, calendar: model.calendar)
        commit(occ, to: target, preview: targetPreview(target, from: occ, dayOffset: nil))
    }

    /// Neue Zeit übernehmen. Bei Serien erst nach der Frage nach dem
    /// Geltungsbereich; bis dahin und bis der neue Stand da ist, steht die Vorschau.
    private func commit(_ occ: Occurrence, to target: DateInterval, preview: PreviewTarget?) {
        guard target.start != occ.start || target.end != occ.end else { return }
        settling = preview.map { Settling(target: $0, occurrenceId: occ.id) }
        if GridInteraction.moveScopes(occ).isEmpty {
            perform(occ, to: target, scope: .all)
        } else {
            pendingSeriesMove = PendingMove(occurrence: occ, interval: target)
        }
    }

    private func perform(_ occ: Occurrence, to target: DateInterval, scope: EditScope) {
        settling?.isCommitted = true
        let token = settling?.id
        Task {
            let changed = await model.move(occ, start: target.start, end: target.end, scope: scope)
            guard changed else {
                if settling?.id == token { settling = nil }
                return
            }
            // Ändern sich die Blöcke nicht (z. B. abgelehnt), nicht ewig stehen lassen.
            try? await Task.sleep(for: .seconds(2))
            if let token, settling?.id == token { settling = nil }
        }
    }

    private var isConfirmingSeriesMove: Binding<Bool> {
        Binding(
            get: { pendingSeriesMove != nil },
            set: { if !$0 { pendingSeriesMove = nil } }
        )
    }

    // MARK: Vorschau

    /// Vorschau zum laufenden Zug: Zeitraum, Spaltenversatz, Kalender (für die Farbe).
    private struct PreviewTarget {
        var interval: DateInterval
        var columnOffset: Int
        var calendarId: String?
    }

    /// Vorschau, die nach dem Loslassen stehen bleibt (LIC-47).
    private struct Settling {
        let id = UUID()
        var target: PreviewTarget
        /// Verschobener Termin (blass, bis der neue Stand da ist).
        var occurrenceId: String? = nil
        /// Entwurf im Editor bei einem neuen Termin.
        var draftId: String? = nil
        /// Schon an den Engine gegeben (bei Serien: Bereich gewählt).
        var isCommitted = false
    }

    /// Serientermin, für den die Frage nach dem Geltungsbereich offen ist.
    private struct PendingMove {
        var occurrence: Occurrence
        var interval: DateInterval
    }

    #if os(iOS)
    // MARK: Menü am Block (iPhone, LIC-54)

    /// Aktion aus dem Popover-Menü, die auf dessen Schließen wartet.
    private struct PendingMenuAction {
        var run: @MainActor () -> Void
    }

    private func menuBinding(for id: String) -> Binding<Bool> {
        Binding(
            get: { menuOccurrenceId == id },
            set: { shown in if !shown, menuOccurrenceId == id { menuOccurrenceId = nil } }
        )
    }

    private func runPendingMenuAction() {
        guard let action = pendingMenuAction else { return }
        pendingMenuAction = nil
        action.run()
    }

    // MARK: Menü auf leerer Fläche (iPhone, LIC-58)

    /// Gehaltene Stelle: Uhrzeit zum Einfügen und Lage in der Spalte (für
    /// „Neuer Termin hier“).
    private struct SlotMenuTarget {
        var start: Date
        var contentY: CGFloat
    }

    /// Ob „Hier einfügen“ geht: Kopie da, Zwischenablage hält sie noch.
    private var canPaste: Bool {
        model.clipboard?.matches(Pasteboard.string) == true
    }

    /// Vorschau beim Halten mit Kopie (LIC-63): Solange Loslassen das Menü
    /// „Hier einfügen“ öffnen würde, zeigt sie den eingefügten Termin in
    /// Länge und Farbe der Kopie; wer zieht, sieht wieder das Anlegen.
    /// Ganztägige Kopien behalten den Block des Anlegens.
    private func slotPastePreview(fromY: CGFloat, toY: CGFloat, dx: CGFloat, height: CGFloat) -> PreviewTarget? {
        guard let clip = model.clipboard, canPaste else { return nil }
        let translation = (dx: Double(dx), dy: Double(toY - fromY + scroller.scrolled))
        guard GridInteraction.slotHoldRelease(translation: translation, canPaste: true) == .menu else { return nil }
        // Gleiche Stelle wie `openSlotMenu`, damit Vorschau und Menü dieselbe Uhrzeit nennen.
        let start = GridInteraction.pasteStart(day: day, atY: Double(scroller.contentY(fromY)), columnHeight: Double(height), calendar: model.calendar)
        guard let interval = clip.pastedInterval(at: start, deviceTimeZone: model.timeZone) else { return nil }
        let calendarId = model.duplicateCalendarId(for: clip.occurrence) ?? clip.occurrence.calendarId
        return PreviewTarget(interval: interval, columnOffset: 0, calendarId: calendarId)
    }

    /// `y` im sichtbaren Ausschnitt. Der Finger stand still, also zählt der
    /// jetzige Versatz (nicht der vom Beginn eines Zugs).
    private func openSlotMenu(atViewportY y: CGFloat, height: CGFloat) {
        let contentY = scroller.contentY(y)
        let start = GridInteraction.pasteStart(day: day, atY: Double(contentY), columnHeight: Double(height), calendar: model.calendar)
        slotMenu = SlotMenuTarget(start: start, contentY: contentY)
        showsSlotMenu = true
    }

    /// Unsichtbarer Anker für das Popover in der Zeile der gehaltenen
    /// Uhrzeit. Steht immer in der Spalte, damit das Popover einen Ort hat,
    /// sobald es aufgeht.
    private func slotMenuAnchor(width: CGFloat, height: CGFloat) -> some View {
        let slot: GridFrame? = slotMenu.flatMap { target in
            let interval = DateInterval(start: target.start, duration: Double(GridInteraction.snapMinutes) * 60)
            return GridInteraction.frame(of: interval, day: day, calendar: model.calendar)
        }
        let top: CGFloat = slot.map { height * CGFloat($0.top) } ?? 0
        let slotHeight: CGFloat = slot.map { max(1, height * CGFloat($0.height)) } ?? 1
        return Color.clear
            .frame(width: width, height: slotHeight)
            // Vor `offset`, damit das Popover an der Zeile hängt.
            .popover(isPresented: $showsSlotMenu) {
                if let target = slotMenu {
                    SlotMenuPopover(model: model, start: target.start, newEvent: {
                        createAt(y: target.contentY, height: height)
                    }) { action in
                        pendingMenuAction = PendingMenuAction(run: action)
                        showsSlotMenu = false
                    }
                    .onDisappear { runPendingMenuAction() }
                }
            }
            .offset(y: top)
            .allowsHitTesting(false)
    }
    #endif

    /// Gezeichnet wird in der Spalte des Zugs (`dayOffset`, auch für Termine
    /// aus dem Vortag), sonst in der Spalte, in der der neue Beginn liegt;
    /// `nil`, wenn der Termin in keiner sichtbaren Spalte zu sehen wäre.
    private func targetPreview(_ target: DateInterval, from occ: Occurrence, dayOffset: Int?) -> PreviewTarget? {
        let startOffset = day.days(until: LocalDate(target.start, calendar: model.calendar))
        let offset = [dayOffset, startOffset].compactMap { $0 }.first { offset in
            (0..<dayCount).contains(index + offset)
                && GridInteraction.frame(of: target, day: day.adding(days: offset), calendar: model.calendar) != nil
        }
        return offset.map { PreviewTarget(interval: target, columnOffset: $0, calendarId: occ.calendarId) }
    }

    private func previewTarget(width: CGFloat, height: CGFloat) -> PreviewTarget? {
        guard let drag else { return settling?.target }
        switch drag {
        case let .create(fromY, toY, dx):
            return createPreview(fromY: fromY, toY: toY, dx: dx, height: height)
        case let .move(id, translation, x):
            guard let occ = blocks.first(where: { $0.occurrence.id == id })?.occurrence else { return nil }
            let offset = moveDayOffset(pointerX: x, width: width)
            let target = movedInterval(occ, translation: translation, dayOffset: offset, height: height)
            return targetPreview(target, from: occ, dayOffset: offset)
        case let .resize(id, dy):
            guard let occ = blocks.first(where: { $0.occurrence.id == id })?.occurrence else { return nil }
            return PreviewTarget(interval: resizedInterval(occ, dy: dy, height: height), columnOffset: 0, calendarId: occ.calendarId)
        }
    }

    /// Zug auf leerer Fläche: der Zeitraum des Anlegens. Am iPhone mit Kopie
    /// beim ruhigen Halten stattdessen der eingefügte Termin (LIC-63).
    private func createPreview(fromY: CGFloat, toY: CGFloat, dx: CGFloat, height: CGFloat) -> PreviewTarget {
        #if os(iOS)
        if let paste = slotPastePreview(fromY: fromY, toY: toY, dx: dx, height: height) { return paste }
        #endif
        return PreviewTarget(interval: creationInterval(fromY: fromY, toY: toY, height: height), columnOffset: 0, calendarId: nil)
    }

    @ViewBuilder
    private func preview(width: CGFloat, height: CGFloat) -> some View {
        if let target = previewTarget(width: width, height: height),
           let frame = GridInteraction.frame(of: target.interval, day: day.adding(days: target.columnOffset), calendar: model.calendar) {
            let color = target.calendarId.map { model.color(for: $0) } ?? Theme.text2
            // In der Zielspalte mit deren Breite.
            let total = daysWidth(width)
            let column = index + target.columnOffset
            let previewX: CGFloat = CGFloat(widths.leading(column) - widths.leading(index)) * total + 1
            let previewWidth: CGFloat = target.columnOffset == 0 ? width : CGFloat(widths.width(from: column, through: column)) * total
            RoundedRectangle(cornerRadius: Theme.blockRadius, style: .continuous)
                .fill(color.opacity(0.3))
                .overlay {
                    RoundedRectangle(cornerRadius: Theme.blockRadius, style: .continuous)
                        .strokeBorder(color, style: StrokeStyle(lineWidth: 1.5, dash: [5, 3]))
                }
                .overlay(alignment: .topLeading) {
                    Text(GridInteraction.timeText(target.interval, calendar: model.calendar))
                        .font(.caption2.monospacedDigit().weight(.semibold))
                        .lineLimit(1)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 2)
                }
                .frame(width: max(0, previewWidth - 2), height: max(14, height * frame.height))
                .offset(x: previewX, y: height * frame.top)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }
}

/// Griff am unteren Rand eines Blocks zum Länge-Ändern.
/// Die Fläche greift immer; der Strich erscheint nur, wenn `showsBar`.
private struct ResizeHandle: View {
    let showsBar: Bool

    var body: some View {
        Color.clear
            .frame(maxWidth: .infinity)
            .frame(height: 8)
            .contentShape(Rectangle())
            .overlay {
                if showsBar {
                    Capsule()
                        .fill(Theme.text2.opacity(0.6))
                        .frame(width: 18, height: 3)
                }
            }
            #if os(macOS)
            .onHover { inside in
                if inside { NSCursor.resizeUpDown.push() } else { NSCursor.pop() }
            }
            #endif
            .accessibilityHidden(true)
    }
}

/// Bedienhilfen am Block: „15 Minuten früher/später“ (LIC-47), nur bei
/// Terminen, die sich auch ziehen lassen.
private struct NudgeActions: ViewModifier {
    let enabled: Bool
    let nudge: (Int) -> Void

    func body(content: Content) -> some View {
        content.accessibilityActions {
            if enabled {
                ForEach(GridInteraction.nudgeActions) { action in
                    Button(action.name) { nudge(action.minutes) }
                }
            }
        }
    }
}

/// Leere Fläche einer Spalte am Mac (LIC-24): Klick hebt die Markierung auf
/// und wählt den Tag, Doppelklick legt einen Termin an der Stelle an (LIC-47),
/// Rechtsklick bietet „Neuer Termin hier“ zur Uhrzeit unter dem Mauszeiger.
/// Am iPhone bleibt es beim Anlegen per Ziehen.
private struct EmptyAreaInteraction: ViewModifier {
    @Environment(AppModel.self) private var model
    let day: LocalDate
    let height: CGFloat
    let hover: HoverPoint
    /// Doppelklick an der Stelle `y` der Spalte.
    let onDoubleClick: (CGFloat) -> Void

    func body(content: Content) -> some View {
        #if os(macOS)
        content
            .onTapGesture(count: 2) { point in onDoubleClick(point.y) }
            .simultaneousGesture(TapGesture().onEnded { model.tapEmptyArea(day: day) })
            .onContinuousHover { phase in
                if case let .active(point) = phase { hover.y = point.y }
            }
            .contextMenu {
                DaySlotMenu(day: day, start: { slotStart() })
            }
        #else
        content
        #endif
    }

    private func slotStart() -> LocalDateTime {
        let y: Double? = hover.y.map { Double($0) }
        return GridInteraction.slotStart(day: day, atY: y, columnHeight: Double(height))
    }
}

/// Kontextmenü am Block. Am iPhone ist langes Drücken bei ziehbaren Terminen
/// das Aufnehmen zum Verschieben; ihr Menü öffnet sich beim Loslassen ohne
/// Bewegung als Popover (`DayColumn.moveGesture`, LIC-54).
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
                RoundedRectangle(cornerRadius: Theme.blockRadius, style: .continuous).fill(color.opacity(0.08))
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
                        .foregroundStyle(Theme.text2)
                        .lineLimit(1)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 1)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: Theme.blockRadius, style: .continuous))
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Reisezeit \(TravelTime.label(minutes: minutes))")
    }
}

/// Termin im Raster in einem von drei Zuständen (Spec §4): vorbei ohne
/// Fläche, kommend getönt, laufend in voller Kalenderfarbe.
struct EventBlockView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL
    let occurrence: Occurrence
    /// Schmale Spalte: Schrift schmal, einzeilig ohne Uhrzeit. Sonst (Fokus-Tag)
    /// mit Ort und ab `joinHeight` mit „Beitreten“.
    var compact: Bool = false

    /// Darunter ist ein Termin eine Zeile.
    static let oneLineHeight: CGFloat = 28
    /// Darunter wird ein Block auch im Fokus-Tag schmal gesetzt.
    private static let tightWidth: CGFloat = 60
    /// Ab dieser Höhe hat „Beitreten“ im Fokus-Tag Platz.
    private static let joinHeight: CGFloat = 60

    var body: some View {
        // Jede Minute neu: Der Zustand wechselt auch bei offenem Raster.
        TimelineView(.everyMinute) { context in
            GeometryReader { geo in
                block(state: Theme.state(of: occurrence, now: context.date), size: geo.size)
            }
        }
    }

    private func block(state: Theme.EventState, size: CGSize) -> some View {
        let height = size.height
        let oneLine = height < Self.oneLineHeight
        // Unter 60 pt Breite wie eine schmale Spalte, Titel einzeilig (kein Umbruch mitten im Wort).
        let tight = size.width < Self.tightWidth
        let compact = self.compact || tight
        // Fokus-Tag: großer Knopf statt Symbol, auch vor dem Zeitfenster (Spec §4: kommend oder laufend).
        let joinLink: VideoLink? = !compact && state != .past && height >= Self.joinHeight
            ? VideoLinkDetector.detect(in: occurrence.details) : nil
        let color = model.color(for: occurrence.calendarId)
        let isHighlighted: Bool = model.highlightedOccurrence?.id == occurrence.id
        // Laufend: Titel und Zeit in derselben deckenden Farbe, Rang nur über Größe und Gewicht.
        let runningText: Color = Theme.onRunning(for: model.colorHex(for: occurrence.calendarId))
        let titleColor: Color = state == .running ? runningText : (state == .past ? Theme.text2 : Theme.text1)
        // Kommend Text 1: Text 2 erreicht auf der getönten Fläche keine 4,5:1 (3,7–4,4).
        let detailColor: Color = state == .running ? runningText : (state == .past ? Theme.text2 : Theme.text1)
        let fill: Color = state == .running ? color : (state == .past ? Color.clear : Theme.eventFill(color))
        // Fokus-Tag größer (Vorlage: 13/11 statt 11/10).
        let detailFont: Font = compact || oneLine ? Theme.digits(.caption2) : .system(size: 11).monospacedDigit()
        let title = Text(occurrence.title)
            .font(compact || oneLine ? .caption.weight(.medium) : .system(size: 13, weight: .medium))
            .strikethrough(occurrence.details.status == .cancelled)
            .foregroundStyle(titleColor)
        return HStack(alignment: oneLine ? .center : .top, spacing: 0) {
            if oneLine {
                HStack(spacing: 6) {
                    title.lineLimit(1)
                    if !compact {
                        Text(startText)
                            .font(detailFont)
                            .foregroundStyle(detailColor)
                            .fixedSize()
                    }
                }
                .padding(.leading, 9)
                .padding(.trailing, 6)
            } else {
                VStack(alignment: .leading, spacing: 1) {
                    title.lineLimit(tight ? 1 : 2)
                    HStack(spacing: 6) {
                        Text(AgendaBuilder.timeText(occurrence, calendar: model.calendar))
                            .layoutPriority(1)
                        if !compact, let location = occurrence.details.location, !location.isEmpty {
                            Text(location)
                        }
                    }
                    .font(detailFont)
                    .foregroundStyle(detailColor)
                    .lineLimit(1)
                    if let joinLink {
                        Button {
                            JoinButton.join(joinLink, with: openURL)
                        } label: {
                            Label("Beitreten", systemImage: "video.fill")
                        }
                        .buttonStyle(Theme.PrimaryButtonStyle())
                        .controlSize(.small)
                        .help("\(joinLink.provider.displayName) beitreten")
                        .accessibilityLabel("\(joinLink.provider.displayName) beitreten")
                        .padding(.top, 5)
                    }
                }
                .padding(.leading, 9)
                .padding(.trailing, 6)
                .padding(.vertical, 2)
            }
            Spacer(minLength: 0)
            if joinLink == nil {
                JoinButton(occurrence: occurrence, iconOnly: true) // Beitreten (LIC-42)
                    .tint(titleColor)
                    .foregroundStyle(titleColor)
            }
        }
        .fontWidth(compact ? .condensed : .standard)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: oneLine ? .leading : .topLeading)
        .background(fill)
        .overlay(alignment: .leading) {
            Rectangle().fill(color.opacity(state == .past ? 0.4 : 1)).frame(width: 3)
        }
        .clipShape(RoundedRectangle(cornerRadius: Theme.blockRadius, style: .continuous))
        // Vorbei hat keine Fläche; Tippen, Ziehen und Menü treffen trotzdem den ganzen Block.
        .contentShape(RoundedRectangle(cornerRadius: Theme.blockRadius, style: .continuous))
        .modifier(HighlightBorder(isOn: isHighlighted))
        .accessibilityElement(children: .combine)
        .accessibilityValue(Self.accessibilityValue(state))
        .accessibilityAddTraits(isHighlighted ? .isSelected : [])
    }

    /// Zustand nie nur über Farbe (Spec §4).
    static func accessibilityValue(_ state: Theme.EventState) -> String {
        switch state {
        case .past: "vorbei"
        case .running: "läuft gerade"
        case .upcoming: ""
        }
    }

    /// Beginn „09:30“ für die einzeilige Fassung.
    private var startText: String {
        DayGridView.clockText(occurrence.start, calendar: model.calendar)
    }
}

/// Rahmen um den markierten Termin (LIC-24): 1,5 pt in Text 1, 1 pt Abstand.
struct HighlightBorder: ViewModifier {
    let isOn: Bool

    func body(content: Content) -> some View {
        content.overlay {
            if isOn {
                RoundedRectangle(cornerRadius: Theme.blockRadius + 2.5, style: .continuous)
                    .strokeBorder(Theme.text1, lineWidth: 1.5)
                    .padding(-2.5)
            }
        }
    }
}

/// Jetzt-Linie: 1,5 pt in Sonne über die Spalte.
struct NowLine: View {
    static let thickness: CGFloat = 1.5
    let width: CGFloat

    var body: some View {
        Rectangle()
            .fill(Theme.sun)
            .frame(width: width, height: Self.thickness)
    }
}

/// Kopf einer Tagesspalte in einer Zeile: Fokus-Tag „Dienstag 10“ groß
/// (Name leicht, Zahl halbfett), übrige „Mo 9“ in Textgröße. Heute steht die
/// Zahl in Licht, am Wochenende in Text 2.
struct DayHeaderCell: View {
    let day: LocalDate
    let isToday: Bool
    /// Gewählter Tag (Ziel für Einfügen und „Neuer Termin“), nur für VoiceOver;
    /// gezeichnet wird er von `selectedDayMarker`.
    let isSelected: Bool
    let isWeekend: Bool
    let isFocus: Bool
    var focusSize: CGFloat = 26

    var body: some View {
        let index = day.weekday.rawValue - 1
        let numberColor: Color = isToday ? Theme.light : (isWeekend ? Theme.text2 : Theme.text1)
        let number = Text(verbatim: "\(day.day)").monospacedDigit().foregroundStyle(numberColor)
        // Ein Text, damit er beim Schrumpfen als Ganzes kleiner wird.
        Group {
            if isFocus {
                Text(QuickEntryPreview.weekdayLong[index]).fontWeight(.light).foregroundStyle(Theme.text1)
                    + Text(verbatim: " ")
                    + number.fontWeight(isToday ? .bold : .semibold)
            } else {
                Text(QuickEntryPreview.weekdayShort[index]).foregroundStyle(Theme.text2)
                    + Text(verbatim: " ")
                    + number.fontWeight(isToday ? .bold : .regular)
            }
        }
        .font(isFocus ? .system(size: focusSize) : Self.textFont)
        .fontWidth(isFocus ? .standard : .condensed)
        .lineLimit(1)
        .minimumScaleFactor(0.5)
        .padding(.horizontal, 8)
        .accessibilityElement(children: .combine)
        .accessibilityValue(isToday ? "Heute" : "")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    /// Übrige Tage in Textgröße; am iPhone kleiner, damit sieben Tage passen.
    private static var textFont: Font {
        #if os(iOS)
        .footnote
        #else
        .body
        #endif
    }
}

/// Ganztägige Zeile über dem Raster (und Balken im Monatsraster).
/// `widths`: Spaltenbreiten mit Fokus-Tag (Woche); ohne sind alle gleich breit.
struct AllDayRow: View {
    @Environment(AppModel.self) private var model
    let bars: [LaneBar]
    let columns: Int
    let visibleLanes: Int
    var leadingInset: CGFloat = 0
    var widths: ColumnWidths? = nil
    var laneHeight: CGFloat = 18

    var body: some View {
        let lanes = min(visibleLanes, (bars.map(\.lane).max() ?? -1) + 1)
        let hidden = LaneLayout.overflow(bars, columns: columns, visibleLanes: visibleLanes)
        let widths = widths ?? ColumnWidths(columns: columns, focus: nil)
        GeometryReader { geo in
            let total: CGFloat = max(0, geo.size.width - leadingInset)
            ZStack(alignment: .topLeading) {
                ForEach(bars.filter { $0.lane < visibleLanes }) { bar in
                    let color = model.color(for: bar.occurrence.calendarId)
                    // Getrennt berechnet: Als ein Ausdruck sprengt das den Type-Checker (Xcode 26.6, Release).
                    let span = CGFloat(widths.width(from: bar.firstColumn, through: bar.lastColumn))
                    let width: CGFloat = max(0, total * span - 4)
                    let x: CGFloat = leadingInset + total * CGFloat(widths.leading(bar.firstColumn)) + 2
                    let y: CGFloat = CGFloat(bar.lane) * laneHeight
                    let isHighlighted: Bool = model.highlightedOccurrence?.id == bar.occurrence.id
                    Text(bar.occurrence.title)
                        .font(.caption2.weight(.medium))
                        .fontWidth(.condensed)
                        .foregroundStyle(Theme.text1)
                        .lineLimit(1)
                        .padding(.leading, 6)
                        .padding(.trailing, 4)
                        .frame(width: width, height: laneHeight - 2, alignment: .leading)
                        .background { AllDayBarShape(color: color) }
                        .modifier(HighlightBorder(isOn: isHighlighted))
                        .offset(x: x, y: y)
                        .modifier(OccurrenceTap(occurrence: bar.occurrence)) // LIC-24
                        .contextMenu { OccurrenceMenu(occurrence: bar.occurrence) }
                }
                ForEach(0..<columns, id: \.self) { column in
                    if hidden[column] > 0 {
                        Text("+\(hidden[column])")
                            .font(Theme.digits(.caption2, weight: .semibold))
                            .foregroundStyle(Theme.text2)
                            .offset(x: leadingInset + total * CGFloat(widths.leading(column)) + 4, y: CGFloat(lanes) * laneHeight)
                    }
                }
            }
        }
        .frame(height: CGFloat(lanes) * laneHeight + (hidden.contains { $0 > 0 } ? 14 : 0) + 4)
    }
}

/// Hintergrund eines Balkens: Kalenderfarbe auf dem Grund, Radius 3.
private struct AllDayBarShape: View {
    let color: Color

    var body: some View {
        RoundedRectangle(cornerRadius: 3, style: .continuous).fill(Theme.eventFill(color))
    }
}

/// Termine hinter „+n“ im Fokus-Tag als Liste. Ein Tipp öffnet das Detail,
/// sobald das Popover zu ist (`onOpen`); Rechtsklick bietet das Menü des Termins.
private struct OverflowList: View {
    let model: AppModel
    let occurrences: [Occurrence]
    let onOpen: (Occurrence) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(occurrences) { occ in
                Button { onOpen(occ) } label: {
                    HStack(spacing: 8) {
                        CalendarBar(color: model.color(for: occ.calendarId), width: 3)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(occ.title)
                                .font(.callout.weight(.medium))
                                .foregroundStyle(Theme.text1)
                            Text(AgendaBuilder.timeText(occ, calendar: model.calendar))
                                .font(Theme.digits(.caption))
                                .foregroundStyle(Theme.text2)
                        }
                        .lineLimit(1)
                        Spacer(minLength: 0)
                    }
                    .padding(.vertical, 4)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .contextMenu { OccurrenceMenu(occurrence: occ) }
            }
        }
        .padding(12)
        .frame(minWidth: 220, alignment: .leading)
        .environment(model)
    }
}
