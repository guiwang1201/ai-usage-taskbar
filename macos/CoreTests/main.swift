import Foundation
var usage = MacUsage()
let body = Data(#"{"five_hour":{"utilization":32,"resets_at":"2026-10-03T00:00:00Z"},"seven_day":{"utilization":64}}"#.utf8)
let status = body.withUnsafeBytes { bytes in
    iusage_parse(0, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, &usage)
}
precondition(status == 0 && usage.five_hour == 32 && usage.seven_day == 64 && usage.present == 7)
var gate = MacGate()
precondition(iusage_start(&gate, 1000))
precondition(!iusage_start(&gate, 1119))
precondition(iusage_can_request(&gate, 1120))
precondition(iusage_complete(&gate, 1120, 429, 600, 180))
precondition(!iusage_can_request(&gate, 1719))
precondition(iusage_can_request(&gate, 1720))
let restored = gate
precondition(restored.blocked_until == 1720 && restored.backoff == 300)
precondition(iusage_complete(&gate, 1720, 429, 0, 180))
precondition(gate.backoff == 600 && gate.blocked_until == 2320)
precondition(iusage_complete(&gate, 2320, 200, 0, 180))
precondition(gate.backoff == 0 && gate.next_poll == 2500)
print("PASS: Swift/Rust ABI parsing and request-gate integration (offline)")
