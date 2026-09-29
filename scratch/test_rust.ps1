param([Parameter(ValueFromRemainingArguments = $true)][string[]]$TestArguments)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    $buildOutput = & cargo test --manifest-path src-tauri/Cargo.toml --lib --offline --no-run --message-format=json
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $testExecutable = $buildOutput | ForEach-Object {
        try { $record = $_ | ConvertFrom-Json } catch { return }
        if ($record.reason -eq 'compiler-artifact' -and $record.profile.test -and $record.executable) { $record.executable }
    } | Select-Object -Last 1
    if (-not $testExecutable) { throw 'Cargo no devolvió el ejecutable de pruebas.' }
    # Tauri attaches its application manifest to binaries, not the library test
    # harness. Without Common Controls v6 the harness cannot import TaskDialogIndirect.
    $manifestTool = Get-Command mt.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -First 1
    if (-not $manifestTool) {
        $sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
        $manifestTool = Get-ChildItem -LiteralPath $sdkRoot -Directory |
            Sort-Object Name -Descending |
            ForEach-Object { Join-Path $_.FullName 'x64/mt.exe' } |
            Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    }
    if (-not $manifestTool) { throw 'No se encuentra mt.exe del Windows SDK.' }
    & $manifestTool -nologo -manifest (Join-Path $PSScriptRoot 'tests.manifest') "-outputresource:$testExecutable;#1"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $testExecutable @TestArguments
    exit $LASTEXITCODE
} finally { Pop-Location }
