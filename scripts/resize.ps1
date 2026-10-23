param([int]$Width = 1280, [int]$Height = 800)

Add-Type @'
using System;
using System.Runtime.InteropServices;
public class QiShellResize {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
  [DllImport("user32.dll")] public static extern uint GetDpiForSystem();
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int hh, uint flags);
}
'@

[void][QiShellResize]::SetProcessDpiAwarenessContext([IntPtr](-4))
$dpi = [QiShellResize]::GetDpiForSystem()
$scale = $dpi / 96.0
$process = Get-Process qishell -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $process) { Write-Output "qishell is not running"; exit 1 }
$w = [int]($Width * $scale)
$h = [int]($Height * $scale)
[void][QiShellResize]::SetWindowPos($process.MainWindowHandle, [IntPtr]::Zero, 80, 80, $w, $h, 0x0040)
Write-Output ("dpi=" + $dpi + " logical=" + $Width + "x" + $Height + " physical=" + $w + "x" + $h)
