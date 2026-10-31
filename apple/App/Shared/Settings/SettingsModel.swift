// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import Foundation
import LichtungKit
import Observation

/// Zustand der Einstellungen (LIC-43). Alle Änderungen gehen direkt über
/// `ManagementAPI` (Vertrag §7, nur online); Konten, Kalender und Sets
/// kommen danach über den Pull in den Stand.
@MainActor
@Observable
final class SettingsModel {
    @ObservationIgnored private weak var session: AppSession?

    /// `GET /api/me` teilt sich die Sitzung mit dem Abgleich des
    /// Gerätenamens (LIC-56).
    var me: ServerInfo? { session?.me }
    private(set) var devices: [Device] = []
    private(set) var tokens: [APIToken] = []
    /// Neu erzeugtes API-Token: Das Geheimnis ist nur jetzt sichtbar.
    var createdToken: APIToken?
    /// Letzter Fehler als Satz; die Ansichten zeigen ihn oben an.
    var errorMessage: String?
    private(set) var isWorking = false
    /// Kalender-Einstellungen, wie sie nach der Antwort aussehen sollen, bis
    /// der Pull sie nachgezogen hat (sonst springt ein Schalter kurz zurück).
    private var calendarOverrides: [String: CalendarInfo] = [:]

    func attach(_ session: AppSession) {
        self.session = session
    }

    /// Nach Anmelden, Abmelden, Demo: nichts vom alten Server stehen lassen.
    func reset() {
        devices = []
        tokens = []
        createdToken = nil
        errorMessage = nil
        calendarOverrides = [:]
        deletedOpeningIds = []
    }

    var isAvailable: Bool { session?.management != nil }

    private var state: SyncState { session?.model.state ?? SyncState() }

    // MARK: Lesen

