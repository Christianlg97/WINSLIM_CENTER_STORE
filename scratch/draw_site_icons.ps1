# Dibuja los iconos de las fichas web que no tienen un logotipo propio decente:
# sitios de descargas y de ROMs cuyo favicon es de 16 px o directamente no existe.
#
#   powershell -ExecutionPolicy Bypass -File scratch/draw_site_icons.ps1
#
# Cada icono sale a 256 px, a sangre y sin esquinas redondeadas: la ficha ya
# recorta con su propio radio, y el acceso directo de Windows quiere un mapa de
# bits cuadrado. Se dibujan con GDI+, que es parte de Windows y no añade
# dependencias, igual que hace resize_icons.ps1.
param(
  [string]$OutDir = (Join-Path (Split-Path -Parent $PSScriptRoot) "src\assets\catalog")
)

Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = "Stop"
$SIZE = 256

$installed = New-Object System.Drawing.Text.InstalledFontCollection
$families = $installed.Families | ForEach-Object { $_.Name }

# La primera de la lista que esté instalada; todas son de sistema, así que en la
# práctica manda "Segoe UI Black" y las demás son el seguro.
function Get-Font {
  param([string[]]$Candidates, [single]$Size, [System.Drawing.FontStyle]$Style = [System.Drawing.FontStyle]::Bold)
  foreach ($name in $Candidates) {
    if ($families -contains $name) { return New-Object System.Drawing.Font($name, $Size, $Style, [System.Drawing.GraphicsUnit]::Pixel) }
  }
  return New-Object System.Drawing.Font("Arial", $Size, $Style, [System.Drawing.GraphicsUnit]::Pixel)
}

function New-Canvas {
  $bitmap = New-Object System.Drawing.Bitmap($SIZE, $SIZE, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
  $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
  $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
  return @{ Bitmap = $bitmap; Graphics = $graphics }
}

function New-Gradient {
  param([string]$From, [string]$To)
  $rect = New-Object System.Drawing.Rectangle(0, 0, $SIZE, $SIZE)
  return New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    $rect,
    [System.Drawing.ColorTranslator]::FromHtml($From),
    [System.Drawing.ColorTranslator]::FromHtml($To),
    [System.Drawing.Drawing2D.LinearGradientMode]::ForwardDiagonal)
}

function New-RoundedPath {
  param([single]$X, [single]$Y, [single]$W, [single]$H, [single]$R)
  $path = New-Object System.Drawing.Drawing2D.GraphicsPath
  $d = $R * 2
  $path.AddArc($X, $Y, $d, $d, 180, 90)
  $path.AddArc($X + $W - $d, $Y, $d, $d, 270, 90)
  $path.AddArc($X + $W - $d, $Y + $H - $d, $d, $d, 0, 90)
  $path.AddArc($X, $Y + $H - $d, $d, $d, 90, 90)
  $path.CloseFigure()
  return $path
}

function Fill-Rounded {
  param($Graphics, $Brush, [single]$X, [single]$Y, [single]$W, [single]$H, [single]$R)
  $path = New-RoundedPath -X $X -Y $Y -W $W -H $H -R $R
  $Graphics.FillPath($Brush, $path)
  $path.Dispose()
}

# El texto se centra por su dibujo, no por la caja de la fuente: así el trazo
# queda en el medio óptico del icono y no colgando de la línea base.
function Add-CenteredText {
  param($Graphics, [string]$Text, $Font, $Brush, [single]$CenterX, [single]$CenterY, $ShadowBrush = $null, [single]$ShadowOffset = 6)
  $path = New-Object System.Drawing.Drawing2D.GraphicsPath
  $format = New-Object System.Drawing.StringFormat
  $path.AddString($Text, $Font.FontFamily, [int]$Font.Style, $Font.Size, (New-Object System.Drawing.PointF(0, 0)), $format)
  $bounds = $path.GetBounds()
  $matrix = New-Object System.Drawing.Drawing2D.Matrix
  $matrix.Translate($CenterX - ($bounds.X + $bounds.Width / 2), $CenterY - ($bounds.Y + $bounds.Height / 2))
  $path.Transform($matrix)
  if ($ShadowBrush) {
    $shadow = $path.Clone()
    $move = New-Object System.Drawing.Drawing2D.Matrix
    $move.Translate($ShadowOffset, $ShadowOffset)
    $shadow.Transform($move)
    $Graphics.FillPath($ShadowBrush, $shadow)
    $shadow.Dispose()
  }
  $Graphics.FillPath($Brush, $path)
  $path.Dispose()
  $format.Dispose()
  $matrix.Dispose()
}

