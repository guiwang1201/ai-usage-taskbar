import Foundation
var saved = OfflineState()
var clock: Int64 = 1000
var sends = 0
var pending: ((Result<UsageHTTP.Response, Error>) -> Void)?
let runtime = OfflineRuntime(state: saved, save: { saved = $0 })
let coordinator = FetchCoordinator(runtime: runtime, now: { clock }) { url, headers, callback in
    precondition(saved.gates[1].lastAttempt == clock) // persisted before transport
    precondition(url.host == "chatgpt.com" && headers["Authorization"] == "Bearer mock")
    sends += 1; pending = callback
}
var finished = false
precondition(try! coordinator.fetch(provider: 1, token: "mock", account: nil) { _ in finished = true })
clock = 1200 // even after the gap, the outstanding request remains unique
precondition(try! !coordinator.fetch(provider: 1, token: "mock", account: nil) { _ in })
precondition(sends == 1)
pending?(.success(UsageHTTP.Response(status: 429, body: Data(), retryAfter: 600)))
let deadline = Date().addingTimeInterval(2)
while !finished && Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
precondition(finished && saved.gates[1].blockedUntil == 1800)
precondition(try! !coordinator.fetch(provider: 1, token: "mock", account: nil) { _ in })
print("PASS: async coordinator persists reservation, prohibits overlap, and retains 429 cooldown; mock only")
