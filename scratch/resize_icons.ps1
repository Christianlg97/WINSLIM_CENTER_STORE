# Reduce los PNG del catálogo que superan el lado máximo indicado.
# GDI+ forma parte de Windows, así que esto no añade dependencias al proyecto.
# Lo invoca scratch/optimize_catalog_icons.mjs.
param(
  [Parameter(Mandatory = $true)][string]$IconDirectory,
  [Parameter(Mandatory = $true)][int]$MaxSide
)

Add-Type -AssemblyName System.Drawing

foreach ($file in Get-ChildItem -Path $IconDirectory -Filter *.png) {
  $image = $null
  $canvas = $null
  $graphics = $null
  $temporary = "$($file.FullName).tmp"
  try {
    $image = [System.Drawing.Image]::FromFile($file.FullName)
    $side = [Math]::Max($image.Width, $image.Height)
    if ($side -le $MaxSide) { continue }

    $scale = $MaxSide / $side
    $width = [int][Math]::Max(1, [Math]::Round($image.Width * $scale))
    $height = [int][Math]::Max(1, [Math]::Round($image.Height * $scale))
    $canvas = New-Object System.Drawing.Bitmap($width, $height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($canvas)
    # SourceCopy conserva el canal alfa en vez de mezclarlo con el fondo.
    $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
    $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $graphics.DrawImage($image, 0, 0, $width, $height)
    $graphics.Dispose(); $graphics = $null
    $image.Dispose(); $image = $null

    $canvas.Save($temporary, [System.Drawing.Imaging.ImageFormat]::Png)
    $canvas.Dispose(); $canvas = $null

    $shrunk = Get-Item $temporary
    if ($shrunk.Length -lt $file.Length) {
      Move-Item -Force -Path $temporary -Destination $file.FullName
      "  $($file.Name) $($file.Length) -> $($shrunk.Length) ($width x $height)"
    } else {
      Remove-Item -Force $temporary
    }
  } catch {
    "  [aviso] $($file.Name): $($_.Exception.Message)"
  } finally {
    if ($graphics) { $graphics.Dispose() }
    if ($canvas) { $canvas.Dispose() }
    if ($image) { $image.Dispose() }
    if (Test-Path $temporary) { Remove-Item -Force $temporary }
  }
}
