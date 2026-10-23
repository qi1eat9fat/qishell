param([int]$X, [int]$Y)

Add-Type @'
using System;
using System.Runtime.InteropServices;
public class RightMouse {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
'@

[void][RightMouse]::SetProcessDpiAwarenessContext([IntPtr](-4))
[void][RightMouse]::SetCursorPos($X, $Y)
Start-Sleep -Milliseconds 80
[RightMouse]::mouse_event(0x0008, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 40
[RightMouse]::mouse_event(0x0010, 0, 0, 0, [UIntPtr]::Zero)
