[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$Directory, [string]$Output = '.harness/local-evidence/windows-ui', [switch]$ResizeDisplay)
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
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] public struct DisplayMode {
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string DeviceName;
        public ushort SpecVersion, DriverVersion, Size, DriverExtra;
        public uint Fields;
        public int PositionX, PositionY;
        public uint Orientation, FixedOutput;
        public short Color, Duplex, YResolution, TTOption, Collate;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string FormName;
        public ushort LogPixels;
        public uint BitsPerPixel, Width, Height, DisplayFlags, Frequency;
        public uint IcmMethod, IcmIntent, MediaType, DitherType, Reserved1, Reserved2, PanningWidth, PanningHeight;
    }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rectangle);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern bool EnumDisplaySettings(string device, int mode, ref DisplayMode data);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int ChangeDisplaySettings(ref DisplayMode data, uint flags);
}
'@
$OriginalDisplay = New-Object NativeSmokeWindow+DisplayMode
$OriginalDisplay.Size = [Runtime.InteropServices.Marshal]::SizeOf($OriginalDisplay)
if (![NativeSmokeWindow]::EnumDisplaySettings($null, -1, [ref]$OriginalDisplay)) { throw "Could not read display mode: size=$($OriginalDisplay.Size) error=$([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
$DisplayChanged = $false
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
    if ($ResizeDisplay) {
        $Selected = $null
        for ($Index = 0; $Index -lt 500; $Index++) {
            $Candidate = New-Object NativeSmokeWindow+DisplayMode
            $Candidate.Size = [Runtime.InteropServices.Marshal]::SizeOf($Candidate)
            if (![NativeSmokeWindow]::EnumDisplaySettings($null, $Index, [ref]$Candidate)) { break }
            if ($Candidate.Width -eq 1920 -and $Candidate.Height -eq 1080) { $Selected = $Candidate; break }
            if ($Candidate.Width -ge 1280 -and $Candidate.Height -ge 960 -and (!$Selected -or $Candidate.Width * $Candidate.Height -lt $Selected.Width * $Selected.Height)) { $Selected = $Candidate }
        }
        if (!$Selected) { throw 'No display mode large enough for full-window screenshots.' }
        $Selected.Fields = $Selected.Fields -bor 0x180000
        if ([NativeSmokeWindow]::ChangeDisplaySettings([ref]$Selected, 0) -ne 0) { throw 'Could not resize the isolated CI display.' }
        $DisplayChanged = $true
        Start-Sleep -Seconds 2
    }
    $CurrentDisplay = New-Object NativeSmokeWindow+DisplayMode
    $CurrentDisplay.Size = [Runtime.InteropServices.Marshal]::SizeOf($CurrentDisplay)
    [NativeSmokeWindow]::EnumDisplaySettings($null, -1, [ref]$CurrentDisplay) | Out-Null
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
            [NativeSmokeWindow]::SetForegroundWindow($Process.MainWindowHandle) | Out-Null
            $Rectangle = New-Object NativeSmokeWindow+Rect
            if (![NativeSmokeWindow]::GetWindowRect($Process.MainWindowHandle, [ref]$Rectangle)) { throw 'Could not read application window bounds.' }
            $Width = $Rectangle.Right - $Rectangle.Left
            $Height = $Rectangle.Bottom - $Rectangle.Top
            if ($Width -gt $CurrentDisplay.Width -or $Height -gt $CurrentDisplay.Height) { throw 'Display is too small to capture the whole window; use -ResizeDisplay in isolated CI.' }
            $X = [int](($CurrentDisplay.Width - $Width) / 2)
            $Y = [int](($CurrentDisplay.Height - $Height) / 2)
            [NativeSmokeWindow]::SetWindowPos($Process.MainWindowHandle, [IntPtr]::Zero, $X, $Y, 0, 0, 5) | Out-Null
            Start-Sleep -Milliseconds 700
            [NativeSmokeWindow]::GetWindowRect($Process.MainWindowHandle, [ref]$Rectangle) | Out-Null
            $Image = New-Object Drawing.Bitmap ($Rectangle.Right - $Rectangle.Left), ($Rectangle.Bottom - $Rectangle.Top)
            $Graphics = [Drawing.Graphics]::FromImage($Image)
            try {
                $Graphics.CopyFromScreen($Rectangle.Left, $Rectangle.Top, 0, 0, $Image.Size)
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
    if ($DisplayChanged) { [NativeSmokeWindow]::ChangeDisplaySettings([ref]$OriginalDisplay, 0) | Out-Null }
    if ($DpiContext -ne [IntPtr]::Zero) { [NativeSmokeWindow]::SetThreadDpiAwarenessContext($DpiContext) | Out-Null }
    Remove-Item -LiteralPath $Temporary -Recurse -Force
}
