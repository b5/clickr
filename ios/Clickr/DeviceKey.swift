import ClickrKit
import Foundation
import Security

/// This install's iroh secret key, kept in the Keychain.
///
/// Receivers trust phones by endpoint id, which is derived from this key, so it must stay
/// the same across launches and reconnects. The item is only readable on this device and
/// is not restored onto other devices from backups.
enum DeviceKey {
    private static let service = "computer.n0.clickr"
    private static let account = "secretKey"

    /// Returns the stored key, creating and storing one on first use.
    static func load() -> Data {
        if let existing = read(), existing.count == 32 {
            return existing
        }
        let key = generateSecretKey()
        store(key)
        return key
    }

    private static func read() -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: AnyObject?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess else {
            return nil
        }
        return result as? Data
    }

    private static func store(_ key: Data) {
        let match: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(match as CFDictionary)
        var item = match
        item[kSecValueData as String] = key
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(item as CFDictionary, nil)
        if status != errSecSuccess {
            NSLog("clickr: failed to store secret key in Keychain: \(status)")
        }
    }
}
