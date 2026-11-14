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
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rectangle);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
}
'@
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
            Start-Sleep -Seconds 5
            $Process.Refresh()
            if ($Process.HasExited -or !$Process.Responding -or $Process.MainWindowTitle -notlike 'SU-03T Tools*') { throw 'Native GUI did not remain responsive.' }
            [NativeSmokeWindow]::SetForegroundWindow($Process.MainWindowHandle) | Out-Null
            Start-Sleep -Milliseconds 700
            $Rectangle = New-Object NativeSmokeWindow+Rect
            if (![NativeSmokeWindow]::GetWindowRect($Process.MainWindowHandle, [ref]$Rectangle)) { throw 'Could not read application window bounds.' }
            $Image = New-Object Drawing.Bitmap ($Rectangle.Right - $Rectangle.Left), ($Rectangle.Bottom - $Rectangle.Top)
            $Graphics = [Drawing.Graphics]::FromImage($Image)
            try {
                $Graphics.CopyFromScreen($Rectangle.Left, $Rectangle.Top, 0, 0, $Image.Size)
                $Image.Save("$Output/windows-$Language.png", [Drawing.Imaging.ImageFormat]::Png)
            } finally { $Graphics.Dispose(); $Image.Dispose() }
            if (!$Process.CloseMainWindow() -or !$Process.WaitForExit(15000) -or $Process.ExitCode -ne 0) { throw 'Application did not close normally.' }
            Write-Output "PASS: Windows $Language desktop launch, responsive native window, screenshot and normal close"
        } finally {
            if ($Process -and !$Process.HasExited) { $Process.Kill(); $Process.WaitForExit() }
            if ($Launcher -and !$Launcher.HasExited) { $Launcher.Kill(); $Launcher.WaitForExit() }
        }
    }
} finally {
    $env:APPDATA = $OriginalAppData
    Remove-Item -LiteralPath $Temporary -Recurse -Force
}
