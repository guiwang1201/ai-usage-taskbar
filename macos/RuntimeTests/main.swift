import Foundation
let root = FileManager.default.temporaryDirectory.appendingPathComponent("iusage-runtime-\(UUID().uuidString)")
defer { try? FileManager.default.removeItem(at: root) }
let store = OfflineStore(url: root.appendingPathComponent("cache.json"))
let runtime = OfflineRuntime(state: try store.load(), save: store.save)
let body = #"{"five_hour":{"utilization":32},"seven_day":{"utilization":64}}"#
var sends = 0
precondition(try! runtime.simulate(provider: 0, now: 1000, status: 200, body: body, onSend: { sends += 1 }))
precondition(runtime.state.readings[0]?.five == 32)
precondition(runtime.nextEvent(provider: 0, now: 1001, idle: false) == 1180)
precondition(runtime.nextEvent(provider: 0, now: 1001, idle: true) == 1900)
let restarted = OfflineRuntime(state: try store.load(), save: store.save)
precondition(try! !restarted.simulate(provider: 0, now: 1119, status: 200, body: body, onSend: { sends += 1 }))
precondition(sends == 1)
precondition(try! restarted.simulate(provider: 0, now: 1180, status: 429, retryAfter: 600, body: ""))
let cooled = OfflineRuntime(state: try store.load(), save: store.save)
precondition(cooled.nextEvent(provider: 0, now: 1300, idle: false) == 1780)
precondition(cooled.state.readings[0]?.five == 32)
precondition(try! !cooled.simulate(provider: 0, now: 1779, status: 200, body: body))
precondition(try! cooled.simulate(provider: 0, now: 1780, status: 200, body: "{}"))
precondition(cooled.state.readings[0]?.fetchedAt == 1000)
try cooled.select(1)
precondition(try! store.load().selectedProvider == 1)
let failing = OfflineRuntime(state: OfflineState(), save: { _ in throw CocoaError(.fileWriteNoPermission) })
do { _ = try failing.simulate(provider: 0, now: 1000, status: 200, body: body, onSend: { fatalError("Must not send without reservation") }); fatalError("Expected failure") }
catch {}
let partial = OfflineRuntime(state: OfflineState(), save: { _ in })
precondition(try! partial.simulate(provider: 0, now: 1000, status: 200, body: #"{"five_hour":null,"seven_day":{"utilization":0}}"#))
precondition(partial.state.readings[0]?.five == nil && partial.state.readings[0]?.seven == 0)
precondition(try! partial.simulate(provider: 0, now: 1180, status: 0, body: ""))
precondition(partial.errors[0] == "离线" && partial.state.readings[0]?.seven == 0)
var corrupt = OfflineState()
corrupt.gates[0].blockedUntil = Int64.max
try store.save(corrupt)
do { _ = try store.load(); fatalError("Must reject corrupt timestamps") }
catch {}
var lock: InstanceLock? = try InstanceLock(url: root.appendingPathComponent("instance.lock"))
precondition(lock != nil)
do { _ = try InstanceLock(url: root.appendingPathComponent("instance.lock")); fatalError("Expected lock contention") }
catch {}
lock = nil
let newLock = try InstanceLock(url: root.appendingPathComponent("instance.lock"))
withExtendedLifetime(newLock) {}
print("PASS: runtime cache/restart, cooldown, mock failure, write failure, selection and single-instance lock")

let durableURL = root.appendingPathComponent("durable.json")
try DurableFile.replace(url: durableURL, data: Data("old".utf8))
do {
    try DurableFile.replace(url: durableURL, data: Data("new".utf8), beforeRename: { throw CocoaError(.fileWriteUnknown) })
    fatalError("Expected injected failure")
} catch {}
precondition(try! String(contentsOf: durableURL, encoding: .utf8) == "old")
precondition(try! FileManager.default.contentsOfDirectory(atPath: root.path).filter { $0.hasSuffix(".tmp") }.isEmpty)
try DurableFile.replace(url: durableURL, data: Data("new".utf8))
precondition(try! String(contentsOf: durableURL, encoding: .utf8) == "new")
print("PASS: durable replacement preserves old file on failure and cleans temporary files")

let resets = OfflineRuntime(state: OfflineState(), save: { _ in })
precondition(try! resets.simulate(provider: 1, now: 1000, status: 200, body: #"{"rate_limit":{"primary_window":{"used_percent":42,"reset_at":1100},"secondary_window":{"used_percent":61,"reset_at":2000}}}"#))
precondition(resets.state.readings[1]?.fiveReset == 1100)
precondition(resets.state.readings[1]?.values(now: 1100)[0] == 0)
precondition(resets.state.readings[1]?.values(now: 1100)[1] == 61)
try store.save(resets.state)
precondition(try! store.load().readings[1]?.sevenReset == 2000)
print("PASS: reset timestamps persist and elapsed windows display zero without requests")

try runtime.setRemaining(true)
precondition(try! store.load().showRemaining == true)
precondition(DisplayFormat.countdown(2 * 86400 + 5) == "2d")
precondition(DisplayFormat.countdown(4 * 3600 + 59 * 60) == "4h")
precondition(DisplayFormat.longCountdown(6 * 86400 + 13 * 3600) == "6 天 13 小时")
print("PASS: display mode persists and countdown matches Windows units")

var marks = [AlertMark(), AlertMark()]
let high = SavedReading(five: 80, seven: 10, fetchedAt: 1000, fiveReset: 2000, sevenReset: 3000)
precondition(AlertPolicy.evaluate(reading: high, marks: &marks, now: 1000).count == 1)
precondition(AlertPolicy.evaluate(reading: high, marks: &marks, now: 1001).isEmpty)
let newWindow = SavedReading(five: 95, seven: 10, fetchedAt: 1000, fiveReset: 4000, sevenReset: 5000)
precondition(AlertPolicy.evaluate(reading: newWindow, marks: &marks, now: 1000).count == 1)
print("PASS: alerts deduplicate and rearm on a new window")

let issues = OfflineRuntime(state: OfflineState(), save: { _ in })
precondition(issues.issue(0) == nil && issues.issue(2) == nil)
issues.setLocalIssue(0, "已过期")
precondition(issues.issue(0) == "已过期" && issues.issue(1) == nil)
precondition(try! issues.simulate(provider: 0, now: 1000, status: 429, body: ""))
precondition(issues.errors[0] == "限流" && issues.localIssues[0] == nil && issues.issue(0) == "限流")
issues.setLocalIssue(0, "未登录")
precondition(issues.issue(0) == "未登录") // the newer local finding wins over a fetch error
issues.setLocalIssue(5, "ignored")
precondition(issues.issue(5) == nil)
var alertState = OfflineState()
alertState.notifications = true
alertState.readings[0] = SavedReading(five: 95, seven: 10, fetchedAt: 1000, fiveReset: 5000, sevenReset: 6000)
let quiet = OfflineRuntime(state: alertState, save: { _ in })
quiet.setLocalIssue(0, "已过期")
precondition(try! quiet.consumeAlert(provider: 0, now: 1000).isEmpty)
quiet.setLocalIssue(0, nil)
precondition(try! !quiet.consumeAlert(provider: 0, now: 1000).isEmpty)
print("PASS: local credential issues outrank fetch errors, clear on a completed request, and silence alerts")
