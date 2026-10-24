import Foundation

// Explicit one-shot validation. No recurring timer, no token output.
let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("IUsage")
let lock = try InstanceLock(url: root.appendingPathComponent("instance.lock"))
let store = OfflineStore(url: root.appendingPathComponent("cache.json"))
let runtime = OfflineRuntime(state: try store.load(), save: store.save)
let sources = LocalSources(home: FileManager.default.homeDirectoryForCurrentUser, environment: ProcessInfo.processInfo.environment, reader: CredentialReader())
var remaining = 0
var transports: [UsageHTTP] = []
let coordinator = FetchCoordinator(runtime: runtime, now: { Int64(Date().timeIntervalSince1970) }) { url, headers, callback in
    let transport = UsageHTTP(configuration: .ephemeral)
    transports.append(transport)
    transport.get(url: url, headers: headers, completion: callback)
}
for provider in 0...1 {
    let name = provider == 0 ? "Claude" : "Codex"
    let now = Int64(Date().timeIntervalSince1970)
    guard runtime.nextEvent(provider: provider, now: now, idle: false) <= now else {
        print("\(name): cached or cooling down; no request")
        continue
    }
    var counted = false
    do {
        remaining += 1
        counted = true
        let completion: (Error?) -> Void = { error in
            if error != nil { print("\(name): persistence failed; no retry") }
            else if let status = runtime.errors[provider] { print("\(name): \(status); retry state saved") }
            else { print("\(name): request completed; discovery may precede usage") }
            remaining -= 1
        }
        let started: Bool
        if provider == 0 { started = try coordinator.fetchClaude(session: ClaudeSession.load(), completion: completion) }
        else {
            let credential = try sources.codex()
            started = try coordinator.fetch(provider: 1, token: credential.accessToken, account: credential.accountID, completion: completion)
        }
        if !started { remaining -= 1; print("\(name): request gate refused") }
    } catch {
        // Local failures never print headers, JSON, token or opaque error payloads.
        if counted { remaining -= 1 }
        print("\(name): local preparation failed; no retry")
    }
}
while remaining > 0 { RunLoop.main.run(until: Date().addingTimeInterval(0.05)) }
withExtendedLifetime(lock) {}
print("One-shot validation finished; recurring polling disabled")
