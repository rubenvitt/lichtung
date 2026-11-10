import LichtungKit
import SwiftUI
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Gestaltung der Apps an einer Stelle: Farben, Schrift, Maße.
///
/// Leitidee: Heute ist die Lichtung. Die Oberfläche hat keine eigene
/// Akzentfarbe; Farbe kommt nur von den Kalendern. Alle Farben haben eine
/// helle und eine dunkle Fassung (Spec §4).
enum Theme {
    // MARK: Farben

    /// Grund: Fenster und alle Tage außer heute.
    static let shade = Color(light: 0xECEEE8, dark: 0x1B1D1B)
    /// Licht-Fläche: Spalte von heute.
    static let lit = Color(light: 0xFFFFFF, dark: 0xF4E9C9, darkAlpha: 0.075)
    /// Licht: Zahl von heute, Hauptknopf, Auswahl.
    static let light = Color(light: 0x1B1D1A, dark: 0xF4E9C9)
    /// Schrift auf `light`.
    static let onLight = Color(light: 0xFFFFFF, dark: 0x1B1D1B)
    /// Text in drei Rängen. `text3` nur für Stundenzahlen und Linienbeschriftung.
    static let text1 = Color(light: 0x1B1D1A, dark: 0xECEBE4)
    static let text2 = Color(light: 0x5F635B, dark: 0x9A9C93)
    static let text3 = Color(light: 0xA1A49C, dark: 0x63655E)
    /// Sonne: Jetzt-Linie und ihre Uhrzeit, sonst nichts.
    static let sun = Color(light: 0xC58500, dark: 0xF0B840)
    /// Stundenlinien und Trennungen.
    static let line = Color(light: 0x000000, dark: 0xFFFFFF, lightAlpha: 0.07, darkAlpha: 0.06)
    /// Schrift auf einem laufenden Termin (volle Kalenderfarbe): dunkel
    /// `#141513` oder weiß, je nachdem, was nach WCAG mehr Kontrast hat.
    /// Unbekannte Farbe → dunkel, passend zur grauen Ersatzfarbe.
    static func onRunning(for hex: String?) -> Color {
        let dark: UInt32 = 0x141513
        guard let calendar = hexValue(hex) else { return Color(light: dark, dark: dark) }
        let l = luminance(calendar)
        // Weiß hat die Leuchtdichte 1.
        let text: UInt32 = contrast(l, luminance(dark)) >= contrast(l, 1) ? dark : 0xFFFFFF
        return Color(light: text, dark: text)
    }

