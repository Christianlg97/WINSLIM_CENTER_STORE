<#
  Installs one piece of the toolchain build.cmd needs, so the build can carry on
  in the same run instead of asking to be launched again.

    -Component vctools   C++ Build Tools: MSVC compiler and Windows SDK
    -Component rust      rustup with the stable MSVC toolchain
    -Component node      Node.js LTS

  winget is tried first. When it is missing (it does not ship with every
  Windows build) or the install does not take, the vendor's own installer is
  downloaded from its official address instead. Whether a component counts as
  installed is decided by looking for it where it installs, never by an exit
  code: winget reports some successes as errors and some failures as success.

  Exit code 0 when the component is in place afterwards, 1 otherwise.
#>
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('vctools', 'rust', 'node')]
    [string]$Component
)

$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest crawls with its progress bar on PS 5.1
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$DownloadDir = Join-Path $env:TEMP 'winslimcenter-toolchain'

function Write-Info([string]$Message) { Write-Host "[INFO] $Message" }
function Write-Warn([string]$Message) { Write-Host "[ADVERTENCIA] $Message" }

function Invoke-Winget([string]$Id, [string[]]$Extra = @()) {
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        Write-Warn 'winget no esta disponible en este equipo.'
        return
    }
    Write-Info "Instalando $Id con winget..."
    $wingetArgs = @('install', '--id', $Id, '--exact', '--source', 'winget',
        '--accept-package-agreements', '--accept-source-agreements', '--disable-interactivity') + $Extra
    & winget @wingetArgs
    if ($LASTEXITCODE -ne 0) { Write-Warn "winget termino con codigo $LASTEXITCODE." }
}

function Get-Installer([string]$Url, [string]$FileName) {
    New-Item -ItemType Directory -Force -Path $DownloadDir | Out-Null
    $destination = Join-Path $DownloadDir $FileName
    Write-Info "Descargando $Url"
    Invoke-WebRequest -Uri $Url -OutFile $destination -UseBasicParsing -ErrorAction Stop
    $destination
}

# Only an installer signed by its vendor is run.
function Assert-Signed([string]$Path) {
    $signature = Get-AuthenticodeSignature -FilePath $Path
    if ($signature.Status -ne 'Valid') {
        throw "La firma de $(Split-Path $Path -Leaf) no es valida ($($signature.Status)); no se ejecutara."
    }
}

# Machine-wide installers need the administrator token; Windows asks for it (UAC).
function Start-Elevated([string]$File, [string]$Arguments) {
    $process = Start-Process -FilePath $File -ArgumentList $Arguments -Verb RunAs -Wait -PassThru -ErrorAction Stop
    if ($process.ExitCode -eq 3010) {
        Write-Warn 'El instalador pide reiniciar Windows; se continua igualmente.'
    } elseif ($process.ExitCode -ne 0) {
        Write-Warn "$(Split-Path $File -Leaf) termino con codigo $($process.ExitCode)."
    }
}

# ------------------------------------------------------------------ vctools

$VsInstallerDir = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer'
$VsWhere = Join-Path $VsInstallerDir 'vswhere.exe'
$VsArgs = '--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'

function Test-VcTools {
    if (-not (Test-Path $VsWhere)) { return $false }
    $path = & $VsWhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    [bool]$path -and (Test-Path (Join-Path $path 'VC\Auxiliary\Build\vcvars64.bat'))
}

function Install-VcTools {
    # Build Tools already there but without the compiler: completed, not doubled.
    $existing = if (Test-Path $VsWhere) {
        & $VsWhere -latest -products Microsoft.VisualStudio.Product.BuildTools -property installationPath
    }
    if ($existing) {
        Write-Info "Agregando el compilador C++ a las Build Tools de $existing ..."
        Start-Elevated (Join-Path $VsInstallerDir 'setup.exe') "modify --installPath `"$existing`" $VsArgs"
        if (Test-VcTools) { return }
    }

    Invoke-Winget 'Microsoft.VisualStudio.2022.BuildTools' @('--override', $VsArgs)
    if (Test-VcTools) { return }

    Write-Warn 'Usando el instalador oficial de Microsoft...'
    $bootstrapper = Get-Installer 'https://aka.ms/vs/17/release/vs_BuildTools.exe' 'vs_BuildTools.exe'
    Assert-Signed $bootstrapper
    Start-Elevated $bootstrapper $VsArgs
}

# --------------------------------------------------------------------- rust

$CargoBin = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME 'bin' } else { Join-Path $env:USERPROFILE '.cargo\bin' }

function Test-Rust {
    Test-Path (Join-Path $CargoBin 'rustup.exe')
}

function Install-Rust {
    Invoke-Winget 'Rustlang.Rustup'
    if (Test-Rust) { return }

    Write-Warn 'Usando el instalador oficial de rust-lang.org...'
    $init = Get-Installer 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe' 'rustup-init.exe'
    # Per-user install: no administrator token involved.
    & $init -y --default-toolchain stable-x86_64-pc-windows-msvc --profile minimal
}

# --------------------------------------------------------------------- node

$NodeExe = Join-Path $env:ProgramFiles 'nodejs\node.exe'

function Test-Node {
    Test-Path $NodeExe
}

function Install-Node {
    Invoke-Winget 'OpenJS.NodeJS.LTS'
    if (Test-Node) { return }

    Write-Warn 'Usando el instalador oficial de nodejs.org...'
    $index = Invoke-RestMethod -Uri 'https://nodejs.org/dist/index.json' -UseBasicParsing -ErrorAction Stop
    $version = ($index | Where-Object { $_.lts } | Select-Object -First 1).version
    $msi = Get-Installer "https://nodejs.org/dist/$version/node-$version-x64.msi" "node-$version-x64.msi"
    Assert-Signed $msi
    Start-Elevated 'msiexec.exe' "/i `"$msi`" /qn /norestart"
}

# ---------------------------------------------------------------------------

$steps = @{
    vctools = @{ Name = 'C++ Build Tools'; Test = { Test-VcTools }; Install = { Install-VcTools } }
    rust    = @{ Name = 'Rust'; Test = { Test-Rust }; Install = { Install-Rust } }
    node    = @{ Name = 'Node.js'; Test = { Test-Node }; Install = { Install-Node } }
}
$step = $steps[$Component]

try {
    if (-not (& $step.Test)) { & $step.Install }
} catch {
    Write-Warn $_.Exception.Message
}
if (& $step.Test) {
    Write-Info "$($step.Name) instalado."
    exit 0
}
Write-Host "[ERROR] No se pudo instalar $($step.Name)."
exit 1
