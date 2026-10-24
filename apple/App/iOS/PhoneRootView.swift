// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI

/// iPhone: Liste (A) als Start, dazu Raster (B) und Monat (D) (Spec §5).
struct PhoneRootView: View {
    @Environment(AppModel.self) private var model

    enum Mode: String, CaseIterable, Identifiable {
        case list = "Liste", grid = "Raster", month = "Monat"
        var id: String { rawValue }
    }

    @State private var mode: Mode = .list
    @State private var gridDays = 1

    var body: some View {
        @Bindable var model = model
        NavigationStack {
            content
                .navigationTitle(MonthGrid.title(year: model.selectedDay.year, month: model.selectedDay.month))
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Heute") { model.selectedDay = model.today }
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        Menu {
                            Picker("Ansicht", selection: $mode) {
                                ForEach(Mode.allCases) { Text($0.rawValue).tag($0) }
                            }
                            if mode == .grid {
                                Picker("Tage", selection: $gridDays) {
                                    Text("Tag").tag(1)
                                    Text("3 Tage").tag(3)
                                    Text("Woche").tag(7)
                                }
                            }
                        } label: {
                            Image(systemName: "calendar")
                        }
                        .accessibilityLabel("Ansicht wählen")
                    }
                }
                .safeAreaInset(edge: .bottom) {
                    QuickEntryBar().background(.bar)
                }
        }
        .sheet(item: $model.selectedOccurrence) { occ in
            EventDetailView(occurrence: occ)
        }
    }

    @ViewBuilder
    private var content: some View {
        switch mode {
        case .list:
            AgendaView()
                .refreshable { await model.refresh() }
        case .grid:
            let start = gridDays == 7
                ? model.selectedDay.adding(days: -(model.selectedDay.weekday.rawValue - 1))
                : model.selectedDay
            DayGridView(start: start, dayCount: gridDays)
                .gesture(DragGesture(minimumDistance: 40).onEnded { value in
                    let step = value.translation.width < 0 ? gridDays : -gridDays
                    model.selectedDay = model.selectedDay.adding(days: step)
                })
        case .month:
            MonthView(year: model.selectedDay.year, month: model.selectedDay.month, visibleLanes: 2)
                .gesture(DragGesture(minimumDistance: 40).onEnded { value in
                    let step = value.translation.height < 0 ? 1 : -1
                    model.selectedDay = LocalDate(year: model.selectedDay.year, month: model.selectedDay.month, day: 1).adding(months: step)
                })
        }
    }
}