    /// Relative Leuchtdichte nach WCAG 2.x (sRGB linearisiert, 0…1).
    static func luminance(_ value: UInt32) -> Double {
        func linear(_ c: Double) -> Double { c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4) }
        let (r, g, b) = rgb(value)
        return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    /// Kontrastverhältnis nach WCAG (1…21) zweier Leuchtdichten.
    static func contrast(_ a: Double, _ b: Double) -> Double {
        (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }

    /// `#RRGGBB` aus der API als `0xRRGGBB`; sonst `nil`.
    fileprivate static func hexValue(_ hex: String?) -> UInt32? {
        var text = (hex ?? "").trimmingCharacters(in: .whitespaces)
        if text.hasPrefix("#") { text.removeFirst() }
        guard text.count == 6 else { return nil }
        return UInt32(text, radix: 16)
    }

    /// Fläche eines kommenden Termins: Kalenderfarbe deckend auf `shade`
    /// gemischt, dunkel 30 %, hell 20 % (wie `color-mix(in srgb …)` im Entwurf).
    static func eventFill(_ color: Color) -> Color {
        // Kalenderfarben sind feste Hex-Werte, also in jedem Modus gleich.
        let c = color.resolve(in: EnvironmentValues())
        let calendar = (Double(c.red), Double(c.green), Double(c.blue))
        return Color(dynamic: { isDark in
            let base = rgb(isDark ? 0x1B1D1B : 0xECEEE8)
            let amount = isDark ? 0.3 : 0.2
            func mix(_ a: Double, _ b: Double) -> Double { amount * a + (1 - amount) * b }
            return (mix(calendar.0, base.0), mix(calendar.1, base.1), mix(calendar.2, base.2), 1)
        })
    }

    /// `0xRRGGBB` als sRGB-Anteile 0…1.
    fileprivate static func rgb(_ value: UInt32) -> (Double, Double, Double) {
        (Double((value >> 16) & 0xFF) / 255, Double((value >> 8) & 0xFF) / 255, Double(value & 0xFF) / 255)
    }

    // MARK: Zustand eines Termins

    enum EventState { case past, upcoming, running }

    /// Ganztägige Termine laufen nie (`isRunning`), sie sind kommend oder vorbei.
    static func state(of occ: Occurrence, now: Date) -> EventState {
        if occ.isRunning(now: now) { return .running }
        if occ.isPast(now: now) { return .past }
        return .upcoming
    }

    // MARK: Schrift

    /// Ziffern (Uhrzeiten, Tageszahlen): Systemschrift, gleich breit.
    static func digits(_ style: Font.TextStyle, weight: Font.Weight = .regular) -> Font {
        .system(style, weight: weight).monospacedDigit()
    }

    // MARK: Maße

    static let blockRadius: CGFloat = 4
    static let buttonRadius: CGFloat = 7
    static let cardRadius: CGFloat = 14

    // MARK: Knöpfe

    /// Hauptknopf (Anmelden, Beitreten, +): Fläche `light`, Schrift `onLight`.
    /// `.controlSize(.small)` gibt die kleine Fassung (Liste, Raster).
    struct PrimaryButtonStyle: ButtonStyle {
        @Environment(\.isEnabled) private var isEnabled
        @Environment(\.controlSize) private var controlSize

        func makeBody(configuration: Configuration) -> some View {
            let small = controlSize == .small || controlSize == .mini
            configuration.label
                .font(small ? .caption.weight(.semibold) : .body.weight(.semibold))
                .foregroundStyle(Theme.onLight)
                .padding(.horizontal, small ? 8 : 12)
                .padding(.vertical, small ? 3 : 6)
                .background(RoundedRectangle(cornerRadius: Theme.buttonRadius, style: .continuous).fill(Theme.light))
                .opacity(isEnabled ? (configuration.isPressed ? 0.8 : 1) : 0.4)
                .contentShape(RoundedRectangle(cornerRadius: Theme.buttonRadius, style: .continuous))
        }
    }
}

extension Color {
    /// `#RRGGBB` aus der API; unbekannt → Grau.
    init(hex: String?) {
        guard let value = Theme.hexValue(hex) else {
            self = .gray
            return
        }
        let (r, g, b) = Theme.rgb(value)
        self = Color(red: r, green: g, blue: b)
    }

    /// Farbe mit eigener Fassung für den dunklen Modus (`0xRRGGBB`).
    init(light: UInt32, dark: UInt32, lightAlpha: Double = 1, darkAlpha: Double = 1) {
        self.init(dynamic: { isDark in
            let (r, g, b) = Theme.rgb(isDark ? dark : light)
            return (r, g, b, isDark ? darkAlpha : lightAlpha)
        })
    }

    /// Farbe, die je Modus neu berechnet wird (sRGB-Anteile und Deckung, 0…1).
    init(dynamic rgba: @escaping @Sendable (_ isDark: Bool) -> (Double, Double, Double, Double)) {
        #if os(macOS)
        let color = NSColor(name: nil) { appearance in
            let (r, g, b, a) = rgba(appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua)
            return NSColor(srgbRed: r, green: g, blue: b, alpha: a)
        }
        self.init(nsColor: color)
        #else
        let color = UIColor { traits in
            let (r, g, b, a) = rgba(traits.userInterfaceStyle == .dark)
            return UIColor(red: r, green: g, blue: b, alpha: a)
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

/// Monat halbfett, Jahr gedämpft daneben („November 2026“).
struct MonthTitle: View {
    let year: Int
    let month: Int
    var style: Font.TextStyle = .title2

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(Self.monthName(month))
                .font(.system(style, weight: .semibold))
                .foregroundStyle(Theme.text1)
            Text(verbatim: "\(year)")
                .font(.system(style))
                .foregroundStyle(Theme.text2)
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

/// Kopf einer Gruppe in Seitenleisten und Listen („Als Nächstes“): eine halbfette Zeile.
struct SectionLabel: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(Theme.text1)
    }
}

extension Occurrence {
    /// Liegt ganz in der Vergangenheit.
    func isPast(now: Date) -> Bool { end <= now && start < now }

    /// Läuft gerade (nicht ganztägig).
    func isRunning(now: Date) -> Bool { !isAllDay && start <= now && now < end }
}
