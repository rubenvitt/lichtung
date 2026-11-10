import Foundation
@testable import LichtungKit
import Testing

/// Ungleiche Spaltenbreiten: der Fokus-Tag ist 2,1-mal so breit wie die übrigen Tage.
@Suite("Spaltenbreiten mit Fokus-Tag")
struct ColumnWidthsTests {
    @Test func sevenColumnsWithFocus() {
        let w = ColumnWidths(columns: 7, focus: 1)
        #expect(abs(w.fractions.reduce(0, +) - 1) < 1e-9)
        #expect(abs(w.fractions[1] / w.fractions[0] - 2.1) < 1e-9)
        #expect(abs(w.leading(2) - (1 + 2.1) / 8.1) < 1e-9)
    }

    @Test func noFocusIsEven() {
        let w = ColumnWidths(columns: 7, focus: nil)
        for fraction in w.fractions {
            #expect(abs(fraction - 1.0 / 7.0) < 1e-9)
        }
        // Außerhalb des Bereichs verhält sich ein Fokus wie kein Fokus.
        #expect(ColumnWidths(columns: 7, focus: 9) == w)
    }

    @Test func singleColumn() {
        #expect(ColumnWidths(columns: 1, focus: 0).fractions == [1])
    }

    @Test func noColumnsIsEmptyAndSafe() {
        let w = ColumnWidths(columns: 0, focus: 0)
        #expect(w.fractions.isEmpty)
        #expect(w.column(atFraction: 0.5) == 0)
        #expect(ColumnWidths(columns: -3, focus: nil).fractions.isEmpty)
    }

    @Test func columnAtFractionClamps() {
        let w = ColumnWidths(columns: 7, focus: 1)
        #expect(w.column(atFraction: -0.2) == 0)
        #expect(w.column(atFraction: 1.5) == 6)
        // Genau auf der Grenze gehört der Punkt zur rechten Spalte.
        #expect(w.column(atFraction: w.leading(3)) == 3)
    }

    @Test func dayOffsetAcrossFocus() {
        let w = ColumnWidths(columns: 7, focus: 1)
        // Zeiger mitten in Spalte 2.
        let pointer = (w.leading(2) + w.leading(3)) / 2
        #expect(w.dayOffset(fromColumn: 0, pointerFraction: pointer) == 2)
    }

    @Test func widthAcrossAllIsOne() {
        let w = ColumnWidths(columns: 7, focus: 1)
        #expect(abs(w.width(from: 0, through: 6) - 1) < 1e-9)
        #expect(abs(w.width(from: 1, through: 1) - 2.1 / 8.1) < 1e-9)
    }
}
