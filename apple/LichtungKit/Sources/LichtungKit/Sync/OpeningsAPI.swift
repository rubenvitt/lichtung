import Foundation

// Buchungslinks verwalten (docs/api/openings.md §5, LIC-35). Wie die übrigen
// Verwaltungsaufrufe direkt, ohne Outbox. Der neue Stand kommt zusätzlich
// über den Pull (Objekttyp `opening`).

extension ManagementAPI {
    /// Alle Links, neueste zuerst.
    public func openings() async throws -> [Opening] {
        try await list("api/openings")
    }

    /// Legt einen Link an (`201`). Mit Vorschlägen legt der Server dabei die
    /// Platzhalter an; schlägt das fehl, gibt es keinen Link
    /// (`502 provider_unavailable` bzw. `400` mit dem Code des Anbieters).
    /// Zielkalender ungeeignet: `400 calendar_not_found`,
    /// `calendar_read_only` oder `calendar_cannot_invite`.
    public func createOpening(_ opening: NewOpening) async throws -> Opening {
        try await send("POST", "api/openings", body: opening)
    }

    public func updateOpening(_ id: String, _ patch: OpeningPatch) async throws -> Opening {
        try await send("PATCH", "api/openings/\(id)", body: patch)
    }

    /// Löscht den Link und entfernt seine Platzhalter (`204`).
    public func deleteOpening(_ id: String) async throws {
        try await sendEmpty("DELETE", "api/openings/\(id)")
    }
}
