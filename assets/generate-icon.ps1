# Regenerate the small, code-drawn application icon. No image tools or network needed.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$frames = @()
foreach ($size in @(16, 24, 32, 48, 64, 128, 256)) {
    $bitmap = New-Object System.Drawing.Bitmap ($size * 4), ($size * 4)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $graphics.ScaleTransform(($size * 4 / 256.0), ($size * 4 / 256.0))
    function Fill-RoundRect($x, $y, $width, $height, $radius, $hex) {
        $path = New-Object System.Drawing.Drawing2D.GraphicsPath
        $d = $radius * 2
        $path.AddArc($x, $y, $d, $d, 180, 90)
        $path.AddArc(($x + $width - $d), $y, $d, $d, 270, 90)
        $path.AddArc(($x + $width - $d), ($y + $height - $d), $d, $d, 0, 90)
        $path.AddArc($x, ($y + $height - $d), $d, $d, 90, 90)
        $path.CloseFigure()
        $brush = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml($hex))
        $graphics.FillPath($brush, $path)
        $brush.Dispose()
        $path.Dispose()
    }
    Fill-RoundRect 8 8 240 240 56 '#17243B'
    Fill-RoundRect 44 66 168 46 23 '#33445F'
    Fill-RoundRect 44 66 130 46 23 '#36D9EF'
    Fill-RoundRect 44 144 168 46 23 '#33445F'
    Fill-RoundRect 44 144 88 46 23 '#FFAD66'
    $graphics.Dispose()
    $small = New-Object System.Drawing.Bitmap $size, $size
    $g = [System.Drawing.Graphics]::FromImage($small)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.DrawImage($bitmap, 0, 0, $size, $size)
    $g.Dispose()
    $bitmap.Dispose()
    $stream = New-Object System.IO.MemoryStream
    $small.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $bytes = $stream.ToArray()
    $frames += [pscustomobject]@{ Size = $size; Bytes = $bytes }
    if ($size -eq 256) { [System.IO.File]::WriteAllBytes((Join-Path $PSScriptRoot 'iusage.png'), $bytes) }
    $stream.Dispose()
    $small.Dispose()
}
$output = New-Object System.IO.MemoryStream
$writer = New-Object System.IO.BinaryWriter $output
$writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$frames.Count)
$offset = 6 + 16 * $frames.Count
foreach ($frame in $frames) {
    $dimension = if ($frame.Size -eq 256) { 0 } else { $frame.Size }
    $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]32)
    $writer.Write([uint32]$frame.Bytes.Length); $writer.Write([uint32]$offset)
    $offset += $frame.Bytes.Length
}
foreach ($frame in $frames) { $writer.Write([byte[]]$frame.Bytes) }
[System.IO.File]::WriteAllBytes((Join-Path $PSScriptRoot 'iusage.ico'), $output.ToArray())
$writer.Dispose(); $output.Dispose()
