import Foundation

/// Eine Änderung, wie die Oberfläche sie meint: ohne Basisversion und mit
/// Event-IDs, die auch lokale Platzhalter sein dürfen. Erst beim Ausführen
/// wird daraus eine Mutation gegen den dann aktuellen Stand.
///
/// Grundlage für Rückgängig/Wiederholen: Eine Absicht, die zehn Sekunden
/// später zurückgenommen wird, muss gegen die Version gelten, die dann gilt.
public enum EditIntent: Hashable, Sendable {
    case create(calendarId: String, event: EventInput)
    case update(eventId: String, scope: EditScope = .all, occurrence: DateValue? = nil, changes: EventChanges)
    case delete(eventId: String, scope: EditScope = .all, occurrence: DateValue? = nil)
    case respond(eventId: String, occurrence: DateValue? = nil, status: InvitationResponse)

    /// Mutation gegen `state`: aktuelle Event-ID und Basisversion.
    public func operation(in state: SyncState) -> Mutation.Operation {
        switch self {
        case let .create(cal, event):
            return .createEvent(calendarId: cal, event: event)
        case let .update(id, scope, occ, changes):
            let current = state.currentId(for: id)
            return .updateEvent(eventId: current, baseVersion: state.baseVersion(of: current), scope: scope, occurrence: occ, changes: changes)
        case let .delete(id, scope, occ):
            let current = state.currentId(for: id)
            return .deleteEvent(eventId: current, baseVersion: state.baseVersion(of: current), scope: scope, occurrence: occ)
        case let .respond(id, occ, status):
            return .respond(eventId: state.currentId(for: id), occurrence: occ, status: status, comment: nil)
        }
    }

    /// Absicht, die diese hier zurücknimmt. `events` ist der sichtbare Stand
    /// *vor* der Ausführung, `createdId` die ID eines neu angelegten Events.
    /// `nil`, wenn sich die Änderung nicht sauber zurücknehmen lässt.
    public func inverse(before events: [String: Event], createdId: String?, deviceTimeZone tz: TimeZone) -> EditIntent? {
        switch self {
        case .create:
            return createdId.map { .delete(eventId: $0) }

        case let .update(id, scope, occ, changes):
            guard let old = events[id] else { return nil }
            switch (scope, occ) {
            case (.all, _), (_, nil):
                var back = EventChanges.reverting(changes, from: old.details)
                if changes.recurrence != nil { back.recurrence = old.recurrence.map { .set($0) } ?? .clear }
                if changes.calendarId != nil { back.calendarId = old.calendarId }
                return .update(eventId: id, scope: .all, changes: back)
            case (.this, let occ?):
                let before = OptimisticApply.overrideDetails(of: old, at: occ, tz: tz)
                return .update(eventId: id, scope: .this, occurrence: occ, changes: EventChanges.reverting(changes, from: before))
            case (.thisAndFollowing, _):
                // Der Server teilt die Serie; das neue Event kennt erst der Server.
                return nil
            }

        case let .delete(id, scope, occ):
            guard let old = events[id] else { return nil }
            switch (scope, occ) {
            case (.all, _), (_, nil):
                // Neu anlegen. Geänderte Vorkommen und Teilnehmer-Antworten
                // gehen dabei verloren; die UID bleibt, damit Einladungen passen.
                var input = EventInput(copying: old.details, recurrence: old.recurrence)
                input.uid = old.uid
                return .create(calendarId: old.calendarId, event: input)
            case (.this, _), (.thisAndFollowing, _):
                guard let r = old.recurrence else { return nil }
                return .update(eventId: id, scope: .all, changes: EventChanges(recurrence: .set(r)))
            }

        case let .respond(id, occ, _):
            guard let old = events[id] else { return nil }
            let previous = occ.flatMap { o in old.overrides.first { $0.recurrenceId.normalized == o.normalized }?.details.selfStatus }
                ?? old.details.selfStatus
            switch previous {
            case .accepted?: return .respond(eventId: id, occurrence: occ, status: .accepted)
            case .tentative?: return .respond(eventId: id, occurrence: occ, status: .tentative)
            case .declined?: return .respond(eventId: id, occurrence: occ, status: .declined)
            // „Noch offen“ lässt sich beim Anbieter nicht wieder setzen.
            default: return nil
            }
        }
    }
}

