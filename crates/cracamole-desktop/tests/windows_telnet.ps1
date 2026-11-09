param([string]$Binary, [int]$Port, [string]$SyncDir, [switch]$RemoteClose, [switch]$Expire)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class TelnetGui {
    public static string PixelDiagnostic = "not sampled";
    public delegate bool Callback(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Callback c, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, Callback c, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, string s);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] public static extern IntPtr SendMessageInt(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int command);
    [DllImport("user32.dll", EntryPoint="GetWindowLongW")] public static extern int GetWindowLong(IntPtr h, int index);
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(Point p);
    [DllImport("user32.dll")] private static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] private static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] private static extern uint GetPixel(IntPtr dc, int x, int y);
    public static bool VisibleTerminalInk(IntPtr frame, uint pid) {
        Rect rect;
        if (!GetWindowRect(frame, out rect)) { PixelDiagnostic = "GetWindowRect failed"; return false; }
        IntPtr dc = GetDC(IntPtr.Zero);
        if (dc == IntPtr.Zero) { PixelDiagnostic = "GetDC failed"; return false; }
        int red = 0, bright = 0;
        try {
            for (int y = rect.Top + 2; y < Math.Min(rect.Bottom, rect.Top + 54); y += 4) {
                for (int x = rect.Left + 2; x < Math.Min(rect.Right, rect.Left + 122); x += 4) {
                    uint owner;
                    GetWindowThreadProcessId(WindowFromPoint(new Point { X = x, Y = y }), out owner);
                    if (owner != pid) { PixelDiagnostic = String.Format("frame=({0},{1},{2},{3}) point=({4},{5}) owner={6} expected={7}", rect.Left, rect.Top, rect.Right, rect.Bottom, x, y, owner, pid); return false; }
                    uint color = GetPixel(dc, x, y);
                    if (color == 0xffffffff) { PixelDiagnostic = String.Format("GetPixel failed at ({0},{1})", x, y); return false; }
                    int r = (int)(color & 255), g = (int)((color >> 8) & 255), b = (int)((color >> 16) & 255);
                    if (r > 120 && g < 110 && b < 110) red++;
                    if (r > 140 && g > 140 && b > 140) bright++;
                    if (red > 8 && bright > 3) return true;
                }
            }
            PixelDiagnostic = String.Format("frame=({0},{1},{2},{3}) red={4} bright={5}", rect.Left, rect.Top, rect.Right, rect.Bottom, red, bright);
            return false;
        } finally { ReleaseDC(IntPtr.Zero, dc); }
    }
    public static string Text(IntPtr h) { var s = new StringBuilder(4096); GetWindowText(h,s,s.Capacity); return s.ToString(); }
    public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassName(h,s,s.Capacity); return s.ToString(); }
    public static IntPtr Find(uint pid) { IntPtr result = IntPtr.Zero; EnumWindows((h,p) => { uint owner; GetWindowThreadProcessId(h,out owner); if (owner == pid && Text(h).StartsWith("cracamole — Windows")) { result=h; return false; } return true; },IntPtr.Zero); return result; }
    public static IntPtr[] Children(IntPtr h) { var result = new List<IntPtr>(); EnumChildWindows(h,(child,p) => { result.Add(child); return true; },IntPtr.Zero); return result.ToArray(); }
}
'@
function Wait-For([scriptblock]$Condition, [string]$Message) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not (& $Condition)) { if ([DateTime]::UtcNow -ge $deadline) { throw $Message }; Start-Sleep -Milliseconds 50 }
}
function Click([IntPtr]$h) { [void][TelnetGui]::SendMessage($h, 0x00F5, [IntPtr]::Zero, $null) }
function Set-Text([IntPtr]$h, [string]$s) { [void][TelnetGui]::SendMessage($h, 0x000C, [IntPtr]::Zero, $s) }
$env:CRACAMOLE_ADMIN_USERNAME = 'test-admin'
$env:CRACAMOLE_ADMIN_PASSWORD = 'desktop-test-password-123'
$env:CRACAMOLE_DESKTOP_SESSION_MAX_SECS = if ($Expire) { '8' } else { '60' }
$testDir = Join-Path ([IO.Path]::GetTempPath()) ('cracamole-telnet-gui-' + [Guid]::NewGuid().ToString('N'))
$env:CRACAMOLE_DESKTOP_DATA_DIR = $testDir
$process = $null
try {
    $process = Start-Process -FilePath $Binary -WindowStyle Hidden -PassThru
    Wait-For { [TelnetGui]::Find([uint32]$process.Id) -ne [IntPtr]::Zero } '找不到桌面視窗'
    $window = [TelnetGui]::Find([uint32]$process.Id)
    if (-not $process.WaitForInputIdle(10000)) { throw '控制項初始化逾時' }
    $children = [TelnetGui]::Children($window)
    $inputs = @($children | Where-Object { [TelnetGui]::Class($_) -eq 'Edit' })
    $combos = @($children | Where-Object { [TelnetGui]::Class($_) -eq 'ComboBox' })
    $list = @($children | Where-Object { [TelnetGui]::Class($_) -eq 'ListBox' })[0]
    if ($inputs.Count -ne 13 -or $combos.Count -ne 3) { throw '找不到預期的編輯控制項' }
    function Button([string]$label) { $found = @($children | Where-Object { [TelnetGui]::Text($_) -eq $label }); if ($found.Count -ne 1) { throw "找不到按鈕：$label" }; return $found[0] }
    Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
    if ($env:CRACAMOLE_TEST_DESKTOP_KEYBOARD) {
        [void][TelnetGui]::SendMessageInt($inputs[1], 0x0100, [IntPtr]13, [IntPtr]::Zero)
    } else { Click (Button '登入') }
    if ($env:CRACAMOLE_TEST_LIBRARY_RESIZE -eq '1') {
        $before = New-Object TelnetGui+Rect
        $after = New-Object TelnetGui+Rect
        if (-not [TelnetGui]::GetWindowRect($list, [ref]$before)) { throw '無法讀取清單尺寸' }
        if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 900, 700, 0x0006)) { throw '無法調整視窗' }
        Wait-For {
            [TelnetGui]::GetWindowRect($list, [ref]$after) -and
            ($after.Right - $after.Left) -gt ($before.Right - $before.Left) -and
            ($after.Bottom - $after.Top) -gt ($before.Bottom - $before.Top)
        } '連線清單未隨視窗放大'
    }
    Click (Button '新增')
    if ($env:CRACAMOLE_TEST_DESKTOP_KEYBOARD) {
        Set-Text $inputs[3] '應取消的連線'
        [void][TelnetGui]::SendMessageInt($inputs[3], 0x0100, [IntPtr]27, [IntPtr]::Zero)
        Wait-For { ([TelnetGui]::GetWindowLong($inputs[3], -16) -band 0x10000000) -eq 0 } 'Escape 未關閉編輯器'
        Click (Button '新增')
    }
    Set-Text $inputs[3] 'Telnet fixture'
    [void][TelnetGui]::SendMessageInt($combos[0], 0x014E, [IntPtr]1, [IntPtr]::Zero)
    $selectionChanged = [IntPtr]([TelnetGui]::GetDlgCtrlID($combos[0]) -bor (1 -shl 16))
    [void][TelnetGui]::SendMessageInt($window, 0x0111, $selectionChanged, $combos[0])
    Set-Text $inputs[4] '127.0.0.1'
    Set-Text $inputs[5] ([string]$Port)
    if ($env:CRACAMOLE_TEST_DESKTOP_KEYBOARD) {
        [void][TelnetGui]::SendMessageInt($inputs[4], 0x0100, [IntPtr]13, [IntPtr]::Zero)
    } else { Click (Button '儲存') }
    if ($env:CRACAMOLE_TEST_DESKTOP_KEYBOARD) {
        if ([TelnetGui]::SendMessageInt($list, 0x0188, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero) { throw '連線清單未預設選取首筆' }
        if ($env:CRACAMOLE_TEST_DESKTOP_KEYBOARD -eq 'double') {
            $doubleClick = [IntPtr]([TelnetGui]::GetDlgCtrlID($list) -bor (2 -shl 16))
            [void][TelnetGui]::SendMessageInt($window, 0x0111, $doubleClick, $list)
        } else { [void][TelnetGui]::SendMessageInt($list, 0x0100, [IntPtr]13, [IntPtr]::Zero) }
    } else {
        [void][TelnetGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
        Click (Button '連線')
    }
    Wait-For { @($children | Where-Object { [TelnetGui]::Class($_) -eq 'Static' -and [TelnetGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -gt 0 } 'Telnet 畫面未顯示'
    if ($env:CRACAMOLE_TEST_VISIBLE_TELNET -eq '1') {
        $frame = @($children | Where-Object { [TelnetGui]::Class($_) -eq 'Static' -and [TelnetGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero })[0]
        try {
            [void][TelnetGui]::ShowWindow($window, 4) # SW_SHOWNOACTIVATE
            if (-not [TelnetGui]::SetWindowPos($window, [IntPtr](-1), 0, 0, 0, 0, 0x0011)) { throw '無法顯示測試視窗' }
            try { Wait-For { [TelnetGui]::VisibleTerminalInk($frame, [uint32]$process.Id) } '未偵測到 Telnet 文字及背景' }
            catch { throw "未偵測到 Telnet 文字及背景：$([TelnetGui]::PixelDiagnostic)" }
            Write-Output '已確認 Windows 螢幕上的 Telnet 測試文字與背景。'
        } finally { [void][TelnetGui]::ShowWindow($window, 0) }
    }
    [void][TelnetGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]122, [IntPtr]::Zero)
    [void][TelnetGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]0x4e2d, [IntPtr]::Zero)
    if ($Expire) {
        Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '已登出，請重新登入。' }).Count -eq 1 } '工作階段未到期'
        Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'closed')) } '工作階段到期後 Telnet 連線未正常關閉'
        if (@($children | Where-Object { [TelnetGui]::Class($_) -eq 'Static' -and [TelnetGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -ne 0) { throw '工作階段到期後仍顯示舊影格' }
    } elseif ($RemoteClose) {
        Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '連線已結束或失敗。' }).Count -eq 1 } '遠端關閉後狀態未更新'
        if (@($children | Where-Object { [TelnetGui]::Class($_) -eq 'Static' -and [TelnetGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -ne 0) { throw '遠端關閉後仍顯示舊影格' }
    } else {
        Start-Sleep -Milliseconds 500
        Click (Button '登出')
        Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'closed')) } '登出後 Telnet 連線未正常關閉'
    }
    [void][TelnetGui]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $process.WaitForExit(10000)) { throw '桌面程式未結束' }
    $process.Refresh(); if ($process.ExitCode -ne 0) { throw '桌面程式異常結束' }
} finally {
    if ($process -and -not $process.HasExited) { Stop-Process -Id $process.Id -Force; [void]$process.WaitForExit(10000) }
    if ([IO.Directory]::Exists($testDir)) {
        [IO.File]::Delete((Join-Path $testDir 'connections.json'))
        [IO.File]::Delete((Join-Path $testDir 'desktop.lock'))
        [IO.Directory]::Delete($testDir)
    }
}
