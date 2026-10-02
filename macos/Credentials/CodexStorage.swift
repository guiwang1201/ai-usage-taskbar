import Foundation
import CryptoKit

// Storage selection is explicit. Keyring names are not guessed; unsupported
// modes never silently select an old auth.json belonging to a different login.
enum CodexKeyringBackend { case direct, secrets }

enum CodexStorageMode: String {
    case file, keyring, auto, ephemeral
    static func parseConfiguration(_ text: String) throws -> CodexStorageMode {
        var selected: CodexStorageMode?
        for raw in text.components(separatedBy: .newlines) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("[") { break } // This key is a top-level setting.
            guard line.hasPrefix("cli_auth_credentials_store") else { continue }
            let pieces = line.split(separator: "=", maxSplits: 1).map(String.init)
            guard pieces.count == 2, pieces[0].trimmingCharacters(in: .whitespaces) == "cli_auth_credentials_store",
                  selected == nil else { throw CredentialFailure.malformed }
            let value = pieces[1].components(separatedBy: "#")[0].trimmingCharacters(in: .whitespaces)
            guard value.count >= 2, let quote = value.first, quote == "\"" || quote == "'", value.last == quote,
                  let mode = CodexStorageMode(rawValue: String(value.dropFirst().dropLast())) else {
                throw CredentialFailure.malformed
            }
            selected = mode
        }
        return selected ?? .file
    }
}
extension CredentialReader {
    static func codexKey(directory: URL) -> String {
        let canonical = directory.resolvingSymlinksInPath().standardizedFileURL.path
        let hash = SHA256.hash(data: Data(canonical.utf8)).map { String(format: "%02x", $0) }.joined()
        return "cli|" + hash.prefix(16)
    }
    // Supports the official direct Keychain backend; encrypted secrets backend
    // must be selected explicitly and is not silently interpreted as direct.
    func codex(directory: URL, mode: CodexStorageMode, backend: CodexKeyringBackend = .direct) throws -> OAuthCredential {
        guard mode == .file || mode == .ephemeral || backend == .direct else { throw CredentialFailure.unsupportedLogin }
        switch mode {
        case .file: return try codexFile(directory: directory)
        case .ephemeral: throw CredentialFailure.unsupportedLogin
        case .keyring:
            return try Self.parseCodex(readKeychain("Codex Auth", Self.codexKey(directory: directory)))
        case .auto:
            let data: Data
            do { data = try readKeychain("Codex Auth", Self.codexKey(directory: directory)) }
            catch { return try codexFile(directory: directory) }
            // A malformed stored item is not silently substituted with an old file.
            return try Self.parseCodex(data)
        }
    }
}
