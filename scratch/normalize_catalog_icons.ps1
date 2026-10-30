# Normaliza los PNG usados por el catalogo sin cambiar el aspecto que tenia
# object-fit en el avatar: contain anade transparencia y cover recorta.
param(
  [string]$ProjectRoot = (Join-Path $PSScriptRoot ".."),
  [int]$MaxSide = 256
)

Add-Type -AssemblyName System.Drawing

$root = [IO.Path]::GetFullPath($ProjectRoot)
$catalogPath = Join-Path $root "src-tauri\apps.json"
$apps = Get-Content -Raw -LiteralPath $catalogPath | ConvertFrom-Json
$seen = @{}

foreach ($app in $apps) {
  $relative = [string]$app.icon_url
  if (-not $relative.EndsWith(".png", [StringComparison]::OrdinalIgnoreCase)) { continue }
  if ($seen.ContainsKey($relative)) { continue }
  $seen[$relative] = $true

  $path = Join-Path (Join-Path $root "src") ($relative.Replace("/", "\"))
  if (-not (Test-Path -LiteralPath $path)) { throw "No existe el icono de $($app.id): $relative" }
  $path = (Resolve-Path -LiteralPath $path).Path

  $image = $null
  $canvas = $null
  $graphics = $null
  $temporary = "$path.normalize.tmp"
  try {
    $image = [System.Drawing.Image]::FromFile($path)
    $width = $image.Width
    $height = $image.Height
    if ($width -eq $height -and $width -le $MaxSide) { continue }

    $contain = [string]$app.icon_fit -eq "contain"
    $sourceX = 0.0
    $sourceY = 0.0
    $sourceWidth = [double]$width
    $sourceHeight = [double]$height

    if ($contain) {
      $logicalSide = [double][Math]::Max($width, $height)
      $outputSide = [int][Math]::Min($logicalSide, $MaxSide)
      $scale = $outputSide / $logicalSide
      $destinationX = ($logicalSide - $width) * $scale / 2
      $destinationY = ($logicalSide - $height) * $scale / 2
      $destinationWidth = $width * $scale
      $destinationHeight = $height * $scale
    } else {
      $logicalSide = [double][Math]::Min($width, $height)
      $outputSide = [int][Math]::Min($logicalSide, $MaxSide)
      $sourceWidth = $logicalSide
      $sourceHeight = $logicalSide
      $sourceX = if ([string]$app.icon_position -eq "left") { 0 } else { ($width - $logicalSide) / 2 }
      $sourceY = ($height - $logicalSide) / 2
      $destinationX = 0.0
      $destinationY = 0.0
      $destinationWidth = [double]$outputSide
      $destinationHeight = [double]$outputSide
    }

    $canvas = New-Object System.Drawing.Bitmap($outputSide, $outputSide, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($canvas)
    $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
    $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $graphics.Clear([System.Drawing.Color]::Transparent)
    $destination = New-Object System.Drawing.RectangleF($destinationX, $destinationY, $destinationWidth, $destinationHeight)
    $source = New-Object System.Drawing.RectangleF($sourceX, $sourceY, $sourceWidth, $sourceHeight)
    $graphics.DrawImage($image, $destination, $source, [System.Drawing.GraphicsUnit]::Pixel)
    $graphics.Dispose(); $graphics = $null
    $image.Dispose(); $image = $null

    $canvas.Save($temporary, [System.Drawing.Imaging.ImageFormat]::Png)
    $canvas.Dispose(); $canvas = $null
    [IO.File]::Copy($temporary, $path, $true)
    Remove-Item -LiteralPath $temporary -Force
    Write-Output ("  {0}: {1}x{2} -> {3}x{3}" -f $relative, $width, $height, $outputSide)
  } finally {
    if ($graphics) { $graphics.Dispose() }
    if ($canvas) { $canvas.Dispose() }
    if ($image) { $image.Dispose() }
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
  }
}

