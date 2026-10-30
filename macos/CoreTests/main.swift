import Foundation
var usage = MacUsage()
let body = Data(#"{"five_hour":{"utilization":32,"resets_at":"2026-10-03T00:00:00Z"},"seven_day":{"utilization":64}}"#.utf8)
let status = body.withUnsafeBytes { bytes in
    iusage_parse(0, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, &usage)
}
precondition(status == 0 && usage.five_hour == 32 && usage.seven_day == 64 && usage.present == 7)
precondition(iusage_min_gap_secs() == 65)
var gate = MacGate()
precondition(iusage_start(&gate, 1000))
precondition(!iusage_start(&gate, 1064))
precondition(iusage_can_request(&gate, 1065))
precondition(iusage_complete(&gate, 1065, 429, 600, 65))
precondition(!iusage_can_request(&gate, 1664))
precondition(iusage_can_request(&gate, 1665))
let restored = gate
precondition(restored.blocked_until == 1665 && restored.backoff == 300)
precondition(iusage_complete(&gate, 1665, 429, 0, 65))
precondition(gate.backoff == 600 && gate.blocked_until == 2265)
precondition(iusage_complete(&gate, 2265, 200, 0, 65))
precondition(gate.backoff == 0 && gate.next_poll == 2330)
print("PASS: Swift/Rust ABI parsing and request-gate integration (offline)")
