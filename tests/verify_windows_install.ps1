[CmdletBinding()]
param([string]$Version = 'latest')
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^latest$|^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') { throw 'Expected latest or vX.Y.Z.' }
$Endpoint = if ($Version -eq 'latest') { 'latest' } else { 'tags/' + $Version }
$Release = gh api "repos/MatriQ/su03t-tools/releases/$Endpoint" | ConvertFrom-Json
if ($LASTEXITCODE -or $Release.draft -or $Release.prerelease) { throw 'Expected a public stable Release.' }
$Tag = $Release.tag_name
$Download = Join-Path $env:RUNNER_TEMP 'public-installer'
New-Item -ItemType Directory -Path $Download -Force | Out-Null
gh release download $Tag --repo MatriQ/su03t-tools --pattern install.ps1 --dir $Download
if ($LASTEXITCODE) { throw 'Public installer download failed.' }
$Installer = Join-Path $Download 'install.ps1'
$Asset = @($Release.assets | Where-Object { $_.name -eq 'install.ps1' })
if ($Asset.Count -ne 1 -or $Asset[0].digest -ne ('sha256:' + (Get-FileHash $Installer -Algorithm SHA256).Hash.ToLowerInvariant())) { throw 'Installer digest mismatch.' }
$Prefix = Join-Path $env:RUNNER_TEMP ('public install ' + [char]0x4E2D + [char]0x6587)
try {
  & $Installer -Version $Tag -Prefix $Prefix -NoShortcut
  $Cli = Join-Path $Prefix 'app/su03t-update.exe'
  if ((& $Cli --version) -ne "su03t-update $($Tag.TrimStart('v'))") { throw 'Public installed version mismatch.' }
  python3 tests/cli_smoke.py $Cli
  if ($LASTEXITCODE) { throw 'Public installed CLI checks failed.' }
} finally {
  if (Test-Path (Join-Path $Prefix 'install.json')) { & $Installer -Prefix $Prefix -NoShortcut -Uninstall }
}
if (Test-Path (Join-Path $Prefix 'app')) { throw 'Uninstall left application files behind.' }
Write-Output "PASS: public $Tag Windows download, installer SHA-256, installation, Unicode path, CLI and uninstall"
if ([version]($Tag.TrimStart('v')) -ge [version]'0.6.1') {
  $Name = "SU03T-Tools-$Tag-Windows-x86_64-portable.zip"
  gh release download $Tag --repo MatriQ/su03t-tools --pattern $Name --dir $Download
  if ($LASTEXITCODE) { throw 'Public portable download failed.' }
  $Archive = Join-Path $Download $Name
  $Asset = @($Release.assets | Where-Object { $_.name -eq $Name })
  if ($Asset.Count -ne 1 -or $Asset[0].digest -ne ('sha256:' + (Get-FileHash $Archive -Algorithm SHA256).Hash.ToLowerInvariant())) { throw 'Portable archive digest mismatch.' }
  $Portable = Join-Path $Download 'portable'
  Expand-Archive -LiteralPath $Archive -DestinationPath $Portable
  if ((& (Join-Path $Portable 'su03t-update.exe') --version) -ne "su03t-update $($Tag.TrimStart('v'))") { throw 'Portable version mismatch.' }
  ./tests/windows_gui_smoke.ps1 -Directory $Portable -Portable -Output .harness/local-evidence/public-windows-ui
  Write-Output "PASS: public $Tag portable archive, SHA-256, native GUI, local settings and relocation"
}
