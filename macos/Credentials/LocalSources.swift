import Foundation

struct LocalSources {
    let home: URL
    let environment: [String: String]
    let reader: CredentialReader
    func claude(now: Date = Date()) throws -> OAuthCredential {
        try reader.claude(home: home, account: NSUserName(), configDirectory: environment["CLAUDE_CONFIG_DIR"],
                          secureStorageDirectory: environment["CLAUDE_SECURESTORAGE_CONFIG_DIR"], now: now)
    }
    var codexDirectory: URL {
        environment["CODEX_HOME"].flatMap { $0.isEmpty ? nil : URL(fileURLWithPath: $0) }
            ?? home.appendingPathComponent(".codex")
    }
    func codex() throws -> OAuthCredential {
        let text: String
        do { text = try String(contentsOf: codexDirectory.appendingPathComponent("config.toml"), encoding: .utf8) }
        catch let error as NSError where error.domain == NSCocoaErrorDomain && error.code == NSFileReadNoSuchFileError { text = "" }
        let mode = try CodexStorageMode.parseConfiguration(text)
        // Reject unhandled backend/policy settings until full configuration parsing
        // is available, rather than guessing which cached login is active.
        if text.contains("cli_auth_keyring_backend") || text.contains("forced_login_method") || text.contains("forced_chatgpt_workspace_id") {
            throw CredentialFailure.unsupportedLogin
        }
        return try reader.codex(directory: codexDirectory, mode: mode)
    }
    func claudeVersion() -> String? {
        let directory = home.appendingPathComponent("Library/Application Support/Claude/claude-code")
        guard let names = try? FileManager.default.contentsOfDirectory(atPath: directory.path) else { return nil }
        return names.compactMap { name -> ([Int], String)? in
            let parts = name.split(separator: ".").compactMap { Int($0) }
            guard parts.count == 3, name.split(separator: ".").count == 3 else { return nil }
            return (parts, name)
        }.max { $0.0.lexicographicallyPrecedes($1.0) }?.1
    }
}
