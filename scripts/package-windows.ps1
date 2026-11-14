[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Package the Windows build on Windows.' }
Set-Location (Split-Path $PSScriptRoot -Parent)
$env:PYTHONUTF8 = '1'
cargo build --locked --release --target x86_64-pc-windows-msvc --features windows-launcher
if ($LASTEXITCODE) { throw 'Windows build failed.' }
$Metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE) { throw 'Cargo metadata failed.' }
$PackageVersion = $Metadata.packages[0].version
$TargetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
$Stage = Join-Path $TargetRoot ('windows-package-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Stage -Force | Out-Null
try {
    foreach ($Binary in @('su03t-update.exe', 'su03t-tools.exe')) {
        Copy-Item (Join-Path $TargetRoot "x86_64-pc-windows-msvc/release/$Binary") $Stage
    }
    Copy-Item README.md, README.en.md, LICENSE $Stage
    New-Item -ItemType Directory -Path "$Stage/.harness/docs", "$Stage/docs" -Force | Out-Null
    Copy-Item .harness/docs/protocol.md, .harness/docs/validation.md "$Stage/.harness/docs"
    Copy-Item docs/images "$Stage/docs" -Recurse
    python3 scripts/collect-licenses.py --target x86_64-pc-windows-msvc --output $Stage
    if ($LASTEXITCODE) { throw 'License collection failed.' }
    python3 scripts/release_assets.py windows-archive --directory $Stage
    if ($LASTEXITCODE) { throw 'Windows archive creation failed.' }
    Write-Output "Packaged SU-03T Tools $PackageVersion for Windows x64."
} finally {
    Remove-Item -LiteralPath $Stage -Recurse -Force
}
