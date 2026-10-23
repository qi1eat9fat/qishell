param([int]$X, [int]$Y)
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class Mouse {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
'@
[void][Mouse]::SetProcessDpiAwarenessContext([IntPtr](-4))
[void][Mouse]::SetCursorPos($X, $Y)
Start-Sleep -Milliseconds 80
[Mouse]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 40
[Mouse]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
