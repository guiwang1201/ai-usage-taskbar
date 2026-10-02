import Foundation

enum DisplayFormat {
    static func countdown(_ seconds: Int64) -> String {
        if seconds <= 0 { return "--" }
        if seconds >= 86400 { return "\(seconds / 86400)d" }
        if seconds >= 3600 { return "\(seconds / 3600)h" }
        return "\(max(1, seconds / 60))m"
    }
    static func longCountdown(_ seconds: Int64) -> String {
        if seconds <= 0 { return "已重置" }
        if seconds >= 86400 { return "\(seconds / 86400) 天 \(seconds % 86400 / 3600) 小时" }
        if seconds >= 3600 { return "\(seconds / 3600) 小时 \(seconds % 3600 / 60) 分" }
        return "\(max(1, seconds / 60)) 分钟"
    }
}
