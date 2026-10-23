param([string]$Path = "docs\screenshots\window.png")

Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class QiShellWindow {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern uint GetDpiForSystem();
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
'@

# PER_MONITOR_AWARE_V2 = -4：成功时 Win32 坐标已是物理像素；失败时按系统 DPI 换算。
$aware = [QiShellWindow]::SetProcessDpiAwarenessContext([IntPtr](-4))

$process = Get-Process qishell -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $process) { Write-Output "qishell is not running"; exit 1 }

$rect = New-Object QiShellWindow+RECT
[void][QiShellWindow]::GetWindowRect($process.MainWindowHandle, [ref]$rect)
# Win32 在 DPI 不感知的 PowerShell 里返回逻辑像素；GetDpiForSystem 给出缩放。
$scale = if ($aware) { 1.0 } else { [QiShellWindow]::GetDpiForSystem() / 96.0 }
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$screenWidth = [int]($screen.Width * $scale)
$screenHeight = [int]($screen.Height * $scale)
$rectLeft = [int]($rect.Left * $scale)
$rectTop = [int]($rect.Top * $scale)
$rectRight = [int]($rect.Right * $scale)
$rectBottom = [int]($rect.Bottom * $scale)
$left = [Math]::Max(0, $rectLeft)
$top = [Math]::Max(0, $rectTop)
$right = [Math]::Min($screenWidth, $rectRight)
$bottom = [Math]::Min($screenHeight, $rectBottom)
$width = [Math]::Max(1, $right - $left)
$height = [Math]::Max(1, $bottom - $top)

$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen(
    (New-Object System.Drawing.Point $left, $top),
    [System.Drawing.Point]::Empty,
    (New-Object System.Drawing.Size $width, $height))

$resolved = Join-Path (Get-Location) $Path
$directory = Split-Path $resolved -Parent
if (-not (Test-Path $directory)) { New-Item -ItemType Directory -Path $directory | Out-Null }
$bitmap.Save($resolved, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose(); $bitmap.Dispose()
Write-Output ("saved " + $resolved + " (" + $width + "x" + $height + ", window " + $rect.Left + "," + $rect.Top + ")")
