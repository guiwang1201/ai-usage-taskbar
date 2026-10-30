import Foundation
import Darwin

struct SavedGate: Codable {
    var lastAttempt: Int64 = 0
    var nextPoll: Int64 = 0
    var blockedUntil: Int64 = 0
    var backoff: Int64 = 0
    var native: MacGate { MacGate(last_attempt: lastAttempt, next_poll: nextPoll, blocked_until: blockedUntil, backoff: backoff) }
    init() {}
    init(_ gate: MacGate) {
        lastAttempt = gate.last_attempt; nextPoll = gate.next_poll
        blockedUntil = gate.blocked_until; backoff = gate.backoff
    }
}
struct SavedReading: Codable {
    let five: Double?
    let seven: Double?
    let fetchedAt: Int64
    var fiveReset: Int64? = nil
    var sevenReset: Int64? = nil
    func values(now: Int64) -> [Double?] {
        [fiveReset.map { $0 <= now } == true ? five.map { _ in 0 } : five,
         sevenReset.map { $0 <= now } == true ? seven.map { _ in 0 } : seven]
    }
}
struct OfflineState: Codable {
    var selectedProvider = 0
    var claudeSessionIdentity: String? = nil
    var claudeOrganization: String? = nil
    var claudeSessionExpired: Bool? = nil
    var notifications: Bool? = nil
    var alertMarks: [[AlertMark]]? = nil
    var showRemaining: Bool? = nil
    var gates = [SavedGate(), SavedGate()]
    var readings: [SavedReading?] = [nil, nil]
}
final class OfflineStore {
    let url: URL
    init(url: URL) { self.url = url }
    func load() throws -> OfflineState {
        guard FileManager.default.fileExists(atPath: url.path) else { return OfflineState() }
        let state = try JSONDecoder().decode(OfflineState.self, from: Data(contentsOf: url))
        guard state.gates.count == 2, state.readings.count == 2, (0...1).contains(state.selectedProvider),
              state.gates.allSatisfy({ gate in
                  [gate.lastAttempt, gate.nextPoll, gate.blockedUntil].allSatisfy { (0...253_402_300_799).contains($0) }
                      && (0...960).contains(gate.backoff)
              }),
              state.readings.compactMap({ $0 }).allSatisfy({ [$0.five, $0.seven].compactMap { $0 }.allSatisfy { (0...100).contains($0) } }) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        return state
    }
    func save(_ state: OfflineState) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try DurableFile.replace(url: url, data: JSONEncoder().encode(state))
    }
}

// Shortest time between two requests to one service, and the refresh period: the
// shared Rust value, which is also what Windows uses.
let requestGap = iusage_min_gap_secs()

