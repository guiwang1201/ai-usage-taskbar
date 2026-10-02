import AppKit
import UserNotifications
import ServiceManagement

final class SystemFeatures: NSObject, UNUserNotificationCenterDelegate {
    private static let notificationDelegate = SystemFeatures()
    static func configureNotifications() {
        UNUserNotificationCenter.current().delegate = notificationDelegate
    }
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .list])
    }
    static var installed: Bool { Bundle.main.bundleURL.path.hasPrefix("/Applications/") || Bundle.main.bundleURL.path.hasPrefix(NSHomeDirectory() + "/Applications/") }
    static var startupEnabled: Bool { if #available(macOS 13, *) { return SMAppService.mainApp.status == .enabled }; return false }
    static func setStartup(_ enabled: Bool) throws {
        guard installed else { throw CocoaError(.featureUnsupported) }
        if #available(macOS 13, *) {
            if enabled { try SMAppService.mainApp.register() }
            else { try SMAppService.mainApp.unregister() }
        } else { throw CocoaError(.featureUnsupported) }
    }
    static func requestNotifications(_ callback: @escaping (Bool) -> Void) {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert]) { granted, _ in
            DispatchQueue.main.async { callback(granted) }
        }
    }
    static func notify(provider: String, lines: [String]) {
        let content = UNMutableNotificationContent()
        content.title = "IUsage · \(provider) 低额度提醒"
        content.body = lines.joined(separator: "\n")
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
    }
}
