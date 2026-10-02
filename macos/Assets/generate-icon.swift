import AppKit

// Native vector rendering of the shared assets/iusage.svg design.
let output = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
func color(_ hex: UInt32) -> NSColor {
    NSColor(srgbRed: CGFloat((hex >> 16) & 255) / 255, green: CGFloat((hex >> 8) & 255) / 255, blue: CGFloat(hex & 255) / 255, alpha: 1)
}
for size in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let pixels = size * scale
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
        let transform = AffineTransform(scale: CGFloat(pixels) / 256)
        (transform as NSAffineTransform).concat()
        func rounded(_ rect: NSRect, _ radius: CGFloat, _ fill: UInt32) {
            color(fill).setFill()
            NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius).fill()
        }
        rounded(NSRect(x: 24, y: 24, width: 208, height: 208), 48, 0x17243b)
        rounded(NSRect(x: 55, y: 142, width: 146, height: 40), 20, 0x33445f)
        rounded(NSRect(x: 55, y: 142, width: 113, height: 40), 20, 0x36d9ef)
        rounded(NSRect(x: 55, y: 74, width: 146, height: 40), 20, 0x33445f)
        rounded(NSRect(x: 55, y: 74, width: 77, height: 40), 20, 0xffad66)
        NSGraphicsContext.restoreGraphicsState()
        let suffix = scale == 2 ? "@2x" : ""
        try bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent("icon_\(size)x\(size)\(suffix).png"))
    }
}