// The entire runtime is main-thread/serial-owner only. No live transport exists.
final class OfflineRuntime {
    private(set) var state: OfflineState
    private(set) var errors: [String?] = [nil, nil]
    // Credential problems found before any request. They are newer than a fetch
    // error and are cleared once a request completes, which proves the login works.
    private(set) var localIssues: [String?] = [nil, nil]
    private let save: (OfflineState) throws -> Void
    init(state: OfflineState, save: @escaping (OfflineState) throws -> Void) {
        self.state = state; self.save = save
    }
    func issue(_ provider: Int) -> String? {
        guard (0...1).contains(provider) else { return nil }
        if provider == 0 && state.claudeSessionExpired == true { return "SessionKey 已失效，请重新配置 Claude 登录" }
        return localIssues[provider] ?? errors[provider]
    }
    func setLocalIssue(_ provider: Int, _ text: String?) {
        guard (0...1).contains(provider) else { return }
        localIssues[provider] = text
    }
    func configureClaude(identity: String?) throws {
        var updated = state
        if updated.claudeSessionIdentity != identity {
            updated.readings[0] = nil
            if updated.alertMarks?.count == 2 { updated.alertMarks?[0] = [AlertMark(), AlertMark()] }
        }
        updated.claudeSessionIdentity = identity
        updated.claudeOrganization = nil
        updated.claudeSessionExpired = false
        try save(updated); state = updated
        localIssues[0] = nil; errors[0] = nil
    }
    func completeClaudeSession(now: Int64, status: UInt16, retryAfter: UInt32, body: String, organization: String?, discovery: Bool) throws {
        if status == 401 { state.claudeSessionExpired = true }
        if discovery {
            if status == 200 { state.claudeOrganization = organization }
            try completeRefresh(provider: 0, now: now, status: status, retryAfter: retryAfter,
                                label: status == 200 ? nil : "Claude 登录或组织读取失败，请检查 SessionKey 和组织 ID")
        } else {
            try complete(provider: 0, now: now, status: status, retryAfter: retryAfter, body: body)
        }
        if status == 401 { errors[0] = "SessionKey 已失效，请重新配置 Claude 登录" }
    }
    func select(_ provider: Int) throws {
        guard (0...1).contains(provider) else { return }
        var updated = state; updated.selectedProvider = provider
        try save(updated); state = updated
    }
    func setRemaining(_ remaining: Bool) throws {
        var updated = state
        updated.showRemaining = remaining
        try save(updated)
        state = updated
    }
    func setNotifications(_ enabled: Bool) throws {
        var updated = state; updated.notifications = enabled
        try save(updated); state = updated
    }
    func consumeAlert(provider: Int, now: Int64) throws -> [String] {
        guard state.notifications == true, provider == state.selectedProvider,
              issue(provider) == nil, let reading = state.readings[provider], reading.fetchedAt >= state.gates[provider].lastAttempt else { return [] }
        var updated = state
        var marks = updated.alertMarks ?? [[AlertMark(), AlertMark()], [AlertMark(), AlertMark()]]
        guard marks.count == 2, marks.allSatisfy({ $0.count == 2 }) else { throw CocoaError(.fileReadCorruptFile) }
        let lines = AlertPolicy.evaluate(reading: reading, marks: &marks[provider], now: now)
        updated.alertMarks = marks
        try save(updated); state = updated
        return lines
    }
    func nextEvent(provider: Int, now: Int64, idle: Bool) -> Int64 {
        if inFlight.contains(provider) { return now + requestGap }
        let gate = state.gates[provider]
        let idleUntil = idle ? (state.readings[provider]?.fetchedAt ?? 0) + 900 : 0
        return max(now, gate.nextPoll, gate.blockedUntil, gate.lastAttempt + requestGap, idleUntil)
    }
    // Mock response only; production networking must use a separate explicit adapter.
    @discardableResult
    func simulate(provider: Int, now: Int64, status: UInt16, retryAfter: UInt32 = 0,
                  body: String, onSend: () -> Void = {}) throws -> Bool {
        guard try reserve(provider: provider, now: now) else { return false }
        onSend()
        try complete(provider: provider, now: now, status: status, retryAfter: retryAfter, body: body)
        return true
    }
    private var inFlight: Set<Int> = []
    func canRequest(provider: Int, now: Int64) -> Bool {
        guard (0...1).contains(provider), !inFlight.contains(provider) else { return false }
        var gate = state.gates[provider].native
        return iusage_can_request(&gate, now)
    }
    func reserve(provider: Int, now: Int64) throws -> Bool {
        guard (0...1).contains(provider), !inFlight.contains(provider) else { return false }
        var gate = state.gates[provider].native
        guard iusage_start(&gate, now) else { return false }
        var reserved = state; reserved.gates[provider] = SavedGate(gate)
        try save(reserved)
        state = reserved
        inFlight.insert(provider)
        return true
    }
    // Ends a reservation made for a token renewal, which is a request to the same
    // service and uses the same gate. Success schedules the usage request one minimum
    // gap later; any other answer gets the usual wait. `label` is what the user sees.
    func completeRefresh(provider: Int, now: Int64, status: UInt16, retryAfter: UInt32 = 0, label: String?) throws {
        guard inFlight.remove(provider) != nil else { return }
        localIssues[provider] = nil
        var gate = state.gates[provider].native
        _ = iusage_complete(&gate, now, status, retryAfter, UInt32(requestGap))
        state.gates[provider] = SavedGate(gate)
        errors[provider] = status == 200 ? nil : label
        try save(state)
    }
    func complete(provider: Int, now: Int64, status: UInt16, retryAfter: UInt32 = 0, body: String) throws {
        guard inFlight.remove(provider) != nil else { return }
        localIssues[provider] = nil
        var gate = state.gates[provider].native
        var usage = MacUsage()
        let data = Data(body.utf8)
        let parsed = status == 200 && data.withUnsafeBytes { bytes in
            iusage_parse(UInt32(provider), bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, &usage)
        } == 0
        // Malformed 200 must not overwrite the last successful reading.
        let effectiveStatus: UInt16 = status == 200 && !parsed ? 502 : status
        _ = iusage_complete(&gate, now, effectiveStatus, retryAfter, UInt32(requestGap))
        state.gates[provider] = SavedGate(gate)
        errors[provider] = parsed ? nil : status == 429 ? "限流" : status == 401 || status == 403 ? "凭据被拒绝" : status == 0 ? "离线" : status == 200 ? "数据错误" : "HTTP \(status)"
        if parsed { state.readings[provider] = SavedReading(five: usage.present & 1 != 0 ? usage.five_hour : nil, seven: usage.present & 2 != 0 ? usage.seven_day : nil, fetchedAt: now, fiveReset: usage.present & 4 != 0 ? usage.five_reset : nil, sevenReset: usage.present & 8 != 0 ? usage.seven_reset : nil) }
        try save(state)
    }
}

final class InstanceLock {
    private let descriptor: Int32
    init(url: URL) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        descriptor = Darwin.open(url.path, O_CREAT | O_RDWR | O_NOFOLLOW, S_IRUSR | S_IWUSR)
        guard descriptor >= 0 else { throw CocoaError(.fileWriteNoPermission) }
        guard iusage_flock(descriptor, LOCK_EX | LOCK_NB) == 0 else {
            Darwin.close(descriptor)
            throw CocoaError(.fileLocking)
        }
        _ = fcntl(descriptor, F_SETFD, FD_CLOEXEC)
    }
    deinit { Darwin.close(descriptor) }
}
