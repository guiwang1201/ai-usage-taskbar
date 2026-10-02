import Foundation

struct AlertMark: Codable {
    var level = 0
    var reset: Int64?
}
enum AlertPolicy {
    static func evaluate(reading: SavedReading, marks: inout [AlertMark], now: Int64) -> [String] {
        let values = reading.values(now: now)
        let resets = [reading.fiveReset, reading.sevenReset]
        var lines: [String] = []
        for i in 0...1 {
            if let before = marks[i].reset, let after = resets[i], after > before { marks[i].level = 0 }
            marks[i].reset = resets[i]
            let used = Int((values[i] ?? 0).rounded())
            func level(_ n: Int) -> Int { n >= 90 ? 2 : n >= 75 ? 1 : 0 }
            marks[i].level = min(marks[i].level, level(used + 5))
            let current = level(used)
            if current > marks[i].level {
                marks[i].level = current
                lines.append("\(i == 0 ? "5 小时" : "7 天")额度已用 \(used)%")
            }
        }
        return lines
    }
}
