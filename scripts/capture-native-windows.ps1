#Requires -Version 7.0
param([Parameter(Mandatory)][string] $OutputDirectory)

$ErrorActionPreference = 'Stop'
if (!$IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or !$env:RUNNER_TEMP) {
    throw 'Windows desktop captures require a disposable GitHub-hosted Windows runner.'
}
if (![Environment]::UserInteractive -or [Diagnostics.Process]::GetCurrentProcess().SessionId -eq 0) {
    throw 'The hosted runner has no interactive Windows desktop for GPUI capture.'
}

Add-Type -AssemblyName System.Drawing.Common
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class RefractCaptureWindow {
    [StructLayout(LayoutKind.Sequential)]
    public struct Point {
        public int X, Y;
        public Point(int x, int y) { X = x; Y = y; }
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    public struct MonitorInfo { public int Size; public Rect Bounds, WorkArea; public int Flags; }

    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr window, uint flags);
    [DllImport("user32.dll", EntryPoint = "GetMonitorInfoW")] public static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr order, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
}
'@

function Get-ClientRegion([IntPtr] $Window) {
    $rect = [RefractCaptureWindow+Rect]::new()
    $origin = [RefractCaptureWindow+Point]::new(0, 0)
    if (![RefractCaptureWindow]::GetClientRect($Window, [ref] $rect) -or
        ![RefractCaptureWindow]::ClientToScreen($Window, [ref] $origin)) {
        throw 'Could not find the Refract client region.'
    }
    return [Drawing.Rectangle]::new($origin.X, $origin.Y, $rect.Right, $rect.Bottom)
}

function Set-ClientSize([IntPtr] $Window, [int] $Width, [int] $Height) {
    $outer = [RefractCaptureWindow+Rect]::new()
    $monitor = [RefractCaptureWindow]::MonitorFromWindow($Window, 2)
    $info = [RefractCaptureWindow+MonitorInfo]::new()
    $info.Size = [Runtime.InteropServices.Marshal]::SizeOf([type] [RefractCaptureWindow+MonitorInfo])
    if (![RefractCaptureWindow]::GetMonitorInfo($monitor, [ref] $info)) {
        throw 'Could not find the Refract monitor.'
    }
    $work = $info.WorkArea
    for ($attempt = 0; $attempt -lt 3; $attempt++) {
        if (![RefractCaptureWindow]::GetWindowRect($Window, [ref] $outer)) {
            throw 'Could not find the Refract window bounds.'
        }
        $client = Get-ClientRegion $Window
        $frameWidth = $outer.Right - $outer.Left - $client.Width
        $frameHeight = $outer.Bottom - $outer.Top - $client.Height
        if ($Width + $frameWidth + 16 -gt $work.Right - $work.Left -or
            $Height + $frameHeight + 16 -gt $work.Bottom - $work.Top) {
            throw "The hosted desktop cannot fit an unobstructed ${Width}x${Height} Refract client area."
        }
        # Position both applications away from the taskbar and make only their fixture window topmost.
        if (![RefractCaptureWindow]::SetWindowPos($Window, [IntPtr] -1, ($work.Left + 8), ($work.Top + 8),
            ($Width + $frameWidth), ($Height + $frameHeight), 0x40)) {
            throw 'Could not position the Refract fixture window.'
        }
        $client = Get-ClientRegion $Window
        if ($client.Width -eq $Width -and $client.Height -eq $Height) {
            # Leave hover tooltips and caption button highlights out of the base frame.
            [RefractCaptureWindow]::SetForegroundWindow($Window) | Out-Null
            [RefractCaptureWindow]::SetCursorPos(($work.Right - 1), ($work.Bottom - 1)) | Out-Null
            return $client
        }
    }
    throw "Refract could not resize its client area to ${Width}x${Height}."
}

function Test-ClientVisible([IntPtr] $Window, [Drawing.Rectangle] $Client) {
    if ([RefractCaptureWindow]::GetForegroundWindow() -ne $Window) { return $false }
    # Measure inside the edges to account for rounded native frame corners.
    foreach ($fractionX in @(0.025, 0.25, 0.5, 0.75, 0.975)) {
        foreach ($fractionY in @(0.025, 0.5, 0.975)) {
            $point = [RefractCaptureWindow+Point]::new(($Client.Left + $Client.Width * $fractionX), ($Client.Top + $Client.Height * $fractionY))
            $hit = [RefractCaptureWindow]::WindowFromPoint($point)
            if ([RefractCaptureWindow]::GetAncestor($hit, 2) -ne $Window) { return $false }
        }
    }
    return $true
}

