import Foundation
import Security
import LocalAuthentication

enum ClaudeStoreFailure: Error, Equatable {
    case cannotWrite
    // Someone else changed the credentials since they were read.
    case changed
}

// Writes renewed Claude tokens back to where they were read from, and only there.
// The server retires the old refresh token when it issues a new one, so a lost write
// would sign Claude Code out: the store is proved writable before any request is sent.
struct ClaudeStore {
    let reader: CredentialReader
    let writeKeychain: (String, String, Data) throws -> Void
    let writeFile: (URL, Data) throws -> Void
    let fileReplaceable: (URL) -> Bool

    init(reader: CredentialReader = CredentialReader(),
         writeKeychain: @escaping (String, String, Data) throws -> Void = ClaudeStore.systemKeychainWrite,
         writeFile: @escaping (URL, Data) throws -> Void = { try DurableFile.replace(url: $0, data: $1) },
         fileReplaceable: @escaping (URL) -> Bool = ClaudeStore.systemFileReplaceable) {
        self.reader = reader; self.writeKeychain = writeKeychain
        self.writeFile = writeFile; self.fileReplaceable = fileReplaceable
    }

    func read(_ source: ClaudeSource) throws -> Data { try reader.read(source) }

    // A Keychain item cannot be probed without writing it; the stored bytes are
    // written back unchanged, after checking they are still what was read.
    func prove(_ record: ClaudeCredentialRecord) throws {
        guard (try? reader.read(record.source)) == record.data else { throw ClaudeStoreFailure.changed }
        switch record.source {
        case .keychain(let service, let account):
            do { try writeKeychain(service, account, record.data) }
            catch { throw ClaudeStoreFailure.cannotWrite }
        case .file(let url):
            guard fileReplaceable(url) else { throw ClaudeStoreFailure.cannotWrite }
        }
    }

    // A few tries, as the Windows file store does: a brief lock is not the end of a login.
    func store(_ data: Data, to source: ClaudeSource) throws {
        var failure: Error = ClaudeStoreFailure.cannotWrite
        for attempt in 0..<3 {
            if attempt > 0 { Thread.sleep(forTimeInterval: 0.1) }
            do {
                switch source {
                case .keychain(let service, let account): try writeKeychain(service, account, data)
                case .file(let url): try writeFile(url, data)
                }
                return
            } catch { failure = error }
        }
        throw failure
    }

    // Never prompts: a write that would need approval fails and the renewal is not tried.
    static func systemKeychainWrite(service: String, account: String, data: Data) throws {
        let context = LAContext()
        context.interactionNotAllowed = true
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
            kSecUseAuthenticationContext: context
        ]
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData: data] as CFDictionary)
        guard status == errSecSuccess else { throw CredentialFailure.unavailable(status) }
    }

    // A regular file whose folder takes a new file, since the write replaces it.
    static func systemFileReplaceable(_ url: URL) -> Bool {
        let manager = FileManager.default
        guard let attributes = try? manager.attributesOfItem(atPath: url.path),
              attributes[.type] as? FileAttributeType == .typeRegular,
              manager.isWritableFile(atPath: url.path) else { return false }
        let probe = url.deletingLastPathComponent().appendingPathComponent(".iusage-probe-\(UUID().uuidString)")
        guard manager.createFile(atPath: probe.path, contents: nil, attributes: [.posixPermissions: 0o600]) else { return false }
        try? manager.removeItem(at: probe)
        return true
    }
}
