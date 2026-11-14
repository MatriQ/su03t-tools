# User-scoped Windows installation; all downloaded bytes are checked before replacement.
[CmdletBinding()]
param(
    [string]$Version = 'latest',
    [string]$Prefix = '',
    [switch]$NoShortcut,
    [switch]$Uninstall
)

function Get-Su03tRelease([string]$Tag) {
    $Api = 'https://api.github.com/repos/MatriQ/su03t-tools/releases/'
    if ($Tag -eq 'latest') { $Api += 'latest' } else { $Api += "tags/$Tag" }
    Invoke-RestMethod -Uri $Api -Headers @{ 'User-Agent' = 'su03t-tools-installer' }
}

function Save-Su03tAsset([string]$Tag, [string]$Name, [string]$Destination) {
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/MatriQ/su03t-tools/releases/download/$Tag/$Name" -OutFile $Destination
}

function Expand-Su03tArchive([string]$Archive, [string]$Destination) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $Zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $Seen = @{}
        $Total = [long]0
        foreach ($Entry in $Zip.Entries) {
            $Name = $Entry.FullName.Replace('\', '/')
            if ($Name -match '^/|:|(^|/)\.\.?(/|$)' -or $Name -match '[\x00-\x1f]') {
                throw 'Unsafe archive path.'
            }
            if (($Entry.ExternalAttributes -shr 16 -band 0xF000) -eq 0xA000) { throw 'Archive links are not supported.' }
            $Key = $Name.TrimEnd('/').ToLowerInvariant()
            if (!$Key -or $Seen.ContainsKey($Key)) { throw 'Duplicate or empty archive path.' }
            $Seen[$Key] = $true
            $Total += $Entry.Length
            if ($Seen.Count -gt 5000 -or $Total -gt 536870912) { throw 'Archive exceeds installation limits.' }
            $Target = [IO.Path]::GetFullPath((Join-Path $Destination $Name))
            $Root = [IO.Path]::GetFullPath($Destination).TrimEnd('\') + '\'
            if (!$Target.StartsWith($Root, [StringComparison]::OrdinalIgnoreCase)) { throw 'Archive path leaves installation directory.' }
            if ($Name.EndsWith('/')) { New-Item -ItemType Directory -Path $Target -Force | Out-Null; continue }
            New-Item -ItemType Directory -Path (Split-Path $Target -Parent) -Force | Out-Null
            [IO.Compression.ZipFileExtensions]::ExtractToFile($Entry, $Target)
        }
    } finally { $Zip.Dispose() }
    foreach ($Required in @('su03t-update.exe', 'su03t-tools.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.txt')) {
        if (!(Test-Path -LiteralPath (Join-Path $Destination $Required) -PathType Leaf)) { throw "Missing package file: $Required" }
    }
}

