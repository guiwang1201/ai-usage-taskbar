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
        let credential = try provider == 0 ? sources.claude() : sources.codex()
        remaining += 1
        counted = true
        let started = try coordinator.fetch(provider: provider, token: credential.accessToken, account: credential.accountID, claudeVersion: sources.claudeVersion()) { error in
            if error != nil { print("\(name): persistence failed; no retry") }
            else if let status = runtime.errors[provider] { print("\(name): \(status); retry state saved") }
            else { print("\(name): response parsed and cached") }
            remaining -= 1
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