    var accounts: [Account] {
        state.accounts.values.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    func account(_ id: String) -> Account? { state.accounts[id] }

    private var mergedCalendars: [String: CalendarInfo] {
        state.calendars.merging(calendarOverrides) { _, override in override }
    }

    func calendar(_ id: String) -> CalendarInfo? { mergedCalendars[id] }

    var calendarSections: [CalendarOrder.Section] {
        CalendarOrder.sections(calendars: mergedCalendars, accounts: state.accounts)
    }

    var calendarSets: [CalendarSet] { CalendarOrder.sortedSets(state.calendarSets.values) }

    // MARK: Ablauf

    /// Führt einen Verwaltungsaufruf aus; Fehler landen in `errorMessage`.
    @discardableResult
    private func run(_ work: @MainActor (ManagementAPI) async throws -> Void) async -> Bool {
        guard let api = session?.management else {
            errorMessage = "Ohne Server lässt sich das nicht ändern."
            return false
        }
        isWorking = true
        defer { isWorking = false }
        do {
            try await work(api)
            errorMessage = nil
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }

    /// Pull nachziehen, damit die Änderung im Stand steht.
    private func refresh() async {
        await session?.model.refresh()
    }

    /// Nur `me` (Allgemein: Konto, Gerät, Server-Version). Eine frische
    /// Antwort der Sitzung (etwa vom Aktivwerden) gilt ohne neue Anfrage.
    func loadMe() async {
        guard let session else { return }
        do {
            _ = try await session.loadMe(maxAge: AppSession.meMaxAge)
        } catch is CancellationError {
            // Server gewechselt oder abgemeldet: nichts zu melden.
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// `force` (Ziehen zum Aktualisieren) fragt auch `/api/me` neu.
    func loadOverview(force: Bool = false) async {
        guard let session, let api = session.management else { return }
        let serverURL = session.serverURL
        do {
            do {
                _ = try await session.loadMe(maxAge: force ? 0 : AppSession.meMaxAge)
            } catch is CancellationError {
                // Verworfen, weil dieses Gerät gerade umbenannt wurde: Geräte und
                // Tokens trotzdem laden. Bei anderem Server oder Abmelden nicht.
                guard session.serverURL == serverURL, session.management != nil else { return }
            }
            devices = try await api.devices()
            tokens = try await api.tokens()
        } catch is CancellationError {
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: Konten

    func addAccount(_ account: NewAccount) async -> Bool {
        let ok = await run { _ = try await $0.addAccount(account) }
        if ok { await refresh() }
        return ok
    }

    func renameAccount(_ id: String, to name: String) async -> Bool {
        let n = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !n.isEmpty else {
            errorMessage = "Der Name darf nicht leer sein."
            return false
        }
        let ok = await run { _ = try await $0.renameAccount(id, name: n) }
        if ok { await refresh() }
        return ok
    }

    func updateCredentials(_ id: String, username: String, password: String) async -> Bool {
        let ok = await run { _ = try await $0.updateCredentials(id, username: username.trimmingCharacters(in: .whitespaces), password: password) }
        if ok { await refresh() }
        return ok
    }

    func removeAccount(_ id: String) async {
        if await run({ try await $0.removeAccount(id) }) { await refresh() }
    }

    func syncAccountNow(_ id: String) async {
        if await run({ try await $0.syncAccountNow(id) }) { await refresh() }
    }

    // MARK: Kalender

    func updateCalendar(_ id: String, _ change: CalendarSettings) async {
        guard let current = calendar(id) else { return }
        calendarOverrides[id] = change.applied(to: current)
        let ok = await run { api in
            let updated = try await api.updateCalendar(id, change)
            self.calendarOverrides[id] = updated
        }
        if ok { await refresh() }
        await dropOverrides([id])
    }

    /// Verschieben innerhalb eines Kontos (`onMove`).
    func moveCalendars(section: [String], from: IndexSet, to: Int) async {
        let all = Array(mergedCalendars.values)
        let changes = CalendarOrder.positionsAfterMove(section: section, fromOffsets: from, toOffset: to, all: all)
        guard !changes.isEmpty else { return }
        for (id, position) in changes {
            if let c = calendar(id) { calendarOverrides[id] = CalendarSettings(position: position).applied(to: c) }
        }
        let ok = await run { api in
            for (id, position) in changes.sorted(by: { $0.value < $1.value }) {
                _ = try await api.updateCalendar(id, CalendarSettings(position: position))
            }
        }
        if ok { await refresh() }
        await dropOverrides(Array(changes.keys))
    }

    /// Der neue Stand erreicht das `AppModel` über einen Strom, also kurz
    /// nach dem Ende des Pulls. Erst dann die vorläufigen Werte fallen lassen.
    private func dropOverrides(_ ids: [String]) async {
        try? await Task.sleep(nanoseconds: 400_000_000)
        for id in ids { calendarOverrides[id] = nil }
    }

    // MARK: Kalender-Sets

    func saveSet(id: String?, name: String, calendarIds: [String]) async -> Bool {
        let n = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !n.isEmpty else {
            errorMessage = "Das Set braucht einen Namen."
            return false
        }
        let position = CalendarOrder.nextSetPosition(state.calendarSets.values)
        let ok = await run { api in
            if let id {
                _ = try await api.updateCalendarSet(id, name: n, calendarIds: calendarIds)
            } else {
                _ = try await api.createCalendarSet(name: n, calendarIds: calendarIds, position: position)
            }
        }
        if ok { await refresh() }
        return ok
    }

    func deleteSet(_ id: String) async {
        if await run({ try await $0.deleteCalendarSet(id) }) { await refresh() }
    }

    func moveSets(from: IndexSet, to: Int) async {
        let changes = CalendarOrder.setPositionsAfterMove(Array(state.calendarSets.values), fromOffsets: from, toOffset: to)
        guard !changes.isEmpty else { return }
        let ok = await run { api in
            for (id, position) in changes.sorted(by: { $0.value < $1.value }) {
                _ = try await api.updateCalendarSet(id, position: position)
            }
        }
        if ok { await refresh() }
    }

    // MARK: Geräte und Tokens

    func removeDevice(_ id: String) async {
        if await run({ try await $0.removeDevice(id) }) {
            devices.removeAll { $0.id == id }
        }
    }

    /// Umbenennen (LIC-46). Beim eigenen Gerät merkt sich die Sitzung den
    /// Namen für die nächste Anmeldung.
    @discardableResult
    func renameDevice(_ id: String, to name: String) async -> Bool {
        let n = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !n.isEmpty else {
            errorMessage = "Der Gerätename darf nicht leer sein."
            return false
        }
        return await run { api in
            let updated = try await api.renameDevice(id, name: n)
            if let i = self.devices.firstIndex(where: { $0.id == id }) { self.devices[i] = updated }
            if updated.current == true || self.me?.device?.id == id {
                self.session?.currentDeviceRenamed(updated.name)
            }
        }
    }

    func createToken(name: String) async -> Bool {
        let n = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !n.isEmpty else {
            errorMessage = "Bitte dem Token einen Namen geben, z. B. „Claude Code“."
            return false
        }
        return await run { api in
            let token = try await api.createToken(name: n)
            self.createdToken = token
            var listed = token
            listed.token = nil
            self.tokens.append(listed)
        }
    }

    func revokeToken(_ id: String) async {
        if await run({ try await $0.revokeToken(id) }) {
            tokens.removeAll { $0.id == id }
        }
    }

    // MARK: Buchungslinks (LIC-35)

    /// Gelöschte Links, bis der Pull sie auch entfernt hat.
    private var deletedOpeningIds: Set<String> = []

    var openingList: OpeningList {
        OpeningList(state.openings.values.filter { !deletedOpeningIds.contains($0.id) }, now: Date())
    }

    func opening(_ id: String) -> Opening? { state.openings[id] }

    /// Kalender des Stands (mit vorläufigen Einstellungen) für Texte und Zielauswahl.
    var calendarsById: [String: CalendarInfo] { mergedCalendars }

    var openingTargets: [CalendarInfo] { OpeningTargets.allowed(mergedCalendars) }

    /// Legt einen Link an; das Ergebnis trägt die URL zum Kopieren.
    func createOpening(_ body: NewOpening) async -> Opening? {
        var created: Opening?
        let ok = await run { created = try await $0.createOpening(body) }
        if ok { await refresh() }
        return created
    }

    func updateOpening(_ id: String, _ patch: OpeningPatch) async -> Bool {
        guard !patch.isEmpty else { return true }
        let ok = await run { _ = try await $0.updateOpening(id, patch) }
        if ok { await refresh() }
        return ok
    }

    /// Löscht den Link; der Server entfernt seine Platzhalter.
    @discardableResult
    func deleteOpening(_ id: String) async -> Bool {
        let ok = await run { try await $0.deleteOpening(id) }
        if ok {
            deletedOpeningIds.insert(id)
            await refresh()
        }
        return ok
    }
}
