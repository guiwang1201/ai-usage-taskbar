import AppKit

final class UsageBars: NSView {
    var values: [Double?] = [PreviewUsage.claude.five / 100, PreviewUsage.claude.seven / 100] { didSet { needsDisplay = true } }
    var remaining = false { didSet { needsDisplay = true } }
    var resets: [Int64?] = [nil, nil] { didSet { needsDisplay = true } }
    var stale = false { didSet { needsDisplay = true } }
    var provider = "C" { didSet { needsDisplay = true } }
    var compact = false { didSet { needsDisplay = true } }
    func requiredWidth(compact: Bool) -> CGFloat {
        let attributes: [NSAttributedString.Key: Any] = [.font: NSFont.monospacedDigitSystemFont(ofSize: 9, weight: .medium)]
        let now = Int64(Date().timeIntervalSince1970)
        let width = resets.map { (($0.map { DisplayFormat.countdown($0 - now) } ?? "--") as NSString).size(withAttributes: attributes).width }.max() ?? 0
        return ceil((compact ? 87 : 123) + width + 3)
    }
    override var intrinsicContentSize: NSSize { NSSize(width: 148, height: 22) }
    override func draw(_ dirtyRect: NSRect) {
        let trackX: CGFloat = compact ? 35 : 40
        let trackWidth: CGFloat = compact ? 24 : 46
        let percentRight: CGFloat = compact ? 83 : 119
        let resetX: CGFloat = compact ? 87 : 123
        alphaValue = stale ? 0.55 : 1
        let offset = (bounds.height - 22) / 2
        let text: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 9, weight: .medium),
            .foregroundColor: NSColor.labelColor
        ]
        (provider as NSString).draw(at: NSPoint(x: 3, y: offset + 4), withAttributes: [
            .font: NSFont.systemFont(ofSize: 11, weight: .bold),
            .foregroundColor: provider == "C" ? NSColor.systemOrange : NSColor.systemBlue
        ])
        for (index, raw) in values.prefix(2).enumerated() {
            let used = raw.map { $0.isFinite ? min(1, max(0, $0)) : 0 } ?? 0
            let value = remaining && raw != nil ? 1 - used : used
            let y = offset + (index == 0 ? 11 : 0)
            ((index == 0 ? "5h" : "7d") as NSString).draw(at: NSPoint(x: 18, y: y), withAttributes: text)
            let track = NSRect(x: trackX, y: y + 3, width: trackWidth, height: 5)
            NSColor.labelColor.withAlphaComponent(0.16).setFill()
            NSBezierPath(roundedRect: track, xRadius: 1.5, yRadius: 1.5).fill()
            (used >= 0.9 ? NSColor.systemRed : used >= 0.75 ? .systemYellow : .systemGreen).setFill()
            if value > 0 {
                NSBezierPath(roundedRect: NSRect(x: trackX, y: y + 3, width: trackWidth * value, height: 5), xRadius: 1.5, yRadius: 1.5).fill()
            }
            let percent = (raw == nil ? "--" : "\(Int((value * 100).rounded()))%") as NSString
            let reset = resets[index].map { DisplayFormat.countdown($0 - Int64(Date().timeIntervalSince1970)) } ?? "--"
            (reset as NSString).draw(at: NSPoint(x: resetX, y: y), withAttributes: text)
            percent.draw(at: NSPoint(x: percentRight - percent.size(withAttributes: text).width, y: y), withAttributes: text)
        }
    }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override func viewDidChangeEffectiveAppearance() { needsDisplay = true }
}
