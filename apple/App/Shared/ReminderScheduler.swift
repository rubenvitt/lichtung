// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import Foundation
import LichtungKit
import Observation
import UserNotifications

/// Lokale Benachrichtigungen für Termin-Erinnerungen (LIC-68).
///
/// Welche Erinnerungen fällig sind, rechnet der Kern aus (`ReminderPlanner`);
/// hier wird nur die Differenz zu den ausstehenden Benachrichtigungen
/// eingeplant. Ohne APNs: Termine, die die App nie gesehen hat, erinnern erst,
/// wenn sie einmal geöffnet war (LIC-27).
@MainActor
@Observable
final class ReminderScheduler {
    static let shared = ReminderScheduler()
    /// `@AppStorage`-Schlüssel des Schalters in den Einstellungen (Standard: an).
    static let enabledKey = "remindersEnabled"
    private nonisolated static let prefix = "lichtung.reminder."

    enum Permission: Equatable {
        case unknown, allowed, denied
    }

    /// Ob das System Mitteilungen erlaubt (für den Hinweis in den Einstellungen).
    private(set) var permission: Permission = .unknown
    /// Getippte Benachrichtigung, die die Kalenderansicht noch öffnen soll.
    var openRequest: OpenRequest?

    struct OpenRequest: Equatable {
        var eventId: String
        var occurrenceId: String
        var start: Date
    }

    @ObservationIgnored private let center = UNUserNotificationCenter.current()
    @ObservationIgnored private let delegate = ReminderNotificationDelegate()
    @ObservationIgnored private var pending: Task<Void, Never>?
    @ObservationIgnored private var latest: CalendarQuery?
    /// Zählt jede Anforderung mit. Ein älterer Durchlauf, der noch plant oder
    /// auf die Erlaubnis wartet, darf einen neueren nicht überschreiben und
    /// nach `removeAll()` nichts mehr einplanen.
    @ObservationIgnored private var generation = 0
    /// Letzter Abgleich mit dem System. Abgleiche laufen nacheinander: Ein
    /// neuer (auch das Leeren bei `removeAll()`) beginnt erst, wenn der
    /// vorige fertig ist oder wegen eines neueren Zählers aufgehört hat.
    @ObservationIgnored private var applying: Task<Void, Never>?

    private init() {}

    static var isEnabled: Bool {
        UserDefaults.standard.object(forKey: enabledKey) as? Bool ?? true
    }

    /// Früh beim Start aufrufen, damit ein Tipp auf eine Benachrichtigung,
    /// die die App startet, ankommt.
    func install() {
        center.delegate = delegate
    }