function Measure-Ink([Drawing.Bitmap] $Bitmap, [Drawing.Rectangle] $Region, [bool] $Dark) {
    $count = 0
    $left = $Region.Right
    $right = $Region.Left
    $top = $Region.Bottom
    $bottom = $Region.Top
    for ($y = $Region.Top; $y -lt $Region.Bottom; $y++) {
        for ($x = $Region.Left; $x -lt $Region.Right; $x++) {
            $color = $Bitmap.GetPixel($x, $y)
            $luma = (54 * $color.R + 183 * $color.G + 19 * $color.B) / 256
            if (($Dark -and $luma -gt 125) -or (!$Dark -and $luma -lt 135)) {
                $count++
                $left = [Math]::Min($left, $x)
                $right = [Math]::Max($right, $x)
                $top = [Math]::Min($top, $y)
                $bottom = [Math]::Max($bottom, $y)
            }
        }
    }
    return @{ pixels = $count; left = $left; top = $top; width = $right - $left + 1; height = $bottom - $top + 1 }
}

function Measure-Library([Drawing.Bitmap] $Bitmap, [double] $Scale, [bool] $Dark) {
    $background = $Bitmap.GetPixel([int] (10 * $Scale), [int] (160 * $Scale))
    if (($Dark -and $background.R -gt 30) -or (!$Dark -and $background.R -lt 230)) { return }
    $result = @{}
    foreach ($symbol in @(@{ name = 'minimize'; offset = 114 }, @{ name = 'maximize'; offset = 69 }, @{ name = 'close'; offset = 23 })) {
        $region = [Drawing.Rectangle]::new(($Bitmap.Width - ($symbol.offset + 12) * $Scale), (8 * $Scale), (24 * $Scale), (27 * $Scale))
        $ink = Measure-Ink $Bitmap $region $Dark
        if ($ink.pixels -lt 5 -or $ink.width -lt 4 * $Scale -or $ink.width -gt 18 * $Scale -or $ink.height -gt 18 * $Scale) { return }
        $result[$symbol.name] = $ink
    }

    # Take the complete glyph bounds in the 900x650 CSS-pixel library fixture.
    $contentLeft = 28 * $Scale
    foreach ($text in @(@{ name = 'brand'; x = 70; y = 76; width = 200; height = 45 },
        @{ name = 'heading'; x = 0; y = 141; width = 220; height = 43 },
        @{ name = 'firstProject'; x = 64; y = 411; width = 285; height = 39 })) {
        $region = [Drawing.Rectangle]::new(($contentLeft + $text.x * $Scale), ($text.y * $Scale), ($text.width * $Scale), ($text.height * $Scale))
        $ink = Measure-Ink $Bitmap $region $Dark
        if ($ink.pixels -lt 35) { return }
        $result[$text.name] = $ink
    }
    return $result
}

