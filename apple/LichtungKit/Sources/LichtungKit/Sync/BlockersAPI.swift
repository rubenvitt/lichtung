import Foundation

// Blocker-Regeln verwalten (docs/api/blockers.md, LIC-75). Direkt, ohne
// Outbox; die Blocker selbst kommen als gewöhnliche Termine über den Pull.

extension ManagementAPI {
    public func blockerRules() async throws -> [BlockerRule] {
        try await list("api/blocker-rules")
    }

    /// Legt eine Regel an (`201`). Ziel nur lesbar: `400 calendar_read_only`;
    /// dieselben Kalender schon verbunden: `409`.
    public func createBlockerRule(_ rule: NewBlockerRule) async throws -> BlockerRule {
        try await send("POST", "api/blocker-rules", body: rule)
    }

    public func updateBlockerRule(_ id: String, _ patch: BlockerRulePatch) async throws -> BlockerRule {
        try await send("PATCH", "api/blocker-rules/\(id)", body: patch)
    }

    /// Löscht die Regel; der Server entfernt danach ihre Blocker (`204`).
    public func deleteBlockerRule(_ id: String) async throws {
        try await sendEmpty("DELETE", "api/blocker-rules/\(id)")
    }
}
