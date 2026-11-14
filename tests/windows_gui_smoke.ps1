[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$Directory, [string]$Output = '.harness/local-evidence/windows-ui', [switch]$Portable)
$ErrorActionPreference = 'Stop'
$Directory = [IO.Path]::GetFullPath($Directory)
$Output = [IO.Path]::GetFullPath($Output)
New-Item -ItemType Directory -Path $Output -Force | Out-Null
Add-Type -AssemblyName System.Drawing
if (!('NativeSmokeWindow' -as [type])) { Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class NativeSmokeWindow {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out Rect rectangle);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);
}
'@
}
$Cli = Join-Path $Directory 'su03t-update.exe'
if (@(Get-Process -Name su03t-update -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Cli }).Count) {
    throw 'An existing application must be preserved; run this check in an isolated environment.'
}
$DpiContext = [NativeSmokeWindow]::SetThreadDpiAwarenessContext([IntPtr](-4))
$OriginalAppData = $env:APPDATA
$Temporary = Join-Path ([IO.Path]::GetTempPath()) ('su03t-gui-' + [guid]::NewGuid().ToString('N'))
$env:APPDATA = Join-Path $Temporary 'config'
$ProfileLanguageFile = Join-Path $env:APPDATA 'SU03T Tools/language.json'
try {
    $LanguageFile = $ProfileLanguageFile
    if ($Portable) {
        if (!(Test-Path -LiteralPath (Join-Path $Directory 'portable.flag') -PathType Leaf)) { throw 'Portable archive is missing its marker.' }
        $Copy = Join-Path $Temporary ('portable ' + [char]0x4E2D + [char]0x6587)
        New-Item -ItemType Directory -Path $Copy -Force | Out-Null
        Get-ChildItem -LiteralPath $Directory -Force | Copy-Item -Destination $Copy -Recurse
        $Directory = $Copy
        $Cli = Join-Path $Directory 'su03t-update.exe'
        $LanguageFile = Join-Path $Directory 'data/language.json'
    } elseif (Test-Path -LiteralPath (Join-Path $Directory 'portable.flag')) { throw 'The installed package must not enable portable mode.' }
    New-Item -ItemType Directory -Path (Split-Path $LanguageFile -Parent), (Split-Path $ProfileLanguageFile -Parent) -Force | Out-Null
    foreach ($Language in @('zh', 'en')) {
        [IO.File]::WriteAllText($LanguageFile, ('"' + $Language + '"'), (New-Object Text.UTF8Encoding $false))
        if ($Portable) {
            $ProfileSeed = if ($Language -eq 'zh') { '"en"' } else { '"zh"' }
            [IO.File]::WriteAllText($ProfileLanguageFile, $ProfileSeed, (New-Object Text.UTF8Encoding $false))
        }
        $Process = $null
        $Launcher = $null
        try {
            $Launcher = Start-Process (Join-Path $Directory 'su03t-tools.exe') -PassThru -RedirectStandardError "$Output/$Language.stderr.log" -RedirectStandardOutput "$Output/$Language.stdout.log"
            # Windows PowerShell 5.1 can leave Start-Process.ExitCode unset.
            # Retain the handle and query the actual process exit status instead.
            $LauncherHandle = $Launcher.Handle
            $LauncherExited = $Launcher.WaitForExit(15000)
            $LauncherExitCode = [uint32]0
            $ReadExitCode = [NativeSmokeWindow]::GetExitCodeProcess($LauncherHandle, [ref]$LauncherExitCode)
            if (!$LauncherExited -or !$ReadExitCode -or $LauncherExitCode -ne 0) { throw "Desktop launcher failed: exited=$LauncherExited read=$ReadExitCode exit=$LauncherExitCode" }
            $Deadline = [DateTime]::UtcNow.AddSeconds(45)
            while ([DateTime]::UtcNow -lt $Deadline) {
                $Process = Get-Process -Name su03t-update -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Cli } | Select-Object -First 1
                if ($Process) { $Process.Refresh(); if ($Process.MainWindowHandle -ne [IntPtr]::Zero) { break } }
                Start-Sleep -Milliseconds 200
            }
            if (!$Process -or $Process.MainWindowHandle -eq [IntPtr]::Zero) {
                Get-Content "$Output/$Language.stderr.log" -ErrorAction SilentlyContinue
                throw 'No native application window appeared.'
            }
            # Open the process handle while it is alive; Get-Process alone cannot
            # reliably retrieve an exit code after a fast window close.
            $ProcessHandle = $Process.Handle
            Start-Sleep -Seconds 5
            $Process.Refresh()
            if ($Process.HasExited -or !$Process.Responding -or $Process.MainWindowTitle -notlike 'SU-03T Tools*') { throw 'Native GUI did not remain responsive.' }
            [NativeSmokeWindow]::ShowWindow($Process.MainWindowHandle, 3) | Out-Null
            [NativeSmokeWindow]::SetForegroundWindow($Process.MainWindowHandle) | Out-Null
            Start-Sleep -Milliseconds 900
            $Rectangle = New-Object NativeSmokeWindow+Rect
            $Origin = New-Object NativeSmokeWindow+Point
            if (![NativeSmokeWindow]::GetClientRect($Process.MainWindowHandle, [ref]$Rectangle) -or ![NativeSmokeWindow]::ClientToScreen($Process.MainWindowHandle, [ref]$Origin)) { throw 'Could not read the native client area.' }
            $Image = New-Object Drawing.Bitmap ($Rectangle.Right - $Rectangle.Left), ($Rectangle.Bottom - $Rectangle.Top)
            $Graphics = [Drawing.Graphics]::FromImage($Image)
            try {
                $Graphics.CopyFromScreen($Origin.X, $Origin.Y, 0, 0, $Image.Size)
                $Image.Save("$Output/windows-$Language.png", [Drawing.Imaging.ImageFormat]::Png)
            } finally { $Graphics.Dispose(); $Image.Dispose() }
            $CloseRequested = $Process.CloseMainWindow()
            $Exited = $Process.WaitForExit(15000)
            $ExitCode = $Process.ExitCode
            if (!$CloseRequested -or !$Exited -or $ExitCode -ne 0) { throw "Application did not close normally: requested=$CloseRequested exited=$Exited exit=$ExitCode handle=$ProcessHandle" }
            if ($Portable) {
                if ((Get-Content -LiteralPath $LanguageFile -Raw | ConvertFrom-Json) -ne $Language) { throw 'Portable language settings were lost.' }
                if ([IO.File]::ReadAllText($ProfileLanguageFile) -ne $ProfileSeed) { throw 'Portable mode changed user-profile settings.' }
            }
            Write-Output "PASS: Windows $Language desktop launch, responsive native window, screenshot and normal close"
        } finally {
            if ($Process -and !$Process.HasExited) { $Process.Kill(); $Process.WaitForExit() }
            if ($Launcher -and !$Launcher.HasExited) { $Launcher.Kill(); $Launcher.WaitForExit() }
            # A launcher check can fail before its child has been discovered.
            Get-Process -Name su03t-update -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Cli } | ForEach-Object { $_.Kill(); $_.WaitForExit() }
        }
        if ($Portable -and $Language -eq 'zh') {
            $Moved = Join-Path $Temporary ('moved ' + [char]0x79FB + [char]0x52A8 + ' portable')
            Move-Item -LiteralPath $Directory -Destination $Moved
            $Directory = $Moved
            $Cli = Join-Path $Directory 'su03t-update.exe'
            $LanguageFile = Join-Path $Directory 'data/language.json'
            if ((Get-Content -LiteralPath $LanguageFile -Raw | ConvertFrom-Json) -ne 'zh') { throw 'Moving the portable directory lost its settings.' }
            Write-Output 'PASS: portable settings survived relocation to a Unicode path'
        }
    }
    if ($Portable) { Write-Output 'PASS: portable desktop before/after relocation, local settings and unchanged user profile' }
} finally {
    $env:APPDATA = $OriginalAppData
    if ($DpiContext -ne [IntPtr]::Zero) { [NativeSmokeWindow]::SetThreadDpiAwarenessContext($DpiContext) | Out-Null }
    try { Remove-Item -LiteralPath $Temporary -Recurse -Force }
    catch { Write-Warning "Could not remove the temporary GUI fixture: $_" }
}