function Save-LibraryCapture([string] $Runtime, [string] $Executable, [string] $FixtureProfile, [string] $Theme) {
    $start = [Diagnostics.ProcessStartInfo]::new($Executable)
    $start.WorkingDirectory = $workspace
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.Environment['REFRACT_DATA_DIR'] = $FixtureProfile
    $start.Environment['REFRACT_TEX_BIN'] = $FixtureProfile
    if ($Runtime -eq 'electron') {
        $start.ArgumentList.Add($workspace)
        $start.Environment.Remove('REFRACT_TEST_HIDDEN') | Out-Null
        $start.Environment.Remove('ELECTRON_RUN_AS_NODE') | Out-Null
    }
    $app = [Diagnostics.Process]::Start($start)
    try {
        $deadline = [Diagnostics.Stopwatch]::StartNew()
        while ($deadline.Elapsed.TotalSeconds -lt 30) {
            $app.Refresh()
            if ($app.HasExited) { throw "$Runtime Refract exited with code $($app.ExitCode) before showing its library." }
            $window = $app.MainWindowHandle
            if ($window -ne [IntPtr]::Zero -and $app.MainWindowTitle -eq 'Refract' -and [RefractCaptureWindow]::IsWindowVisible($window)) { break }
            Start-Sleep -Milliseconds 200
        }
        if ($window -eq [IntPtr]::Zero -or $app.MainWindowTitle -ne 'Refract') { throw "$Runtime did not show a Refract window on the hosted desktop." }
        $scale = [RefractCaptureWindow]::GetDpiForWindow($window) / 96.0
        if ($scale -le 0) { throw 'Could not determine the Refract display DPI.' }
        $client = Set-ClientSize $window ([int] (900 * $scale)) ([int] (650 * $scale))
        $bitmap = [Drawing.Bitmap]::new($client.Width, $client.Height, [Drawing.Imaging.PixelFormat]::Format32bppRgb)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $unobstructed = $false
            $name = Join-Path $OutputDirectory "$Runtime-$Theme-library"
            $deadline.Restart()
            while ($deadline.Elapsed.TotalSeconds -lt 20 -and !$app.HasExited) {
                Start-Sleep -Milliseconds 250
                if (!(Test-ClientVisible $window $client)) { continue }
                $unobstructed = $false
                $graphics.CopyFromScreen($client.Location, [Drawing.Point]::Empty, $client.Size)
                if (!(Test-ClientVisible $window $client)) { continue }
                $unobstructed = $true
                $sample = Measure-Library $bitmap $scale ($Theme -eq 'dark')
                if (!$sample) { continue }
                $bitmap.Save("$name.png", [Drawing.Imaging.ImageFormat]::Png)
                @{ width = $client.Width; height = $client.Height; dpi = $scale * 96; ink = $sample } |
                    ConvertTo-Json -Depth 4 | Set-Content -LiteralPath "$name.json" -Encoding utf8NoBOM
                return
            }
            if ($unobstructed) { $bitmap.Save("$name-unverified.png", [Drawing.Imaging.ImageFormat]::Png) }
            throw "No unobstructed $Theme $Runtime frame contained the Refract title bar symbols and library text."
        } finally {
            $graphics.Dispose()
            $bitmap.Dispose()
        }
    } finally {
        if (!$app.HasExited) {
            $app.Kill($true)
            $app.WaitForExit(5000) | Out-Null
        }
        $app.Dispose()
    }
}

$workspace = Split-Path -Parent $PSScriptRoot
$names = @('Spectral methods for fluid dynamics', 'Conference paper - autumn notes', 'A short introduction to graph theory')
$scratch = Join-Path $env:RUNNER_TEMP ('refract-visual-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $OutputDirectory, $scratch -Force | Out-Null
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
$original = Get-ItemPropertyValue -LiteralPath $key -Name AppsUseLightTheme -ErrorAction SilentlyContinue
if (!(Test-Path -LiteralPath $key)) { New-Item -Path $key -Force | Out-Null }
# Read Win32 client geometry in physical pixels even when the display DPI is not 96.
$oldDpi = [RefractCaptureWindow]::SetThreadDpiAwarenessContext([IntPtr] -4)
try {
    foreach ($theme in @('light', 'dark')) {
        New-ItemProperty -LiteralPath $key -Name AppsUseLightTheme -Value ([int] ($theme -eq 'light')) -PropertyType DWord -Force | Out-Null
        foreach ($runtime in @('electron', 'native')) {
            $fixtureProfile = Join-Path $scratch "$runtime-$theme"
            for ($index = 0; $index -lt $names.Count; $index++) {
                $id = '2b98e215-1a9f-4f24-972b-{0:D12}' -f $index
                $project = Join-Path $fixtureProfile "projects/$id"
                New-Item -ItemType Directory -Path $project | Out-Null
                @{ id = $id; name = $names[$index]; created = '2026-01-01T12:00:00.000Z'; rootFile = 'main.tex' } |
                    ConvertTo-Json -Compress | Set-Content -LiteralPath (Join-Path $project '.refract.json') -Encoding utf8NoBOM
                Copy-Item -LiteralPath (Join-Path $workspace 'test/fixtures/parity-document.tex') -Destination (Join-Path $project 'main.tex')
            }
            $relativeExe = if ($runtime -eq 'native') { 'native/target/debug/refract-native.exe' } else { 'node_modules/electron/dist/electron.exe' }
            Save-LibraryCapture $runtime (Join-Path $workspace $relativeExe) $fixtureProfile $theme
        }
    }
} finally {
    if ($null -eq $original) { Remove-ItemProperty -LiteralPath $key -Name AppsUseLightTheme -ErrorAction SilentlyContinue }
    else { Set-ItemProperty -LiteralPath $key -Name AppsUseLightTheme -Value $original }
    if ($oldDpi -ne [IntPtr]::Zero) { [RefractCaptureWindow]::SetThreadDpiAwarenessContext($oldDpi) | Out-Null }
}
