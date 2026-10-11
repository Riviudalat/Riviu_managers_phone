# Hiện cửa sổ của controller nghiệm thu. launch.py mở app với RIVIU_DEV_BACKGROUND=1
# (bật CDP cho các script), nên cửa sổ Tauri bị ẩn. Script chỉ gọi ShowWindow/
# SetForegroundWindow, không đụng controller hay điện thoại.
# Dùng: powershell -NoProfile -File scripts/device-acceptance/show-window.ps1 -ProcId <pid>
# Không có -Show thì chỉ liệt kê cửa sổ của process.
param([int]$ProcId, [switch]$Show)
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  public struct RECT { public int L, T, R, B; }
}
"@
$list = New-Object System.Collections.ArrayList
[W]::EnumWindows({ param($h,$l) $p=0; [void][W]::GetWindowThreadProcessId($h,[ref]$p); if ($p -eq $ProcId) { [void]$list.Add($h) }; $true }, [IntPtr]::Zero) | Out-Null
foreach ($h in $list) {
  $t = New-Object Text.StringBuilder 256; [void][W]::GetWindowText($h,$t,256)
  $c = New-Object Text.StringBuilder 256; [void][W]::GetClassName($h,$c,256)
  $r = New-Object W+RECT; [void][W]::GetWindowRect($h,[ref]$r)
  "{0} vis={1} min={2} cls={3} title='{4}' rect={5},{6},{7},{8}" -f $h,[W]::IsWindowVisible($h),[W]::IsIconic($h),$c,$t,$r.L,$r.T,$r.R,$r.B
  if ($Show -and $c.ToString() -like 'Tauri Window*') { [void][W]::ShowWindow($h, 9); [void][W]::SetForegroundWindow($h); "shown $h" }
}