function Save-Canvas {
  param($Canvas, [string]$Name)
  $path = Join-Path $OutDir $Name
  $Canvas.Graphics.Dispose()
  $Canvas.Bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $Canvas.Bitmap.Dispose()
  Write-Output "[iconos] $Name  ($((Get-Item $path).Length) bytes)"
}

$white = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)

# GamesFull: el mando verde de su propio favicon, redibujado a 256 px.
function Draw-GamesFull {
  $canvas = New-Canvas
  $g = $canvas.Graphics
  $bg = New-Gradient "#12D97A" "#038F4C"
  $g.FillRectangle($bg, 0, 0, $SIZE, $SIZE)

  $pad = New-Object System.Drawing.Drawing2D.GraphicsPath
  $pad.FillMode = [System.Drawing.Drawing2D.FillMode]::Winding
  $body = New-RoundedPath -X 34 -Y 92 -W 188 -H 78 -R 32
  $pad.AddPath($body, $false)
  $pad.AddEllipse(26, 96, 88, 92)
  $pad.AddEllipse(142, 96, 88, 92)
  $g.FillPath($white, $pad)
  $pad.Dispose(); $body.Dispose()

  # Los detalles se recortan repintando con el mismo degradado del fondo.
  Fill-Rounded -Graphics $g -Brush $bg -X 72 -Y 126 -W 56 -H 16 -R 7
  Fill-Rounded -Graphics $g -Brush $bg -X 92 -Y 106 -W 16 -H 56 -R 7
  $g.FillEllipse($bg, 156, 116, 24, 24)
  $g.FillEllipse($bg, 180, 140, 24, 24)

  $bg.Dispose()
  Save-Canvas $canvas "gamesfull.png"
}

# ElAmigosGames.net: su favicon es un "ela" de 244x188 sobre gris; aquí va el
# mismo texto sobre pizarra, que es lo que se lee a 48 px.
function Draw-ElAmigosGames {
  $canvas = New-Canvas
  $g = $canvas.Graphics
  $bg = New-Gradient "#2A3242" "#0D1117"
  $g.FillRectangle($bg, 0, 0, $SIZE, $SIZE)

  $red = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml("#E5484D"))
  $font = Get-Font -Candidates @("Segoe UI Black", "Arial Black", "Segoe UI") -Size 124
  Add-CenteredText -Graphics $g -Text "ela" -Font $font -Brush $white -CenterX 128 -CenterY 112 -ShadowBrush $red -ShadowOffset 7
  Fill-Rounded -Graphics $g -Brush $red -X 68 -Y 190 -W 120 -H 14 -R 7

  $font.Dispose(); $red.Dispose(); $bg.Dispose()
  Save-Canvas $canvas "elamigos_games.png"
}

# Emuparadise: la isla que dice su nombre — sol, mar y las iniciales.
function Draw-EmuParadise {
  $canvas = New-Canvas
  $g = $canvas.Graphics
  $bg = New-Gradient "#17C3C3" "#0A5FB4"
  $g.FillRectangle($bg, 0, 0, $SIZE, $SIZE)

  $sun = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(225, 255, 209, 102))
  $g.FillEllipse($sun, 178, 20, 54, 54)

  $sea = New-Object System.Drawing.Drawing2D.GraphicsPath
  $sea.AddBezier(0, 196, 64, 172, 192, 224, 256, 190)
  $sea.AddLine(256, 256, 0, 256)
  $sea.CloseFigure()
  $water = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(64, 255, 255, 255))
  $g.FillPath($water, $sea)

  $font = Get-Font -Candidates @("Segoe UI Black", "Arial Black", "Segoe UI") -Size 118
  Add-CenteredText -Graphics $g -Text "EP" -Font $font -Brush $white -CenterX 128 -CenterY 118

  $font.Dispose(); $sun.Dispose(); $water.Dispose(); $sea.Dispose(); $bg.Dispose()
  Save-Canvas $canvas "emuparadise.png"
}

