#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import Foundation
import Security
import NexusCore

enum GeminiKeychain {
    enum Failure: Error { case invalid, unavailable(OSStatus) }
    private static var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "dev.nexuside.ubuntu.lab.gemini",
         kSecAttrAccount as String: "local-api-key",
         kSecAttrSynchronizable as String: false]
    }

    static func save(_ key: String) throws {
        guard GeminiCredential.isValid(key) else { throw Failure.invalid }
        let bytes = Data(key.utf8)
        let changes: [String: Any] = [kSecValueData as String: bytes,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly]
        var status = SecItemUpdate(query as CFDictionary, changes as CFDictionary)
        if status == errSecItemNotFound {
            let item = query.merging(changes) { _, new in new }
            status = SecItemAdd(item as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw Failure.unavailable(status) }
    }

    static func load() throws -> String? {
        var request = query
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw Failure.unavailable(status) }
        guard let bytes = result as? Data, let key = String(data: bytes, encoding: .utf8),
              GeminiCredential.isValid(key) else { throw Failure.invalid }
        return key
    }

    static func remove() throws {
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw Failure.unavailable(status) }
    }
}
#endif
