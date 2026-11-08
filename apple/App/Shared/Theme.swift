// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Gestaltung der Apps an einer Stelle: Farben, Schrift, Maße.
///
/// Bild dahinter ist die Lichtung selbst: Waldgrün als Akzent (Auswahl,
/// heute, Knöpfe), warmes Licht für „jetzt“ (Jetzt-Linie, laufende Termine).
/// Alle Farben haben eine helle und eine dunkle Fassung.
enum Theme {
    // MARK: Farben

    /// Waldgrün: Auswahl, heute, Hauptknöpfe.
    static let accent = Color(light: 0x2E7D5B, dark: 0x5FD39A)
    /// Schrift und Symbole auf `accent`: hell weiß (5:1), dunkel fast
    /// schwarz, weil das helle Grün weiße Schrift nicht trägt (8,8:1).
    static let onAccent = Color(light: 0xFFFFFF, dark: 0x0B2419)
    /// Dunkleres Grün für Verläufe (Anmeldung, Marke).
    static let accentDeep = Color(light: 0x1B4D3A, dark: 0x2E7D5B)
    /// Warmes Licht: Jetzt-Linie, „Jetzt“-Marke.
    static let now = Color(light: 0xC2410C, dark: 0xFF7A50)
    /// Schrift auf `now`: hell weiß (5,2:1), dunkel fast schwarz (7:1).
    static let onNow = Color(light: 0xFFFFFF, dark: 0x2A0E05)
    /// Sonnengelb für Hinweise (Demo-Modus).
    static let sun = Color(light: 0xD99A1E, dark: 0xF2B84B)
    /// Feine Linien im Raster und im Monat.
    static let hairline = Color.primary.opacity(0.08)
    /// Noch feinere Linie (halbe Stunden).
    static let hairlineFaint = Color.primary.opacity(0.04)
    /// Wochenende und Tage außerhalb des Monats.
    static let wash = Color.primary.opacity(0.03)

    /// Füllung eines Terminblocks in Kalenderfarbe.
    static func eventFill(_ color: Color, past: Bool = false) -> Color {
        color.opacity(past ? 0.12 : 0.22)
    }

    // MARK: Schrift

    /// Große Titel (Monat, Anmeldung): Serifenschrift, damit die App ein
    /// eigenes Gesicht hat.
    static func display(_ style: Font.TextStyle, weight: Font.Weight = .bold) -> Font {
        .system(style, design: .serif, weight: weight)
    }

    /// Zahlen (Tage, Uhrzeiten): gerundet und gleich breit.
    static func number(_ style: Font.TextStyle, weight: Font.Weight = .regular) -> Font {
        .system(style, design: .rounded, weight: weight).monospacedDigit()
    }

    // MARK: Maße

    static let blockRadius: CGFloat = 6
    static let cardRadius: CGFloat = 14
}

extension Color {
    /// `#RRGGBB` aus der API; unbekannt → Grau.
    init(hex: String?) {
        var text = (hex ?? "").trimmingCharacters(in: .whitespaces)
        if text.hasPrefix("#") { text.removeFirst() }
        guard text.count == 6, let value = Int(text, radix: 16) else {
            self = .gray
            return
        }
        self = Color(
            red: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255
        )
    }

    /// Farbe mit eigener Fassung für den dunklen Modus (`0xRRGGBB`).
    init(light: UInt32, dark: UInt32) {
        #if os(macOS)
        let color = NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            let rgb = isDark ? dark : light
            return NSColor(
                srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255,
                green: CGFloat((rgb >> 8) & 0xFF) / 255,
                blue: CGFloat(rgb & 0xFF) / 255,
                alpha: 1
            )
        }
        self.init(nsColor: color)
        #else
        let color = UIColor { traits in
            let rgb = traits.userInterfaceStyle == .dark ? dark : light
            return UIColor(
                red: CGFloat((rgb >> 16) & 0xFF) / 255,
                green: CGFloat((rgb >> 8) & 0xFF) / 255,
                blue: CGFloat(rgb & 0xFF) / 255,
                alpha: 1
            )
        }
        self.init(uiColor: color)
        #endif
    }
}

extension AppModel {
    func color(for calendarId: String) -> Color {
        Color(hex: colorHex(for: calendarId))
    }
}

/// Farbiger Punkt eines Kalenders.
struct CalendarDot: View {
    let color: Color
    var size: CGFloat = 8

    var body: some View {
        Circle().fill(color).frame(width: size, height: size)
    }
}

/// Senkrechter Streifen in Kalenderfarbe (Liste, Suche, Detail).
struct CalendarBar: View {
    let color: Color
    var width: CGFloat = 4

    var body: some View {
        Capsule().fill(color).frame(width: width)
    }
}

/// Kleine Marke „Jetzt“ für laufende Termine.
struct NowBadge: View {
    var body: some View {
        Text("JETZT")
            .font(.system(size: 9, weight: .heavy, design: .rounded))
            .tracking(0.6)
            .foregroundStyle(Theme.onNow)
            .padding(.horizontal, 5)
            .padding(.vertical, 2)
            .background(Capsule().fill(Theme.now))
            .accessibilityLabel("Läuft gerade")
    }
}

/// Monat in Serifen, Jahr gedämpft daneben („November 2026“).
struct MonthTitle: View {
    let year: Int
    let month: Int
    var style: Font.TextStyle = .title2

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(Self.monthName(month))
                .font(Theme.display(style))
            Text(verbatim: "\(year)")
                .font(Theme.display(style, weight: .regular))
                .foregroundStyle(Theme.accent)
        }
        .lineLimit(1)
        .accessibilityElement(children: .combine)
    }

    static func monthName(_ month: Int) -> String {
        let names = [
            "Januar", "Februar", "März", "April", "Mai", "Juni",
            "Juli", "August", "September", "Oktober", "November", "Dezember",
        ]
        return names.indices.contains(month - 1) ? names[month - 1] : ""
    }
}

/// Kopf einer Gruppe in Seitenleisten und Listen („ALS NÄCHSTES“).
struct SectionLabel: View {
    let text: String

    var body: some View {
        Text(text.uppercased())
            .font(.caption2.weight(.bold))
            .tracking(0.8)
            .foregroundStyle(.secondary)
    }
}

extension Occurrence {
    /// Liegt ganz in der Vergangenheit.
    func isPast(now: Date) -> Bool { end <= now && start < now }

    /// Läuft gerade (nicht ganztägig).
    func isRunning(now: Date) -> Bool { !isAllDay && start <= now && now < end }
}
