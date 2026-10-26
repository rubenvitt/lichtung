// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import Foundation
import LichtungKit
import Security

/// Geheimnisse in der Keychain, als generisches Passwort (LIC-43).
///
/// Bewusst ohne Keychain-Sharing und ohne eigene Access Group: Jedes Gerät
/// meldet sich selbst an und hat sein eigenes Token (Vertrag §2). Auf dem Mac
/// landet der Eintrag damit in der klassischen Keychain, die auch in der
/// Sandbox ohne weiteres Entitlement funktioniert; die Data-Protection-
/// Keychain bräuchte eine signierte Application-ID.
struct KeychainSecretStore: SecretStore {
    var service = "dev.rubeen.lichtung"

    private func query(_ key: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
        ]
    }

    func read(_ key: String) throws -> String? {
        var q = query(key)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw KeychainError(status: status) }
        guard let data = item as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    func write(_ value: String, for key: String) throws {
        let data = Data(value.utf8)
        let update: [String: Any] = [kSecValueData as String: data]
        let status = SecItemUpdate(query(key) as CFDictionary, update as CFDictionary)
        if status == errSecSuccess { return }
        guard status == errSecItemNotFound else { throw KeychainError(status: status) }
        var add = query(key)
        add[kSecValueData as String] = data
        #if os(iOS)
        // Nach dem ersten Entsperren lesbar: Die stille Push-Nachricht (Spec §5)
        // soll auch bei gesperrtem Gerät abgleichen können. Nicht ins Backup.
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        #endif
        let added = SecItemAdd(add as CFDictionary, nil)
        guard added == errSecSuccess else { throw KeychainError(status: added) }
    }

    func delete(_ key: String) throws {
        let status = SecItemDelete(query(key) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw KeychainError(status: status) }
    }
}

struct KeychainError: Error, LocalizedError {
    let status: OSStatus

    var errorDescription: String? {
        "Schlüsselbund-Fehler \(status)"
    }
}
