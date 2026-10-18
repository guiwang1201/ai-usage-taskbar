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
            // Reset times drift by a second or so between readings; only a passed reset is a new window.
            if let before = marks[i].reset, before <= now, resets[i] != nil { marks[i].level = 0 }
            marks[i].reset = resets[i]
            let used = Int((values[i] ?? 0).rounded())
            // One reminder each at 20%, 10% and 5% left, as on Windows.
            func level(_ n: Int) -> Int { [20, 10, 5].filter { 100 - n <= $0 }.count }
            marks[i].level = min(marks[i].level, level(used + 5))
            let current = level(used)
            if current > marks[i].level {
                marks[i].level = current
                lines.append("\(i == 0 ? "5 小时" : "7 天")额度剩余 \(100 - used)%")
            }
        }
        return lines
    }
}