# WoWRoms: un cartucho, que es exactamente lo que se descarga allí.
function Draw-WowRoms {
  $canvas = New-Canvas
  $g = $canvas.Graphics
  $bg = New-Gradient "#9061F9" "#4C1D95"
  $g.FillRectangle($bg, 0, 0, $SIZE, $SIZE)

  Fill-Rounded -Graphics $g -Brush $white -X 62 -Y 42 -W 132 -H 172 -R 16
  # Etiqueta, ranura y contactos, recortados repintando con el fondo.
  Fill-Rounded -Graphics $g -Brush $bg -X 82 -Y 64 -W 92 -H 62 -R 8
  Fill-Rounded -Graphics $g -Brush $bg -X 82 -Y 142 -W 92 -H 10 -R 5
  foreach ($x in 84, 108, 132, 156) { Fill-Rounded -Graphics $g -Brush $bg -X $x -Y 168 -W 16 -H 32 -R 4 }

  $bg.Dispose()
  Save-Canvas $canvas "wowroms.png"
}

# Retrostic: el nombre ya es un joystick de salón recreativo.
function Draw-Retrostic {
  $canvas = New-Canvas
  $g = $canvas.Graphics
  $bg = New-Gradient "#454C55" "#191C20"
  $g.FillRectangle($bg, 0, 0, $SIZE, $SIZE)

  $plate = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml("#ADB5BD"))
  $plateTop = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml("#DEE2E6"))
  $shaft = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml("#CED4DA"))
  $ball = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml("#E5484D"))
  $shine = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(120, 255, 255, 255))
  $button = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml("#FCC419"))

  Fill-Rounded -Graphics $g -Brush $plate -X 40 -Y 176 -W 176 -H 40 -R 18
  $g.FillEllipse($plateTop, 40, 156, 176, 40)
  Fill-Rounded -Graphics $g -Brush $shaft -X 114 -Y 92 -W 26 -H 84 -R 13
  $g.FillEllipse($ball, 88, 36, 78, 78)
  $g.FillEllipse($shine, 106, 50, 26, 18)
  $g.FillEllipse($button, 168, 164, 26, 26)
  $g.FillEllipse($button, 62, 168, 20, 20)

  foreach ($brush in $plate, $plateTop, $shaft, $ball, $shine, $button, $bg) { $brush.Dispose() }
  Save-Canvas $canvas "retrostic.png"
}

# DLPSGame: los cuatro símbolos del mando de PlayStation sobre el azul de la marca.
function Draw-DlpsGame {
  $canvas = New-Canvas
  $g = $canvas.Graphics
  $bg = New-Gradient "#1272E8" "#00368C"
  $g.FillRectangle($bg, 0, 0, $SIZE, $SIZE)

  $pen = New-Object System.Drawing.Pen([System.Drawing.Color]::White, 14)
  $pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
  $pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
  $pen.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round

  $g.DrawPolygon($pen, @(
      (New-Object System.Drawing.Point(84, 56)),
      (New-Object System.Drawing.Point(110, 104)),
      (New-Object System.Drawing.Point(58, 104))))
  $g.DrawEllipse($pen, 148, 56, 50, 50)
  $g.DrawLine($pen, 60, 152, 108, 200)
  $g.DrawLine($pen, 108, 152, 60, 200)
  $square = New-RoundedPath -X 148 -Y 152 -W 50 -H 50 -R 4
  $g.DrawPath($pen, $square)

  $square.Dispose(); $pen.Dispose(); $bg.Dispose()
  Save-Canvas $canvas "dlpsgame.png"
}

Draw-GamesFull
Draw-ElAmigosGames
Draw-EmuParadise
Draw-WowRoms
Draw-Retrostic
Draw-DlpsGame
$white.Dispose()
