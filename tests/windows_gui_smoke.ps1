[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$Directory, [string]$Output = '.harness/local-evidence/windows-ui')
$ErrorActionPreference = 'Stop'
$Directory = [IO.Path]::GetFullPath($Directory)
$Output = [IO.Path]::GetFullPath($Output)
New-Item -ItemType Directory -Path $Output -Force | Out-Null
Add-Type -AssemblyName System.Drawing
Add-Type @'
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
}
'@
$DpiContext = [NativeSmokeWindow]::SetThreadDpiAwarenessContext([IntPtr](-4))
$OriginalAppData = $env:APPDATA
$Temporary = Join-Path ([IO.Path]::GetTempPath()) ('su03t-gui-' + [guid]::NewGuid().ToString('N'))
$env:APPDATA = Join-Path $Temporary 'config'
$LanguageFile = Join-Path $env:APPDATA 'SU03T Tools/language.json'
New-Item -ItemType Directory -Path (Split-Path $LanguageFile -Parent) -Force | Out-Null
$Cli = Join-Path $Directory 'su03t-update.exe'
if (@(Get-Process -Name su03t-update -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Cli }).Count) {
    throw 'An existing application must be preserved; run this check in an isolated environment.'
}
try {
    foreach ($Language in @('zh', 'en')) {
        [IO.File]::WriteAllText($LanguageFile, ('"' + $Language + '"'), (New-Object Text.UTF8Encoding $false))
        $Process = $null
        $Launcher = $null
        try {
            $Launcher = Start-Process (Join-Path $Directory 'su03t-tools.exe') -PassThru -RedirectStandardError "$Output/$Language.stderr.log" -RedirectStandardOutput "$Output/$Language.stdout.log"
            if (!$Launcher.WaitForExit(15000) -or $Launcher.ExitCode -ne 0) { throw 'Desktop launcher failed.' }
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
            Write-Output "PASS: Windows $Language desktop launch, responsive native window, screenshot and normal close"
        } finally {
            if ($Process -and !$Process.HasExited) { $Process.Kill(); $Process.WaitForExit() }
            if ($Launcher -and !$Launcher.HasExited) { $Launcher.Kill(); $Launcher.WaitForExit() }
        }
    }
} finally {
    $env:APPDATA = $OriginalAppData
    if ($DpiContext -ne [IntPtr]::Zero) { [NativeSmokeWindow]::SetThreadDpiAwarenessContext($DpiContext) | Out-Null }
    Remove-Item -LiteralPath $Temporary -Recurse -Force
}
