param([string]$Binary, [string]$Fixture, [switch]$VisiblePixelCheck, [switch]$Nla, [switch]$Auto, [switch]$InteractiveClipboard)
$ErrorActionPreference = 'Stop'
if ($InteractiveClipboard -and $env:CRACAMOLE_ALLOW_INTERACTIVE_RDP_CLIPBOARD_TEST -ne '1') {
    throw 'interactive clipboard test requires explicit CRACAMOLE_ALLOW_INTERACTIVE_RDP_CLIPBOARD_TEST=1'
}
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class RdpFullGui {
    public delegate bool Callback(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Callback c, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, Callback c, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll")] private static extern IntPtr GetClipboardOwner();
    public static bool OwnsClipboard(uint pid) {
        IntPtr owner = GetClipboardOwner();
        if (owner == IntPtr.Zero) return false;
        uint ownerPid;
        GetWindowThreadProcessId(owner, out ownerPid);
        return ownerPid == pid;
    }
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, string s);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] public static extern IntPtr SendMessageInt(IntPtr h, uint msg, IntPtr w, IntPtr l);
    public static void SimulateFocusLoss(IntPtr input) {
        SendMessageInt(input, 0x0100, (IntPtr)0x11, IntPtr.Zero); // Ctrl down
        SendMessageInt(input, 0x0100, (IntPtr)0x41, IntPtr.Zero); // A down
        SendMessageInt(input, 0x0008, IntPtr.Zero, IntPtr.Zero); // WM_KILLFOCUS
    }
    public static void SimulateArrowFocusLoss(IntPtr input) {
        SendMessageInt(input, 0x0100, (IntPtr)0x25, IntPtr.Zero); // Left down
        SendMessageInt(input, 0x0008, IntPtr.Zero, IntPtr.Zero); // WM_KILLFOCUS
    }
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll", EntryPoint="GetWindowLongW")] public static extern int GetWindowLong(IntPtr h, int index);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int width, int height, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int command);
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] private static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] private static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] private static extern uint GetPixel(IntPtr dc, int x, int y);
    public static bool VisibleFixturePixels(IntPtr frame, uint pid, uint color1, uint color2) {
        Rect rect;
        if (!GetWindowRect(frame, out rect)) return false;
        IntPtr dc = GetDC(IntPtr.Zero);
        if (dc == IntPtr.Zero) return false;
        try {
            foreach (int dx in new int[] { -80, 0, 80 }) {
                foreach (int dy in new int[] { -60, 0, 60 }) {
                    Point point = new Point { X = (rect.Left + rect.Right) / 2 + dx, Y = (rect.Top + rect.Bottom) / 2 + dy };
                    uint owner;
                    GetWindowThreadProcessId(WindowFromPoint(point), out owner);
                    if (owner != pid) return false;
                    uint color = GetPixel(dc, point.X, point.Y);
                    if (color != color1 && color != color2) return false;
                }
            }
            return true;
        }
        finally { ReleaseDC(IntPtr.Zero, dc); }
    }
    public static bool HasVisibleStyle(IntPtr h) { return (GetWindowLong(h, -16) & 0x10000000) != 0; }
    public static IntPtr Find(uint pid, string cls) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((h,p) => { uint owner; GetWindowThreadProcessId(h, out owner); if (owner == pid && (String.IsNullOrEmpty(cls) ? Text(h).StartsWith("cracamole — Windows") : Class(h) == cls)) { result = h; return false; } return true; }, IntPtr.Zero);
        return result;
    }
    public static IntPtr[] Children(IntPtr parent) { var result = new List<IntPtr>(); EnumChildWindows(parent, (h,p) => { result.Add(h); return true; }, IntPtr.Zero); return result.ToArray(); }
    public static string Text(IntPtr h) { var s = new StringBuilder(4096); GetWindowText(h,s,s.Capacity); return s.ToString(); }
    public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassName(h,s,s.Capacity); return s.ToString(); }
}
'@
function Wait-For([scriptblock]$Condition, [string]$Message, [int]$Seconds = 15) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (-not (& $Condition)) { if ([DateTime]::UtcNow -ge $deadline) { throw $Message }; Start-Sleep -Milliseconds 50 }
}
function Click([IntPtr]$Handle) { [void][RdpFullGui]::SendMessage($Handle, 0x00F5, [IntPtr]::Zero, $null) }
function Check-VisibleFrame([IntPtr]$Window, [IntPtr]$Frame, [uint32]$ProcessId, [uint32]$Color1, [uint32]$Color2) {
    try {
        [void][RdpFullGui]::ShowWindow($Window, 4) # SW_SHOWNOACTIVATE
        if (-not [RdpFullGui]::SetWindowPos($Window, [IntPtr](-1), 0, 0, 0, 0, 0x0011)) { throw 'cannot place test window above other windows' }
        Wait-For { [RdpFullGui]::VisibleFixturePixels($Frame, $ProcessId, $Color1, $Color2) } 'visible fixture pixels not detected' 5
    } finally {
        [void][RdpFullGui]::ShowWindow($Window, 0) # SW_HIDE
    }
}
function Set-Text([IntPtr]$Handle, [string]$Value) { [void][RdpFullGui]::SendMessage($Handle, 0x000C, [IntPtr]::Zero, $Value) }
function Button([string]$Text) {
    $buttons = @($children | Where-Object { [RdpFullGui]::Text($_) -eq $Text })
    if ($buttons.Count -ne 1) { throw "button not found: $Text" }; return $buttons[0]
}
$savedEnv = @{}
foreach ($name in @('CRACAMOLE_ADMIN_USERNAME', 'CRACAMOLE_ADMIN_PASSWORD', 'CRACAMOLE_DATA_DIR', 'CRACAMOLE_DESKTOP_DATA_DIR', 'CRACAMOLE_DESKTOP_SESSION_MAX_SECS')) { $savedEnv[$name] = [Environment]::GetEnvironmentVariable($name) }
$env:CRACAMOLE_ADMIN_USERNAME = 'test-admin'
$env:CRACAMOLE_ADMIN_PASSWORD = 'desktop-test-password-123'
$env:CRACAMOLE_DESKTOP_SESSION_MAX_SECS = '60'
$testDir = Join-Path ([IO.Path]::GetTempPath()) ('cracamole-rdp-full-' + [Guid]::NewGuid().ToString('N'))
$fixtureDir = Join-Path $testDir 'fixture'
[void][IO.Directory]::CreateDirectory($fixtureDir)
$desktopDir = Join-Path $testDir 'desktop'
$env:CRACAMOLE_DESKTOP_DATA_DIR = $desktopDir
$process = $null; $server = $null
try {
    $fixtureMode = if ($InteractiveClipboard) { if ($Nla) { 'nla-frames-clipboard' } else { 'frames-clipboard' } } elseif ($Nla) { 'nla-frames' } else { 'frames' }
    $server = Start-Process -FilePath $Fixture -ArgumentList @('"' + $fixtureDir + '"', $fixtureMode) -WindowStyle Hidden -PassThru
    Wait-For { [IO.File]::Exists((Join-Path $fixtureDir 'ready')) -or $server.HasExited } 'fixture startup timed out' 30
    if ($server.HasExited) { throw 'fixture exited before startup' }
    $ready = [IO.File]::ReadAllLines((Join-Path $fixtureDir 'ready'))
    if ($ready.Count -ne 2 -or $ready[1].Length -ne 64) { throw 'invalid fixture endpoint' }
    $process = Start-Process -FilePath $Binary -WindowStyle Hidden -PassThru
    Wait-For { [RdpFullGui]::Find([uint32]$process.Id, $null) -ne [IntPtr]::Zero } 'window not found' 30
    $window = [RdpFullGui]::Find([uint32]$process.Id, $null)
    if (-not $process.WaitForInputIdle(10000)) { throw 'controls timed out' }
    $children = [RdpFullGui]::Children($window)
    $inputs = @($children | Where-Object { [RdpFullGui]::Class($_) -eq 'Edit' })
    $combos = @($children | Where-Object { [RdpFullGui]::Class($_) -eq 'ComboBox' })
    $list = @($children | Where-Object { [RdpFullGui]::Class($_) -eq 'ListBox' })[0]
    if ($inputs.Count -ne 13 -or $combos.Count -ne 3) { throw 'editor controls missing' }
    Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
    Click (Button '登入'); Click (Button '新增')
    Set-Text $inputs[3] 'RDP full fixture'
    if ([RdpFullGui]::HasVisibleStyle((Button '禁止從遠端複製')) -or [RdpFullGui]::HasVisibleStyle((Button '禁止貼到遠端'))) { throw 'RDP restrictions visible for SSH' }
    [void][RdpFullGui]::SendMessageInt($combos[0], 0x014E, [IntPtr]2, [IntPtr]::Zero)
    $selectionChanged = [IntPtr]([RdpFullGui]::GetDlgCtrlID($combos[0]) -bor (1 -shl 16))
    [void][RdpFullGui]::SendMessageInt($window, 0x0111, $selectionChanged, $combos[0])
    if (-not [RdpFullGui]::HasVisibleStyle((Button '禁止從遠端複製')) -or -not [RdpFullGui]::HasVisibleStyle((Button '禁止貼到遠端'))) { throw 'RDP restrictions hidden after protocol selection' }
    $securityIndex = if ($Auto) { 2 } elseif ($Nla) { 0 } else { 1 }
    [void][RdpFullGui]::SendMessageInt($combos[1], 0x014E, [IntPtr]$securityIndex, [IntPtr]::Zero)
    Set-Text $inputs[4] '127.0.0.1'; Set-Text $inputs[5] $ready[0]
    Set-Text $inputs[6] 'fixture'; Set-Text $inputs[7] 'fixture-secret'
    [void][RdpFullGui]::SendMessageInt((Button '禁止從遠端複製'), 0x00F1, [IntPtr]1, [IntPtr]::Zero)
    [void][RdpFullGui]::SendMessageInt((Button '禁止貼到遠端'), 0x00F1, [IntPtr]1, [IntPtr]::Zero)
    [void][RdpFullGui]::SendMessageInt((Button '唯讀（禁止遠端輸入）'), 0x00F1, [IntPtr]1, [IntPtr]::Zero)
    Click (Button '儲存')
    $expectedSecurity = if ($Auto) { 'any' } elseif ($Nla) { 'nla' } else { 'tls' }
    $saved = (Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value
    if ($saved.params.security -ne $expectedSecurity) { throw 'RDP security selection was not saved' }
    [void][RdpFullGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    Click (Button '編輯')
    if ([RdpFullGui]::SendMessageInt((Button '禁止從遠端複製'), 0x00F0, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1 -or [RdpFullGui]::SendMessageInt((Button '禁止貼到遠端'), 0x00F0, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1) { throw 'clipboard restrictions not restored in editor' }
    if ([RdpFullGui]::SendMessageInt((Button '唯讀（禁止遠端輸入）'), 0x00F0, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1) { throw 'read-only setting not restored in editor' }
    [void][RdpFullGui]::SendMessageInt((Button '禁止從遠端複製'), 0x00F1, [IntPtr]::Zero, [IntPtr]::Zero)
    if ($InteractiveClipboard) { [void][RdpFullGui]::SendMessageInt((Button '禁止貼到遠端'), 0x00F1, [IntPtr]::Zero, [IntPtr]::Zero) }
    [void][RdpFullGui]::SendMessageInt((Button '唯讀（禁止遠端輸入）'), 0x00F1, [IntPtr]::Zero, [IntPtr]::Zero)
    Click (Button '儲存')
    [void][RdpFullGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    Click (Button '編輯')
    $expectedPasteRestriction = if ($InteractiveClipboard) { [IntPtr]::Zero } else { [IntPtr]1 }
    if ([RdpFullGui]::SendMessageInt((Button '禁止從遠端複製'), 0x00F0, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero -or [RdpFullGui]::SendMessageInt((Button '禁止貼到遠端'), 0x00F0, [IntPtr]::Zero, [IntPtr]::Zero) -ne $expectedPasteRestriction) { throw 'clipboard restriction changes not saved' }
    Click (Button '取消')
    [void][RdpFullGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][RdpFullGui]::PostMessage((Button '核對身分'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-For { [RdpFullGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } 'certificate dialog not found'
    $dialog = [RdpFullGui]::Find([uint32]$process.Id, '#32770')
    Wait-For { (([RdpFullGui]::Children($dialog) | ForEach-Object { [RdpFullGui]::Text($_) }) -join "`n").Contains($ready[1]) } 'certificate fingerprint mismatch'
    [void][RdpFullGui]::PostMessage($dialog, 0x0111, [IntPtr]6, [IntPtr]::Zero)
    Wait-For { @($children | Where-Object { [RdpFullGui]::Text($_) -eq '已儲存主機身分。' }).Count -eq 1 } 'certificate not saved'
    [void][RdpFullGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    Click (Button '連線')
    Wait-For { @($children | Where-Object { [RdpFullGui]::Class($_) -eq 'Static' -and [RdpFullGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -gt 0 } 'RDP frame not installed' 25
    $frameControl = @($children | Where-Object { [RdpFullGui]::Class($_) -eq 'Static' -and [RdpFullGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero })[0]
    $initialGdi = [RdpFullGui]::GetGuiResources($process.Handle, 0)
    if ($initialGdi -eq 0) { throw 'cannot read GDI resource count' }
    $bitmaps = [Collections.Generic.HashSet[long]]::new()
    Wait-For {
        [void]$bitmaps.Add([RdpFullGui]::SendMessageInt($frameControl, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64())
        $count = 0
        $path = Join-Path $fixtureDir 'frames'
        try { [void][int]::TryParse([IO.File]::ReadAllText($path), [ref]$count) } catch [IO.IOException] {}
        $count -ge 80
    } 'repeated RDP frames timed out' 20
    if ($bitmaps.Count -lt 8) { throw 'too few distinct bitmaps installed for GDI resource check' }
    $finalGdi = [RdpFullGui]::GetGuiResources($process.Handle, 0)
    if ($finalGdi -eq 0 -or $finalGdi -gt $initialGdi + 10) { throw "GDI resources grew during repeated frames: $initialGdi -> $finalGdi" }
    Write-Output "重複影格：已觀察 $($bitmaps.Count) 個點陣圖，GDI 物件 $initialGdi -> $finalGdi。"
    Wait-For { try { [IO.File]::ReadAllText((Join-Path $fixtureDir 'frames')) -eq '100' } catch [IO.IOException] { $false } } 'fixture did not finish initial frames' 15
    $script:lastBitmap = [IntPtr]::Zero
    $script:bitmapStableSince = [DateTime]::UtcNow
    Wait-For {
        $current = [RdpFullGui]::SendMessageInt($frameControl, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero)
        if ($current -ne $script:lastBitmap) { $script:lastBitmap = $current; $script:bitmapStableSince = [DateTime]::UtcNow }
        $current -ne [IntPtr]::Zero -and ([DateTime]::UtcNow - $script:bitmapStableSince).TotalMilliseconds -ge 750
    } 'initial bitmap did not settle' 10
    $beforeResizeBitmap = $script:lastBitmap
    if ($VisiblePixelCheck) {
        Check-VisibleFrame $window $frameControl ([uint32]$process.Id) 0x1971d3 0x1971d2
        Write-Output 'Windows 螢幕合成後的 9 個測試畫面像素符合預期。'
    }
    if (-not [RdpFullGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 900, 700, 0x0006) -or -not [RdpFullGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 1000, 800, 0x0006)) { throw 'window resize failed' }
    $rect = New-Object RdpFullGui+Rect
    if (-not [RdpFullGui]::GetWindowRect($frameControl, [ref]$rect)) { throw 'cannot measure frame control' }
    $expectedWidth = [Math]::Max(200, [Math]::Min(4096, ($rect.Right - $rect.Left))) -band -2
    $expectedHeight = [Math]::Max(200, [Math]::Min(4096, ($rect.Bottom - $rect.Top)))
    $expectedLayout = "${expectedWidth}x${expectedHeight}"
    Wait-For { try { [IO.File]::ReadAllText((Join-Path $fixtureDir 'layout')) -eq $expectedLayout } catch [IO.IOException] { $false } } "RDP server did not receive final window size $expectedLayout" 15
    Wait-For {
        $current = [RdpFullGui]::SendMessageInt($frameControl, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero)
        $current -ne [IntPtr]::Zero -and $current -ne $beforeResizeBitmap
    } 'native window did not replace bitmap after RDP resize' 15
    if ($VisiblePixelCheck) {
        Check-VisibleFrame $window $frameControl ([uint32]$process.Id) 0x197153 0x197153
        Write-Output 'RDP 尺寸重新協商後的 9 個螢幕像素符合新影格。'
    }
    Write-Output "視窗尺寸變更：RDP 測試伺服器收到 $expectedLayout。"
    if ($InteractiveClipboard) {
        Wait-For { [RdpFullGui]::IsWindowEnabled((Button '貼上')) -and [RdpFullGui]::OwnsClipboard([uint32]$process.Id) } 'desktop did not acquire the test clipboard' 15
        Click (Button '貼上')
        Wait-For { [IO.File]::Exists((Join-Path $fixtureDir 'clipboard-matched')) -or [IO.File]::Exists((Join-Path $fixtureDir 'clipboard-unexpected')) } 'RDP fixture did not receive clipboard text' 15
        if ([IO.File]::Exists((Join-Path $fixtureDir 'clipboard-unexpected'))) { throw 'RDP fixture received unexpected clipboard text' }
        Click (Button '登出')
        Wait-For { [IO.File]::Exists((Join-Path $fixtureDir 'closed')) } 'RDP connection did not close after logout'
        [void][RdpFullGui]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        if (-not $process.WaitForExit(10000)) { throw 'desktop process failed to exit' }
        $process.Refresh(); if ($process.ExitCode -ne 0) { throw 'desktop process exited abnormally' }
        Write-Output 'Windows system clipboard and local RDP fixture round trip passed.'
        return
    }
    if ([RdpFullGui]::IsWindowEnabled((Button '貼上'))) { throw 'paste button enabled despite disabled paste' }
    if (-not [RdpFullGui]::IsWindowEnabled($inputs[2])) { throw 'input disabled for writable session' }
    if (-not $Nla -and -not [IO.File]::Exists((Join-Path $fixtureDir 'authenticated'))) { throw 'RDP server did not validate credentials' }
    [void][RdpFullGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]122, [IntPtr]::Zero)
    Wait-For { [IO.File]::Exists((Join-Path $fixtureDir 'key')) } 'RDP server did not receive key'
    [RdpFullGui]::SimulateFocusLoss($inputs[2])
    Wait-For { [IO.File]::Exists((Join-Path $fixtureDir 'focus-release')) } 'RDP server did not receive Ctrl+A presses and releases in order'
    [RdpFullGui]::SimulateArrowFocusLoss($inputs[2])
    Wait-For { [IO.File]::Exists((Join-Path $fixtureDir 'focus-arrow-release')) } 'RDP server did not receive left-arrow press and focus-loss release'
    Click (Button '登出')
    Wait-For { [IO.File]::Exists((Join-Path $fixtureDir 'closed')) } 'RDP connection did not close after logout'
    Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
    Click (Button '登入')
    [void][RdpFullGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    Click (Button '編輯')
    [void][RdpFullGui]::SendMessageInt((Button '禁止貼到遠端'), 0x00F1, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][RdpFullGui]::SendMessageInt((Button '唯讀（禁止遠端輸入）'), 0x00F1, [IntPtr]1, [IntPtr]::Zero)
    Click (Button '儲存')
    [void][RdpFullGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    Click (Button '連線')
    Wait-For { @($children | Where-Object { [RdpFullGui]::Class($_) -eq 'Static' -and [RdpFullGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -gt 0 } 'second RDP frame not installed' 25
    if ([RdpFullGui]::IsWindowEnabled((Button '貼上')) -or [RdpFullGui]::IsWindowEnabled($inputs[2])) { throw 'read-only session has enabled input' }
    if (@($children | Where-Object { [RdpFullGui]::Text($_) -eq '已連線（唯讀）；遠端輸入已停用。' }).Count -ne 1) { throw 'read-only status missing' }
    Stop-Process -Id $server.Id -Force
    [void]$server.WaitForExit(10000)
    Wait-For { @($children | Where-Object { [RdpFullGui]::Text($_) -eq '連線已結束或失敗。' }).Count -eq 1 } 'unexpected RDP closure not shown' 15
    if (@($children | Where-Object { [RdpFullGui]::Class($_) -eq 'Static' -and [RdpFullGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -ne 0) { throw 'stale RDP frame remains after server closure' }
    if (-not [RdpFullGui]::HasVisibleStyle((Button '連線')) -or -not [RdpFullGui]::HasVisibleStyle($list) -or [RdpFullGui]::HasVisibleStyle((Button '中斷連線'))) { throw 'connection list not restored after server closure' }
    [void][RdpFullGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    Click (Button '連線')
    Wait-For { @($children | Where-Object { [RdpFullGui]::Text($_) -eq '連線已結束或失敗。' }).Count -eq 1 -and [RdpFullGui]::HasVisibleStyle((Button '連線')) } 'retry against stopped server did not return to connection list' 15
    [void][RdpFullGui]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $process.WaitForExit(10000)) { throw 'desktop process failed to exit' }
    $process.Refresh(); if ($process.ExitCode -ne 0) { throw 'desktop process exited abnormally' }
    Write-Output '原生 RDP 指紋確認、影格、按鍵、登出及意外中斷畫面清除通過。'
} finally {
    foreach ($child in @($process, $server)) { if ($child -and -not $child.HasExited) { Stop-Process -Id $child.Id -Force; [void]$child.WaitForExit(10000) } }
    foreach ($name in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnv[$name]) }
    if ([IO.Directory]::Exists($desktopDir)) {
        [IO.File]::Delete((Join-Path $desktopDir 'connections.json'))
        [IO.File]::Delete((Join-Path $desktopDir 'desktop.lock'))
        [IO.Directory]::Delete($desktopDir)
    }
    if ([IO.Directory]::Exists($fixtureDir)) {
        foreach ($name in @('ready', 'key', 'closed', 'authenticated', 'rejected', 'mouse', 'completed', 'frames', 'layout', 'focus-release', 'focus-arrow-release', 'clipboard-ready', 'clipboard-matched', 'clipboard-unexpected')) { [IO.File]::Delete((Join-Path $fixtureDir $name)) }
        [IO.Directory]::Delete($fixtureDir)
    }
    if ([IO.Directory]::Exists($testDir)) { [IO.Directory]::Delete($testDir) }
}
