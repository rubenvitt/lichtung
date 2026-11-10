import Foundation

/// Spaltenbreiten der Wochenansicht: Der Fokus-Tag ist `focusWeight`-mal so breit
/// wie jeder andere Tag. Die Anteile summieren sich zu 1, die Ansicht rechnet
/// damit in Bruchteilen der Gesamtbreite.
public struct ColumnWidths: Hashable, Sendable {
    public static let focusWeight = 2.1

    /// Anteil jeder Spalte an der Gesamtbreite, Summe 1. Leer bei `columns <= 0`.
    public let fractions: [Double]

    /// Ohne gültigen `focus` sind alle Spalten gleich breit.
    public init(columns: Int, focus: Int?) {
        var weights = Array(repeating: 1.0, count: max(columns, 0))
        if let focus, weights.indices.contains(focus) {
            weights[focus] = Self.focusWeight
        }
        let total = weights.reduce(0, +)
        fractions = weights.map { $0 / total }
    }

    /// Anteil links von `column`.
    public func leading(_ column: Int) -> Double {
        fractions.prefix(max(column, 0)).reduce(0, +)
    }

    /// Anteil der Spalten `first` bis `last` (jeweils einschließlich), auf den Bereich begrenzt.
    public func width(from first: Int, through last: Int) -> Double {
        let lower = max(first, 0)
        let upper = min(last, fractions.count - 1)
        guard lower <= upper else { return 0 }
        return fractions[lower...upper].reduce(0, +)
    }

    /// Spalte unter dem Anteil `x`, auf `0..<columns` begrenzt. Auf einer Grenze gewinnt die rechte Spalte.
    public func column(atFraction x: Double) -> Int {
        guard fractions.count > 1 else { return 0 }
        return (1..<fractions.count).filter { x >= leading($0) }.count
    }

    /// Tagesversatz beim Ziehen: Spalte unter dem Zeiger minus Ausgangsspalte.
    public func dayOffset(fromColumn column: Int, pointerFraction x: Double) -> Int {
        self.column(atFraction: x) - column
    }
}