    /// Neu einplanen, kurz verzögert: Bei vielen Änderungen hintereinander
    /// (erster Abgleich, SSE) zählt nur der letzte Stand.
    func update(_ query: CalendarQuery) {
        latest = query
        generation += 1
        pending?.cancel()
        pending = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(600))
            guard !Task.isCancelled else { return }
            await self?.reschedule()
        }
    }

    /// Alles entfernen (Abmelden, Demo, Schalter aus).
    func removeAll() {
        pending?.cancel()
        pending = nil
        latest = nil
        generation += 1
        let token = generation
        Task { await self.apply(wanted: [], ifCurrent: token) }
    }

    private func reschedule() async {
        guard let query = latest else { return }
        let token = generation
        guard Self.isEnabled else {
            await apply(wanted: [], ifCurrent: token)
            return
        }
        let now = Date()
        let calendar = Calendar.lichtung(timeZone: query.expander.deviceTimeZone)
        let wanted = await Task.detached(priority: .utility) {
            ReminderPlanner.plan(query, now: now, calendar: calendar)
        }.value
        if !wanted.isEmpty {
            let allowed = await ensurePermission()
            guard allowed else { return }
        }
        await apply(wanted: wanted, ifCurrent: token)
    }

    /// Fragt beim ersten Mal nach der Erlaubnis.
    private func ensurePermission() async -> Bool {
        switch await Self.authorizationStatus() {
        case .notDetermined:
            let granted = await Self.requestAuthorization()
            permission = granted ? .allowed : .denied
            return granted
        case .denied:
            permission = .denied
            return false
        default:
            permission = .allowed
            return true
        }
    }

    /// Aktualisiert `permission`, ohne zu fragen (Einstellungen).
    func refreshPermission() async {
        switch await Self.authorizationStatus() {
        case .notDetermined: permission = .unknown
        case .denied: permission = .denied
        default: permission = .allowed
        }
    }

    // Die Abfragen beim System laufen außerhalb des Main Actors und geben nur
    // Sendable-Werte zurück (Swift 6, strikte Prüfung bleibt an).

    private nonisolated static func authorizationStatus() async -> UNAuthorizationStatus {
        await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
    }

    private nonisolated static func requestAuthorization() async -> Bool {
        (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])) ?? false
    }

    /// Plant eine Erinnerung als Benachrichtigung ein; die Anfrage entsteht
    /// erst hier, damit nur `PlannedReminder` (Sendable) die Grenze kreuzt.
    private nonisolated static func schedule(_ reminder: PlannedReminder) async {
        let interval = reminder.fireDate.timeIntervalSinceNow
        guard interval >= 1 else { return }
        let content = UNMutableNotificationContent()
        content.title = reminder.title
        content.body = reminder.body
        content.sound = .default
        content.threadIdentifier = reminder.eventId
        content.userInfo = [
            "eventId": reminder.eventId,
            "occurrenceId": reminder.occurrenceId,
            "fire": reminder.fireDate.timeIntervalSince1970,
        ]
        let trigger = UNTimeIntervalNotificationTrigger(timeInterval: interval, repeats: false)
        let request = UNNotificationRequest(identifier: prefix + reminder.id, content: content, trigger: trigger)
        try? await UNUserNotificationCenter.current().add(request)
    }

    private nonisolated static func scheduledReminders() async -> [PlannedReminder] {
        await UNUserNotificationCenter.current().pendingNotificationRequests().compactMap(reminder(from:))
    }

    /// Gleicht die ausstehenden Benachrichtigungen ab, solange `token` noch
    /// die letzte Anforderung ist.
    private func apply(wanted: [PlannedReminder], ifCurrent token: Int) async {
        let previous = applying
        let task = Task { [weak self] in
            await previous?.value
            await self?.reconcile(wanted: wanted, ifCurrent: token)
        }
        applying = task
        await task.value
    }

    private func reconcile(wanted: [PlannedReminder], ifCurrent token: Int) async {
        guard token == generation else { return }
        let scheduled = await Self.scheduledReminders()
        guard token == generation else { return }
        let (remove, add) = ReminderPlanner.diff(scheduled: scheduled, wanted: wanted)
        if !remove.isEmpty {
            center.removePendingNotificationRequests(withIdentifiers: remove.map { Self.prefix + $0 })
        }
        for reminder in add {
            // Kam inzwischen eine neuere Anforderung (oder `removeAll()`),
            // hört dieser Abgleich hier auf; der nächste räumt danach auf.
            guard token == generation else { return }
            await Self.schedule(reminder)
        }
    }

    /// Rückweg aus einer ausstehenden Benachrichtigung, damit `diff` vergleicht.
    private nonisolated static func reminder(from request: UNNotificationRequest) -> PlannedReminder? {
        guard request.identifier.hasPrefix(prefix) else { return nil }
        let info = request.content.userInfo
        guard let eventId = info["eventId"] as? String,
              let occurrenceId = info["occurrenceId"] as? String,
              let fire = info["fire"] as? Double else { return nil }
        return PlannedReminder(
            id: String(request.identifier.dropFirst(prefix.count)),
            fireDate: Date(timeIntervalSince1970: fire),
            title: request.content.title,
            body: request.content.body,
            eventId: eventId,
            occurrenceId: occurrenceId
        )
    }

    fileprivate func opened(eventId: String, occurrenceId: String, fire: Date) {
        openRequest = OpenRequest(eventId: eventId, occurrenceId: occurrenceId, start: fire)
    }
}

/// Zeigt Erinnerungen auch bei offener App und leitet einen Tipp an die
/// Kalenderansicht weiter.
private final class ReminderNotificationDelegate: NSObject, UNUserNotificationCenterDelegate, Sendable {
    func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let info = response.notification.request.content.userInfo
        guard let eventId = info["eventId"] as? String,
              let occurrenceId = info["occurrenceId"] as? String,
              let fire = info["fire"] as? Double else { return }
        await MainActor.run {
            ReminderScheduler.shared.opened(eventId: eventId, occurrenceId: occurrenceId, fire: Date(timeIntervalSince1970: fire))
        }
    }
}
