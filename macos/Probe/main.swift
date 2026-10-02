import Foundation

// Explicit offline source probe. Never prints credential contents or identifiers.
let home = FileManager.default.homeDirectoryForCurrentUser
let environment = ProcessInfo.processInfo.environment
let reader = CredentialReader()
func report(_ name: String, _ action: () throws -> OAuthCredential) {
    do {
        _ = try action()
        print("\(name): readable OAuth structure (server validity NOT checked)")
    } catch let error as CredentialFailure {
        print("\(name): \(error)")
    } catch {
        print("\(name): local read failed")
    }
}
let sources = LocalSources(home: home, environment: environment, reader: reader)
report("Claude") { try sources.claude() }
report("Codex selected source") { try sources.codex() }
print("Claude bundled version discoverable: \(sources.claudeVersion() != nil)")