extension EventChanges {
    /// Änderung, die die in `changes` genannten Felder auf den Stand von `old` zurücksetzt.
    static func reverting(_ changes: EventChanges, from old: EventDetails) -> EventChanges {
        func patch<T>(_ value: T?) -> Patch<T> { value.map { .set($0) } ?? .clear }
        var back = EventChanges()
        if changes.title != nil { back.title = old.title }
        if changes.location != nil { back.location = patch(old.location) }
        if changes.notes != nil { back.notes = patch(old.notes) }
        if changes.url != nil { back.url = patch(old.url) }
        if changes.start != nil { back.start = old.start }
        if changes.end != nil { back.end = old.end }
        if changes.status != nil { back.status = old.status }
        if changes.availability != nil { back.availability = old.availability }
        if changes.attendees != nil { back.attendees = old.attendees.map { AttendeeInput(email: $0.email, name: $0.name, role: $0.role) } }
        if changes.alarms != nil { back.alarms = old.alarms }
        if changes.travelTimeMinutes != nil { back.travelTimeMinutes = patch(old.travelTimeMinutes) }
        return back
    }
}

/// Verlauf für Rückgängig und Wiederholen (⌘Z / ⇧⌘Z, Schütteln am iPhone).
///
/// Jeder Eintrag ist die Absicht, die beim nächsten Rückgängig ausgeführt
/// wird. Beim Ausführen wird ihre eigene Umkehrung berechnet und landet auf
/// dem anderen Stapel. So bleiben beide Richtungen gegen den aktuellen Stand gültig.
public struct UndoHistory: Hashable, Sendable {
    public struct Entry: Hashable, Sendable {
        /// Was die Aktion war, z. B. „Termin verschieben“. Für „„… widerrufen““.
        public var label: String
        public var intent: EditIntent
    }

    public private(set) var undoStack: [Entry] = []
    public private(set) var redoStack: [Entry] = []
    public var limit = 50

    public init() {}

    public var canUndo: Bool { !undoStack.isEmpty }
    public var canRedo: Bool { !redoStack.isEmpty }
    public var undoLabel: String? { undoStack.last?.label }
    public var redoLabel: String? { redoStack.last?.label }

    /// Neue Aktion der Person: Umkehrung merken, Wiederholen verwerfen.
    mutating func recordNew(label: String, inverse: EditIntent?) {
        redoStack.removeAll()
        guard let inverse else {
            // Nicht umkehrbar: Älteres Rückgängig würde auf einem anderen Stand aufsetzen.
            undoStack.removeAll()
            return
        }
        Self.push(&undoStack, Entry(label: label, intent: inverse), limit: limit)
    }

    mutating func popUndo() -> Entry? { undoStack.popLast() }
    mutating func popRedo() -> Entry? { redoStack.popLast() }

    mutating func pushRedo(_ e: Entry) { Self.push(&redoStack, e, limit: limit) }
    mutating func pushUndo(_ e: Entry) { Self.push(&undoStack, e, limit: limit) }

    private static func push(_ stack: inout [Entry], _ e: Entry, limit: Int) {
        stack.append(e)
        if stack.count > limit { stack.removeFirst(stack.count - limit) }
    }
}

extension SyncEngine {
    /// Führt eine Änderung der Person aus und merkt sie für Rückgängig.
    /// Gibt die ID des betroffenen Events zurück (beim Anlegen die lokale).
    @discardableResult
    public func perform(_ intent: EditIntent, label: String) -> String? {
        let (id, inverse) = execute(intent)
        undoHistory.recordNew(label: label, inverse: inverse)
        return id
    }

    @discardableResult
    public func undo() -> Bool {
        guard let entry = undoHistory.popUndo() else { return false }
        let (_, inverse) = execute(entry.intent)
        if let inverse { undoHistory.pushRedo(.init(label: entry.label, intent: inverse)) }
        return true
    }

    @discardableResult
    public func redo() -> Bool {
        guard let entry = undoHistory.popRedo() else { return false }
        let (_, inverse) = execute(entry.intent)
        if let inverse { undoHistory.pushUndo(.init(label: entry.label, intent: inverse)) }
        return true
    }

    private func execute(_ intent: EditIntent) -> (String?, EditIntent?) {
        let before = visibleEvents()
        let id = enqueue(intent.operation(in: state))
        let createdId: String? = if case .create = intent { id } else { nil }
        return (id, intent.inverse(before: before, createdId: createdId, deviceTimeZone: deviceTimeZone))
    }
}
