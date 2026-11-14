[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$BinaryDirectory)
$ErrorActionPreference = 'Stop'
$BinaryDirectory = [IO.Path]::GetFullPath($BinaryDirectory)
. (Join-Path (Split-Path $PSScriptRoot -Parent) 'install.ps1')
$TestRoot = Join-Path ([IO.Path]::GetTempPath()) ('su03t-installer-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $TestRoot | Out-Null
$Fixture = Join-Path $TestRoot 'fixture'
$Downloads = Join-Path $TestRoot 'downloads'
New-Item -ItemType Directory -Path $Fixture, $Downloads | Out-Null
Copy-Item (Join-Path $BinaryDirectory 'su03t-update.exe'), (Join-Path $BinaryDirectory 'su03t-tools.exe') $Fixture
$Tag = 'v' + ((& (Join-Path $Fixture 'su03t-update.exe') --version) -replace '^su03t-update ', '')
[IO.File]::WriteAllText((Join-Path $Fixture 'LICENSE'), 'test license')
[IO.File]::WriteAllText((Join-Path $Fixture 'THIRD_PARTY_NOTICES.txt'), 'test notices')
Add-Type -AssemblyName System.IO.Compression.FileSystem
$ArchiveName = "SU03T-Tools-$Tag-Windows-x86_64.zip"
$ManifestName = "SU03T-Tools-$Tag-SHA256SUMS.txt"
$Archive = Join-Path $Downloads $ArchiveName
$script:FixtureRelease = $null
$script:Passed = 0

function Update-Fixture {
    $ArchiveHash = (Get-FileHash $Archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $Downloads $ManifestName), "$ArchiveHash  $ArchiveName`n", (New-Object Text.UTF8Encoding $false))
    $Assets = @($ArchiveName, $ManifestName) | ForEach-Object {
        $File = Join-Path $Downloads $_
        @{ name = $_; size = (Get-Item $File).Length; digest = 'sha256:' + (Get-FileHash $File -Algorithm SHA256).Hash.ToLowerInvariant() }
    }
    $script:FixtureRelease = @{ tag_name = $Tag; draft = $false; prerelease = $false; assets = @($Assets) }
}
function New-Fixture {
    if (Test-Path $Archive) { Remove-Item -LiteralPath $Archive }
    [IO.Compression.ZipFile]::CreateFromDirectory($Fixture, $Archive, [IO.Compression.CompressionLevel]::Fastest, $false)
    Update-Fixture
}
function Get-Su03tRelease([string]$RequestedTag) { return $script:FixtureRelease }
function Save-Su03tAsset([string]$RequestedTag, [string]$Name, [string]$Destination) {
    Copy-Item -LiteralPath (Join-Path $Downloads $Name) -Destination $Destination
    if ($script:LockTarget -and $Name -eq $ManifestName) {
        $script:LockedFile = [IO.File]::Open($script:LockTarget, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    }
}
function Assert-True($Condition, [string]$Message) {
    if (!$Condition) { throw $Message }
}
function Expect-Failure([scriptblock]$Action, [string]$Pattern) {
    $Failure = $null
    try { & $Action } catch { $Failure = $_.Exception.Message }
    Assert-True ($Failure -and $Failure -match $Pattern) "Expected failure matching $Pattern; got $Failure"
    $script:Passed++
}

try {
    New-Fixture
    $Unicode = [string][char]0x4E2D + [char]0x6587
    $InstallPrefix = Join-Path $TestRoot "$Unicode installation"
    Install-Su03tTools -Prefix $InstallPrefix -NoShortcut
    $InstalledCli = Join-Path $InstallPrefix 'app/su03t-update.exe'
    $InstalledHash = (Get-FileHash $InstalledCli -Algorithm SHA256).Hash
    Assert-True (Test-Path (Join-Path $InstallPrefix 'install.json')) 'No ownership marker.'
    Assert-True ((Get-Content -LiteralPath $InstalledCli -Stream Zone.Identifier -Raw) -match 'ZoneId=3') 'Download protection was not retained.'
    Assert-True ((& $InstalledCli --version) -eq "su03t-update $($Tag.TrimStart('v'))") 'Installed version mismatch.'
    $script:Passed++
    Install-Su03tTools -Prefix $InstallPrefix -NoShortcut
    Assert-True ((Get-FileHash $InstalledCli -Algorithm SHA256).Hash -eq $InstalledHash) 'Reinstall changed binary bytes.'
    $script:Passed++

    $script:FixtureRelease.assets[0].digest = 'sha256:' + ('0' * 64)
    Expect-Failure { Install-Su03tTools -Prefix $InstallPrefix -NoShortcut } 'checksum'
    Assert-True ((Get-FileHash $InstalledCli -Algorithm SHA256).Hash -eq $InstalledHash) 'Checksum refusal damaged existing installation.'
    Update-Fixture

    $Foreign = Join-Path $TestRoot 'foreign'
    New-Item -ItemType Directory $Foreign | Out-Null
    [IO.File]::WriteAllText((Join-Path $Foreign 'keep.txt'), 'keep')
    Expect-Failure { Install-Su03tTools -Prefix $Foreign -NoShortcut } 'unrecognized'
    Assert-True ((Get-Content (Join-Path $Foreign 'keep.txt')) -eq 'keep') 'Foreign file changed.'
    Expect-Failure { Install-Su03tTools -Prefix $InstallPrefix -Version '../bad' -NoShortcut } 'Version must'
    $script:FixtureRelease.assets = @($script:FixtureRelease.assets[0])
    Expect-Failure { Install-Su03tTools -Prefix $InstallPrefix -NoShortcut } 'Missing or invalid'
    Update-Fixture

    $Zip = [IO.Compression.ZipFile]::Open($Archive, [IO.Compression.ZipArchiveMode]::Update)
    $Entry = $Zip.CreateEntry('../escaped.txt')
    $Writer = New-Object IO.StreamWriter ($Entry.Open())
    $Writer.Write('escape'); $Writer.Dispose(); $Zip.Dispose()
    Update-Fixture
    Expect-Failure { Install-Su03tTools -Prefix $InstallPrefix -NoShortcut } 'Unsafe archive path'
    Assert-True (!(Test-Path (Join-Path $InstallPrefix 'escaped.txt'))) 'Archive escaped destination.'
    New-Fixture

    $Zip = [IO.Compression.ZipFile]::Open($Archive, [IO.Compression.ZipArchiveMode]::Update)
    $Entry = $Zip.CreateEntry('SU03T-UPDATE.EXE')
    $Writer = New-Object IO.StreamWriter ($Entry.Open())
    $Writer.Write('duplicate'); $Writer.Dispose(); $Zip.Dispose()
    Update-Fixture
    Expect-Failure { Install-Su03tTools -Prefix $InstallPrefix -NoShortcut } 'Duplicate'
    New-Fixture

    $Zip = [IO.Compression.ZipFile]::Open($Archive, [IO.Compression.ZipArchiveMode]::Update)
    $Zip.GetEntry('su03t-tools.exe').Delete(); $Zip.Dispose()
    Update-Fixture
    Expect-Failure { Install-Su03tTools -Prefix $InstallPrefix -NoShortcut } 'Missing package file'
    Assert-True ((Get-FileHash $InstalledCli -Algorithm SHA256).Hash -eq $InstalledHash) 'Invalid package damaged existing installation.'
    New-Fixture

    $Sentinel = Join-Path $InstallPrefix 'app/old-version.txt'
    [IO.File]::WriteAllText($Sentinel, 'old installation')
    $script:LockTarget = Join-Path $InstallPrefix 'install.json'
    try { Expect-Failure { Install-Su03tTools -Prefix $InstallPrefix -NoShortcut } 'used by another process|being used' }
    finally { $script:LockedFile.Dispose(); $script:LockTarget = $null }
    Assert-True (Test-Path $Sentinel) 'Replacement failure did not restore the old application.'
    Assert-True ((Get-FileHash $InstalledCli -Algorithm SHA256).Hash -eq $InstalledHash) 'Replacement failure changed the old executable.'

    $LockedMarker = [IO.File]::Open((Join-Path $InstallPrefix 'install.json'), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    try { Expect-Failure { Install-Su03tTools -Prefix $InstallPrefix -NoShortcut } 'used by another process|being used' }
    finally { $LockedMarker.Dispose() }
    Assert-True ((Get-FileHash $InstalledCli -Algorithm SHA256).Hash -eq $InstalledHash) 'Locked installation was damaged.'
    [IO.File]::WriteAllText((Join-Path $InstallPrefix 'keep.txt'), 'unrelated user file')
    Install-Su03tTools -Prefix $InstallPrefix -NoShortcut -Uninstall
    Assert-True (!(Test-Path $InstalledCli)) 'Uninstall left the command behind.'
    Assert-True (Test-Path (Join-Path $InstallPrefix 'keep.txt')) 'Uninstall removed unrelated user file.'
    $script:Passed++
    $ShortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'SU-03T Tools.lnk'
    if (!(Test-Path -LiteralPath $ShortcutPath)) {
        $ShortcutPrefix = Join-Path $TestRoot 'shortcut installation'
        Install-Su03tTools -Prefix $ShortcutPrefix
        $Shell = New-Object -ComObject WScript.Shell
        Assert-True ($Shell.CreateShortcut($ShortcutPath).TargetPath -eq (Join-Path $ShortcutPrefix 'app/su03t-tools.exe')) 'Start menu shortcut target mismatch.'
        Install-Su03tTools -Prefix $ShortcutPrefix -Uninstall
        Assert-True (!(Test-Path -LiteralPath $ShortcutPath)) 'Uninstall left the managed Start menu shortcut.'
        $script:Passed++
    }
    Write-Output "PASS: $script:Passed Windows installer scenarios (Unicode path, reinstall, checksum, ownership, archive safety, locked files and uninstall)"
} finally {
    Remove-Item -LiteralPath $TestRoot -Recurse -Force
}
