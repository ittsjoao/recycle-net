# Builds target\release\rnetrbin-svc.exe and compiles the Inno Setup installer into
# target\installer\rnetrbin-setup-<version>.exe. Internal use only (embeds netrbin.sys).
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\release\rnetrbin-svc.exe'

# A running window or service keeps the exe locked and cargo would fail late.
if (Test-Path $exe) {
    try {
        [System.IO.File]::Open($exe, 'Open', 'ReadWrite', 'None').Close()
    } catch {
        throw "$exe esta em uso (janela ou servico rodando); feche/pare e rode de novo."
    }
}

$iscc = @(
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
    (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
    throw 'ISCC.exe (Inno Setup 6) nao encontrado em %LOCALAPPDATA%\Programs nem em Program Files (x86).'
}

$manifest = Get-Content (Join-Path $root 'crates\svc\Cargo.toml')
$match = $manifest | Select-String -Pattern '^version = "([^"]+)"' | Select-Object -First 1
if (-not $match) { throw 'versao nao encontrada em crates\svc\Cargo.toml' }
$version = $match.Matches[0].Groups[1].Value

Push-Location $root
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build --release falhou ($LASTEXITCODE)" }
    & $iscc /Q "/DAppVersion=$version" (Join-Path $PSScriptRoot 'rnetrbin.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC falhou ($LASTEXITCODE)" }
} finally {
    Pop-Location
}

$out = Join-Path $root "target\installer\rnetrbin-setup-$version.exe"
Write-Host "Instalador: $out ($([math]::Round((Get-Item $out).Length / 1MB, 1)) MB)"
