import Foundation

// Mock responses travel through the same Rust parser as the Windows application.
// This file has no networking and never obtains credentials.
struct PreviewUsage {
    let five: Double
    let seven: Double
    static func parse(provider: UInt32, body: String) -> PreviewUsage {
        let data = Data(body.utf8)
        var parsed = MacUsage()
        let status = data.withUnsafeBytes { bytes in
            iusage_parse(provider, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, &parsed)
        }
        precondition(status == 0 && parsed.present & 3 == 3, "Invalid bundled mock response")
        return PreviewUsage(five: parsed.five_hour, seven: parsed.seven_day)
    }
    static let claude = parse(provider: 0, body: #"{"five_hour":{"utilization":32},"seven_day":{"utilization":64}}"#)
    static let codex = parse(provider: 1, body: #"{"rate_limit":{"primary_window":{"used_percent":12,"limit_window_seconds":18000},"secondary_window":{"used_percent":82,"limit_window_seconds":604800}}}"#)
}