function Install-Su03tTools {
    [CmdletBinding()]
    param([string]$Version = 'latest', [string]$Prefix = '', [switch]$NoShortcut, [switch]$Uninstall)
    $ErrorActionPreference = 'Stop'
    if ($env:OS -ne 'Windows_NT' -or ![Environment]::Is64BitOperatingSystem) { throw 'Windows x64 is required.' }
    if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() -ne 'X64') { throw 'This package supports Windows x64.' }
    if (!$Prefix) { $Prefix = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs/SU03T Tools' }
    $InstallRoot = [IO.Path]::GetFullPath($Prefix).TrimEnd('\')
    if ($InstallRoot -eq [IO.Path]::GetPathRoot($InstallRoot).TrimEnd('\')) { throw 'Choose an application directory, not a drive root.' }
    $Marker = Join-Path $InstallRoot 'install.json'
    $App = Join-Path $InstallRoot 'app'
    foreach ($ManagedPath in @($InstallRoot, $Marker, $App)) {
        if ((Test-Path -LiteralPath $ManagedPath) -and ((Get-Item -LiteralPath $ManagedPath -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Refusing to change a linked installation path.'
        }
    }
    $Existing = $null
    if (Test-Path -LiteralPath $Marker) {
        $Existing = Get-Content -LiteralPath $Marker -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($Existing.repository -ne 'MatriQ/su03t-tools') { throw 'Installation directory belongs to another application.' }
    } elseif ((Test-Path -LiteralPath $InstallRoot) -and (Get-ChildItem -LiteralPath $InstallRoot -Force | Select-Object -First 1)) {
        throw 'Refusing to change an unrecognized installation directory.'
    }
    $ShortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'SU-03T Tools.lnk'
    if ($Uninstall) {
        if (!$Existing) { throw 'No managed installation found.' }
        if (Test-Path -LiteralPath $App) { Remove-Item -LiteralPath $App -Recurse -Force }
        Remove-Item -LiteralPath $Marker
        if (Test-Path -LiteralPath $ShortcutPath) {
            $Shell = New-Object -ComObject WScript.Shell
            if ($Shell.CreateShortcut($ShortcutPath).TargetPath -eq (Join-Path $App 'su03t-tools.exe')) {
                Remove-Item -LiteralPath $ShortcutPath
            }
        }
        Write-Output 'Uninstalled SU-03T Tools; user preferences were retained.'
        return
    }
    if ($Version -ne 'latest') {
        if ($Version -notmatch '^v?(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') { throw 'Version must be latest or vX.Y.Z.' }
        $Version = 'v' + $Version.TrimStart('v')
    }
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $Release = Get-Su03tRelease $Version
    $Tag = [string]$Release.tag_name
    if ($Release.draft -or $Release.prerelease -or $Tag -notmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') { throw 'Expected a published stable release.' }
    if ($Version -ne 'latest' -and $Tag -ne $Version) { throw 'Release version mismatch.' }
    $Names = @("SU03T-Tools-$Tag-Windows-x86_64.zip", "SU03T-Tools-$Tag-SHA256SUMS.txt")
    $Assets = @{}
    foreach ($Name in $Names) {
        $MatchesFound = @($Release.assets | Where-Object { $_.name -eq $Name })
        if ($MatchesFound.Count -ne 1 -or $MatchesFound[0].digest -notmatch '^sha256:[a-f0-9]{64}$') { throw "Missing or invalid release asset: $Name" }
        $Assets[$Name] = $MatchesFound[0]
    }
    New-Item -ItemType Directory -Path $InstallRoot -Force | Out-Null
    $Stage = Join-Path $InstallRoot ('.stage-' + [guid]::NewGuid().ToString('N'))
    $Backup = Join-Path $InstallRoot ('.backup-' + [guid]::NewGuid().ToString('N'))
    $MovedOld = $false
    $MovedNew = $false
    New-Item -ItemType Directory -Path $Stage | Out-Null
    try {
        foreach ($Name in $Names) {
            $File = Join-Path $Stage $Name
            Save-Su03tAsset $Tag $Name $File
            $Actual = (Get-FileHash -LiteralPath $File -Algorithm SHA256).Hash.ToLowerInvariant()
            if ("sha256:$Actual" -ne $Assets[$Name].digest -or (Get-Item -LiteralPath $File).Length -ne $Assets[$Name].size) { throw 'Release checksum or size mismatch.' }
        }
        $Checks = @{}
        foreach ($Line in [IO.File]::ReadAllLines((Join-Path $Stage $Names[1]))) {
            if ($Line -notmatch '^([a-f0-9]{64})  ([A-Za-z0-9][A-Za-z0-9._-]*)$') { throw 'Invalid checksum manifest.' }
            if ($Checks.ContainsKey($Matches[2])) { throw 'Duplicate checksum manifest entry.' }
            $Checks[$Matches[2]] = $Matches[1]
        }
        if ("sha256:$($Checks[$Names[0]])" -ne $Assets[$Names[0]].digest) { throw 'Archive and manifest disagree.' }
        $StagedApp = Join-Path $Stage 'app'
        New-Item -ItemType Directory -Path $StagedApp | Out-Null
        Expand-Su03tArchive (Join-Path $Stage $Names[0]) $StagedApp
        $Reported = & (Join-Path $StagedApp 'su03t-update.exe') --version
        if ($LASTEXITCODE -ne 0 -or $Reported -ne "su03t-update $($Tag.TrimStart('v'))") { throw 'Packaged executable version check failed.' }
        foreach ($Exe in @('su03t-update.exe', 'su03t-tools.exe')) {
            Set-Content -LiteralPath (Join-Path $StagedApp $Exe) -Stream Zone.Identifier -Value "[ZoneTransfer]`r`nZoneId=3`r`nHostUrl=https://github.com/MatriQ/su03t-tools/releases/tag/$Tag" -Encoding ASCII
        }
        if (Test-Path -LiteralPath $App) { Move-Item -LiteralPath $App -Destination $Backup; $MovedOld = $true }
        Move-Item -LiteralPath $StagedApp -Destination $App
        $MovedNew = $true
        $Json = @{ repository = 'MatriQ/su03t-tools'; version = $Tag } | ConvertTo-Json
        $NewMarker = Join-Path $Stage 'install.json'
        [IO.File]::WriteAllText($NewMarker, $Json, (New-Object Text.UTF8Encoding $false))
        if (Test-Path -LiteralPath $Marker) { [IO.File]::Replace($NewMarker, $Marker, $null) }
        else { [IO.File]::Move($NewMarker, $Marker) }
    } catch {
        if ($MovedNew -and (Test-Path -LiteralPath $App)) { Remove-Item -LiteralPath $App -Recurse -Force }
        if ($MovedOld) { Move-Item -LiteralPath $Backup -Destination $App }
        throw
    } finally {
        if (Test-Path -LiteralPath $Stage) { Remove-Item -LiteralPath $Stage -Recurse -Force }
    }
    if (Test-Path -LiteralPath $Backup) {
        try { Remove-Item -LiteralPath $Backup -Recurse -Force }
        catch { Write-Warning "The old app is still in use; its backup remains at $Backup" }
    }
    if (!$NoShortcut) {
        try {
            $Shell = New-Object -ComObject WScript.Shell
            $Shortcut = $Shell.CreateShortcut($ShortcutPath)
            if ((Test-Path -LiteralPath $ShortcutPath) -and $Shortcut.TargetPath -ne (Join-Path $App 'su03t-tools.exe')) { throw 'An unrelated Start menu shortcut already exists.' }
            $Shortcut.TargetPath = Join-Path $App 'su03t-tools.exe'
            $Shortcut.WorkingDirectory = $App
            $Shortcut.Save()
        } catch { Write-Warning "Installed without Start menu shortcut: $_" }
    }
    Write-Output "Installed SU-03T Tools $Tag in $App"
    Write-Output ('Desktop: & "' + (Join-Path $App 'su03t-tools.exe') + '"')
    Write-Output ('CLI: & "' + (Join-Path $App 'su03t-update.exe') + '" --help')
}

if ($MyInvocation.InvocationName -ne '.') { Install-Su03tTools @PSBoundParameters }
