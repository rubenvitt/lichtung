import Foundation

/// iPhone: Halten auf einem ziehbaren Block (LIC-54). Nach dem kurzen Halten
/// ist der Block aufgenommen; wer ihn dann bewegt, verschiebt ihn, wer ohne
/// Bewegung loslässt, bekommt das Menü des Termins (Kopieren, Duplizieren …).
extension GridInteraction {
    /// Strecke in Punkten, die der Finger nach dem Aufnehmen höchstens
    /// wandern darf, damit Loslassen das Menü öffnet (wie die erlaubte
    /// Bewegung beim langen Drücken unter iOS).
    public static let holdMenuSlop: Double = 10

    /// Ob Loslassen nach dem Aufnehmen das Menü öffnet statt zu verschieben:
    /// Der Finger ist kaum gewandert, und der Termin würde sich nicht
    /// bewegen (unter einem halben Rasterschritt). `dy` enthält das
    /// automatische Scrollen am Rand; hat das Raster sich bewegt, war es ein Zug.
    public static func opensMenuOnRelease(dx: Double, dy: Double, columnHeight: Double) -> Bool {
        guard (dx * dx + dy * dy).squareRoot() < holdMenuSlop else { return false }
        return snapped(minutes(forDistance: dy, columnHeight: columnHeight)) == 0
    }
}

/// iPhone: Halten auf leerer Fläche im Raster (LIC-58). Kurz halten und
/// ziehen legt wie bisher einen Termin an (LIC-22). Liegt eine Kopie vor und
/// lässt der Finger ohne Bewegung los, öffnet sich stattdessen ein Menü mit
/// „Hier einfügen“ zur gehaltenen Uhrzeit. Ohne Kopie ändert sich nichts.
extension GridInteraction {
    /// Was nach dem Halten auf leerer Fläche beim Loslassen passiert.
    public enum SlotHoldRelease: Hashable, Sendable {
        /// Nichts (der Zug hat nach dem Halten nie begonnen, wie bisher).
        case nothing
        /// Termin aus dem Zug anlegen (Editor), wie bisher.
        case create
        /// Menü an der gehaltenen Stelle („Hier einfügen“, „Neuer Termin hier“).
        case menu
    }

    /// Entscheidung beim Loslassen. `translation` ist die Strecke seit dem
    /// Aufnehmen (`dy` mit dem automatischen Scrollen) oder `nil`, wenn der
    /// Zug nach dem Halten nie begonnen hat; dann fehlt auch die Stelle.
    /// `canPaste`: Eine Kopie liegt vor und die Zwischenablage hält sie noch.
    /// Das Menü kommt nur mit Kopie und nur, wenn der Finger kaum gewandert
    /// ist (`holdMenuSlop`, wie am Block); sonst bleibt es beim Anlegen.
    public static func slotHoldRelease(translation: (dx: Double, dy: Double)?, canPaste: Bool) -> SlotHoldRelease {
        guard let translation else { return .nothing }
        guard canPaste else { return .create }
        let distance = (translation.dx * translation.dx + translation.dy * translation.dy).squareRoot()
        return distance < holdMenuSlop ? .menu : .create
    }

    /// Beginn beim Einfügen an der Stelle `y` der Spalte: gerundet wie beim
    /// Anlegen per Ziehen (`creation`), also auf das Raster abgerundet und
    /// spätestens 23:45. Auf der Wanduhr-Achse; in der Lücke beim Vorstellen
    /// der erste Zeitpunkt danach.
    public static func pasteStart(day: LocalDate, atY y: Double, columnHeight: Double, calendar: Calendar) -> Date {
        creation(day: day, fromY: y, toY: y, columnHeight: columnHeight, calendar: calendar).start
    }
}

extension EventClipboard {
    /// Einfügen mit Beginn `start` (Raster am iPhone, LIC-58): Dauer bleibt,
    /// Einzeltermin ohne Teilnehmende (wie Duplizieren). Die Zeitzone des
    /// Termins bleibt, floating bleibt floating. Ganztägige Kopien landen
    /// ganztägig am Tag von `start` (in `deviceTimeZone`).
    public func intent(pastingAt start: Date, calendarId: String, deviceTimeZone tz: TimeZone) -> EditIntent {
        let calendar = Calendar.lichtung(timeZone: tz)
        if occurrence.details.start.isAllDay {
            return intent(pastingOn: LocalDate(start, calendar: calendar), calendarId: calendarId, calendar: calendar)
        }
        let newStart = GridInteraction.dateValue(start, like: occurrence.details.start, deviceTimeZone: tz)
        return .create(calendarId: calendarId, event: EventDuplicate.input(from: occurrence, movedTo: newStart, deviceTimeZone: tz))
    }

    /// Kopfzeile des Menüs beim Einfügen an `start`, z. B.
    /// „„Team-Sync“ einfügen um 14:30“; ganztägig wie `pasteTitle(on:)`.
    public func pasteTitle(at start: Date, calendar: Calendar) -> String {
        if occurrence.details.start.isAllDay {
            return pasteTitle(on: LocalDate(start, calendar: calendar))
        }
        let c = calendar.dateComponents([.hour, .minute], from: start)
        return "„\(occurrence.title)“ einfügen um \(c.hour ?? 0):" + String(format: "%02d", c.minute ?? 0)
    }
}
