# Optional Windows asset maintenance; normal builds use the committed PNG and ICO.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$project = Split-Path -Parent $PSScriptRoot
$assets = Join-Path $project 'assets/branding'
$master = [Drawing.Image]::FromFile((Join-Path $assets 'moon-master.png'))
$frames = [Collections.Generic.List[byte[]]]::new()
$sizes = @(16, 20, 24, 32, 40, 48, 64, 128, 256)
# The icon: the master moon, scaled to leave room around it, in front of a warm yellow
# glow that is strongest at the moon's edge and fades out at the canvas edge. It is
# composed once at 1024 px and every size is scaled from it.
$side = 1024
$moonScale = 0.84
$source = [Drawing.Bitmap]::new($side, $side, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
$compose = [Drawing.Graphics]::FromImage($source)
$glowPath = [Drawing.Drawing2D.GraphicsPath]::new()
try {
    $compose.CompositingQuality = [Drawing.Drawing2D.CompositingQuality]::HighQuality
    $compose.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $compose.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $compose.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $compose.Clear([Drawing.Color]::Transparent)
    $glowPath.AddEllipse(0, 0, $side, $side)
    $glow = [Drawing.Drawing2D.PathGradientBrush]::new($glowPath)
    try {
        # The master's moon spans 88 % of its canvas, so after scaling its edge lies at
        # 0.88 * $moonScale of the radius; the glow rises from clear at the canvas edge
        # to full just inside the moon's edge (blend positions run from the outside in).
        $moonEdge = 0.88 * $moonScale
        $blend = [Drawing.Drawing2D.ColorBlend]::new(4)
        $blend.Colors = @(
            [Drawing.Color]::FromArgb(0, 255, 214, 92),
            [Drawing.Color]::FromArgb(70, 255, 214, 92),
            [Drawing.Color]::FromArgb(210, 255, 220, 110),
            [Drawing.Color]::FromArgb(210, 255, 220, 110))
        $blend.Positions = [single[]]@(0.0, [single]((1 - $moonEdge) * 0.55), [single](1 - $moonEdge + 0.02), 1.0)
        $glow.InterpolationColors = $blend
        $compose.FillEllipse($glow, 0, 0, $side, $side)
    } finally {
        $glow.Dispose()
    }
    $moonSide = [int][math]::Round($side * $moonScale)
    $offset = [int](($side - $moonSide) / 2)
    $compose.DrawImage($master, [Drawing.Rectangle]::new($offset, $offset, $moonSide, $moonSide))
} finally {
    $glowPath.Dispose()
    $compose.Dispose()
    $master.Dispose()
}
try {
    foreach ($size in $sizes) {
        $bitmap = [Drawing.Bitmap]::new($size, $size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $stream = [IO.MemoryStream]::new()
        try {
            $graphics.CompositingMode = [Drawing.Drawing2D.CompositingMode]::SourceCopy
            $graphics.CompositingQuality = [Drawing.Drawing2D.CompositingQuality]::HighQuality
            $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $graphics.DrawImage($source, [Drawing.Rectangle]::new(0, 0, $size, $size))
            $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
            $frames.Add($stream.ToArray())
            if ($size -eq 256) {
                [IO.File]::WriteAllBytes((Join-Path $assets 'midnight-explorer.png'), $frames[-1])
            }
        } finally {
            $stream.Dispose()
            $graphics.Dispose()
            $bitmap.Dispose()
        }
    }
    $stream = [IO.File]::Create((Join-Path $assets 'midnight-explorer.ico'))
    $writer = [IO.BinaryWriter]::new($stream)
    try {
        $writer.Write([uint16]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]$sizes.Count)
        $offset = 6 + 16 * $sizes.Count
        for ($index = 0; $index -lt $sizes.Count; $index++) {
            $dimension = if ($sizes[$index] -eq 256) { 0 } else { $sizes[$index] }
            $writer.Write([byte]$dimension)
            $writer.Write([byte]$dimension)
            $writer.Write([uint16]0)
            $writer.Write([uint16]1)
            $writer.Write([uint16]32)
            $writer.Write([uint32]$frames[$index].Length)
            $writer.Write([uint32]$offset)
            $offset += $frames[$index].Length
        }
        foreach ($frame in $frames) { $writer.Write($frame) }
    } finally {
        $writer.Dispose()
        $stream.Dispose()
    }
} finally {
    $source.Dispose()
}
Write-Output "Created PNG and multi-resolution ICO in $assets"
