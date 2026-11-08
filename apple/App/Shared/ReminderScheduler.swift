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
        Task { await self.apply(wanted: []) }
    }

    private func reschedule() async {
        guard let query = latest else { return }
        guard Self.isEnabled else {
            await apply(wanted: [])
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
        await apply(wanted: wanted)
    }

    /// Fragt beim ersten Mal nach der Erlaubnis.
    private func ensurePermission() async -> Bool {
        let settings = await center.notificationSettings()
        switch settings.authorizationStatus {
        case .notDetermined:
            let granted = (try? await center.requestAuthorization(options: [.alert, .sound, .badge])) ?? false
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
        let status = await center.notificationSettings().authorizationStatus
        switch status {
        case .notDetermined: permission = .unknown
        case .denied: permission = .denied
        default: permission = .allowed
        }
    }

    private func apply(wanted: [PlannedReminder]) async {
        let scheduled = await center.pendingNotificationRequests().compactMap(Self.reminder(from:))
        let (remove, add) = ReminderPlanner.diff(scheduled: scheduled, wanted: wanted)
        if !remove.isEmpty {
            center.removePendingNotificationRequests(withIdentifiers: remove.map { Self.prefix + $0 })
        }
        for reminder in add {
            let interval = reminder.fireDate.timeIntervalSinceNow
            guard interval >= 1 else { continue }
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
            let request = UNNotificationRequest(identifier: Self.prefix + reminder.id, content: content, trigger: trigger)
            try? await center.add(request)
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
