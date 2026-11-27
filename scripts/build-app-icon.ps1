# Optional Windows asset maintenance; normal builds use the committed PNG and ICO.
param(
    [ValidateSet('origami-trail', 'interleave')]
    [string]$Variant
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$project = Split-Path -Parent $PSScriptRoot
$assets = Join-Path $project 'assets/branding'
$frames = [Collections.Generic.List[byte[]]]::new()
$icoSizes = @(16, 20, 24, 32, 40, 48, 64, 128, 256)

# Without -Variant: the default Harvest Navigator icon in assets/branding. With it: the
# alternate design in assets/branding/variants/<name>. Each folder holds its master.png
# and receives the window PNG, the app UI sizes and the multi-resolution ICO.
if ($Variant) {
    $assets = Join-Path $assets "variants/$Variant"
}
$uiDir = [IO.Directory]::CreateDirectory((Join-Path $assets 'ui'))
# Source rectangles in master-image pixels. The wider rectangles give the two
# folders a taller profile when rendered into a square taskbar icon.
$crop = switch ($Variant) {
    'origami-trail' { [Drawing.Rectangle]::new(63, 135, 1128, 938) }
    'interleave'    { [Drawing.Rectangle]::new(146, 126, 966, 966) }
    default         { [Drawing.Rectangle]::new(90, 123, 1076, 956) }
}
$source = [Drawing.Image]::FromFile((Join-Path $assets 'master.png'))
try {
    # About uses a 72px logo, with larger sizes on scaled displays.
    # Generate those extra UI sizes after the ICO frames to keep their order.
    foreach ($size in ($icoSizes + @(72, 96, 144))) {
        $bitmap = [Drawing.Bitmap]::new($size, $size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $stream = [IO.MemoryStream]::new()
        try {
            $graphics.CompositingMode = [Drawing.Drawing2D.CompositingMode]::SourceCopy
            $graphics.CompositingQuality = [Drawing.Drawing2D.CompositingQuality]::HighQuality
            $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $destination = [Drawing.Rectangle]::new(0, 0, $size, $size)
            $graphics.DrawImage($source, $destination, $crop, [Drawing.GraphicsUnit]::Pixel)
            $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
            $png = $stream.ToArray()
            if ($size -in $icoSizes) {
                $frames.Add($png)
            }
            if ($size -eq 256) {
                $path = Join-Path $assets 'midnight-explorer.png'
            } else {
                $path = Join-Path $uiDir.FullName "$size.png"
            }
            [IO.File]::WriteAllBytes($path, $png)
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
        $writer.Write([uint16]$icoSizes.Count)
        $offset = 6 + 16 * $icoSizes.Count
        for ($index = 0; $index -lt $icoSizes.Count; $index++) {
            $dimension = if ($icoSizes[$index] -eq 256) { 0 } else { $icoSizes[$index] }
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
