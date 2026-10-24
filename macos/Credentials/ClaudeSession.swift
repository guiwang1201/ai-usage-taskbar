import Foundation
import Security
import LocalAuthentication
import CryptoKit

struct ClaudeSession: Codable {
    let key: String
    let organization: String?
    var identity: String { SHA256.hash(data: Data((key + "\n" + (organization ?? "")).utf8)).map { String(format: "%02x", $0) }.joined() }
    init(_ input: String, organization: String = "") throws {
        var value = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if value.hasPrefix("sessionKey=") { value.removeFirst(11) }
        value = value.trimmingCharacters(in: CharacterSet(charactersIn: ";").union(.whitespacesAndNewlines))
        let org = organization.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty, value.utf8.count <= 8192,
              value.utf8.allSatisfy({ (33...126).contains($0) && ![59, 44, 34, 92].contains($0) }),
              org.isEmpty || Self.validOrganization(org) else { throw CredentialFailure.malformed }
        key = value; self.organization = org.isEmpty ? nil : org
    }
    static func validOrganization(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= 128 && value.utf8.allSatisfy { (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || $0 == 45 }
    }
    static func discover(_ data: Data) throws -> String {
        guard let list = try JSONSerialization.jsonObject(with: data) as? [[String: Any]], list.count == 1,
              let id = list[0]["uuid"] as? String, validOrganization(id) else { throw CredentialFailure.malformed }
        return id
    }
    static let service = "com.iusage.claude-web"
    static func load() throws -> ClaudeSession {
        let value = try JSONDecoder().decode(Self.self, from: CredentialReader.systemKeychain(service: service, account: "session"))
        return try Self(value.key, organization: value.organization ?? "")
    }
    // Only the explicit configuration action writes IUsage's own Keychain item.
    static func save(_ value: ClaudeSession?) throws {
        let context = LAContext(); context.interactionNotAllowed = true
        let query: [CFString: Any] = [kSecClass: kSecClassGenericPassword, kSecAttrService: service,
                                    kSecAttrAccount: "session", kSecUseAuthenticationContext: context]
        let status: OSStatus
        if let value {
            let data = try JSONEncoder().encode(value)
            let update = SecItemUpdate(query as CFDictionary, [kSecValueData: data] as CFDictionary)
            if update == errSecItemNotFound {
                var item = query; item[kSecValueData] = data
                item[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
                status = SecItemAdd(item as CFDictionary, nil)
            } else { status = update }
        } else {
            let deleted = SecItemDelete(query as CFDictionary)
            status = deleted == errSecItemNotFound ? errSecSuccess : deleted
        }
        guard status == errSecSuccess else { throw CredentialFailure.unavailable(status) }
    }
}
