param([string]$Binary, [int]$Port, [string]$SyncDir, [int]$SwitchPort, [switch]$RemoteClose, [switch]$Expire)
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
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
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageText(IntPtr h, uint m, IntPtr w, StringBuilder l);
    public static string EditText(IntPtr h) { var s = new StringBuilder(4096); SendMessageText(h, 0x000D, (IntPtr)s.Capacity, s); return s.ToString(); }
    public static string ListText(IntPtr h, int index) { var s = new StringBuilder(4096); SendMessageText(h, 0x0189, (IntPtr)index, s); return s.ToString(); }
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
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
    public static IntPtr FindDialog(uint pid) { IntPtr result = IntPtr.Zero; EnumWindows((h,p) => { uint owner; GetWindowThreadProcessId(h,out owner); if (owner == pid && Class(h) == "#32770") { result=h; return false; } return true; },IntPtr.Zero); return result; }
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
    if ($inputs.Count -ne 17 -or $combos.Count -ne 4) { throw '找不到預期的編輯控制項' }
    function Button([string]$label) { $found = @($children | Where-Object { [TelnetGui]::Text($_) -eq $label }); if ($found.Count -ne 1) { throw "找不到按鈕：$label" }; return $found[0] }
    Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
    if ($env:CRACAMOLE_TEST_DESKTOP_KEYBOARD) {
        [void][TelnetGui]::SendMessageInt($inputs[1], 0x0100, [IntPtr]13, [IntPtr]::Zero)
    } else { Click (Button '登入') }
    if ($env:CRACAMOLE_TEST_TIMEOUT_STATUS -eq '1' -or $env:CRACAMOLE_TEST_NOT_FOUND_STATUS -eq '1') {
        Click (Button '新增')
        $missingHost = $env:CRACAMOLE_TEST_NOT_FOUND_STATUS -eq '1'
        Set-Text $inputs[3] $(if ($missingHost) { '主機名稱測試' } else { '逾時測試' })
        Set-Text $inputs[4] $(if ($missingHost) { 'no-such-host.invalid' } else { '127.0.0.1' })
        Set-Text $inputs[5] ([string]$Port)
        Set-Text $inputs[6] 'fixture'
        Set-Text $inputs[7] 'fixture-secret'
        Click (Button '儲存')
        [void][TelnetGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
        if ($missingHost) {
            Click (Button '核對身分')
            Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '找不到主機，請檢查主機名稱或位址。' }).Count -eq 1 } '身分探測未顯示找不到主機'
        }
        Click (Button '連線')
        $expectedMessage = if ($env:CRACAMOLE_TEST_NOT_FOUND_STATUS -eq '1') { '找不到主機。請檢查主機名稱或位址。' } else { '連線逾時。請檢查主機位址、連接埠及網路。' }
        Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq $expectedMessage }).Count -eq 1 } 'SSH 失敗原因未顯示'
        $expectedStatus = if ($missingHost) { '找不到主機' } else { '連線逾時' }
        if (-not [TelnetGui]::ListText($list, 0).Contains($expectedStatus)) { throw ('status-base64=' + [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([TelnetGui]::ListText($list, 0)))) }
        Click (Button '重新載入')
        if (-not [TelnetGui]::ListText($list, 0).Contains($expectedStatus)) { throw '重新載入後連線失敗狀態消失' }
        [void][TelnetGui]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        if (-not $process.WaitForExit(10000)) { throw '桌面程式未結束' }
        $process.Refresh(); if ($process.ExitCode -ne 0) { throw '桌面程式異常結束' }
        return
    }
    Wait-For { ([TelnetGui]::GetWindowLong($list, -16) -band 0x10000000) -ne 0 -and -not [TelnetGui]::IsWindowEnabled((Button '連線')) } '登入後空連線庫未完成初始化'
    foreach ($label in @('連線', '編輯', '刪除', '核對身分', '複製', '複製設定')) {
        if ([TelnetGui]::IsWindowEnabled((Button $label))) { throw "無連線時操作仍啟用：$label" }
    }
    $libraryOriginal = New-Object TelnetGui+Rect
    $librarySmall = New-Object TelnetGui+Rect
    if (-not [TelnetGui]::GetWindowRect($window, [ref]$libraryOriginal) -or -not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 400, 550, 0x0006) -or -not [TelnetGui]::GetWindowRect($window, [ref]$librarySmall)) { throw '無法縮放連線庫視窗' }
    $libraryRects = @()
    foreach ($label in @('重新載入', '連線', '登出', '新增', '編輯', '刪除', '核對身分', '複製', '收折群組', '展開群組', '樹狀檢視', '複製設定', '匯入設定', '群組更名', '釘選／取消')) {
        $r = New-Object TelnetGui+Rect
        if (-not [TelnetGui]::GetWindowRect((Button $label), [ref]$r) -or $r.Left -lt $librarySmall.Left -or $r.Top -lt $librarySmall.Top -or $r.Right -gt $librarySmall.Right -or $r.Bottom -gt $librarySmall.Bottom) { throw "小視窗連線庫控制項超出邊界：$label" }
        foreach ($other in $libraryRects) {
            if ($r.Left -lt $other.Right -and $r.Right -gt $other.Left -and $r.Top -lt $other.Bottom -and $r.Bottom -gt $other.Top) { throw "小視窗連線庫控制項重疊：$label" }
        }
        $libraryRects += $r
    }
    $groupRect = New-Object TelnetGui+Rect
    if (-not [TelnetGui]::GetWindowRect($combos[3], [ref]$groupRect)) { throw '無法讀取群組選擇器尺寸' }
    foreach ($other in $libraryRects) {
        if ($groupRect.Left -lt $other.Right -and $groupRect.Right -gt $other.Left -and $groupRect.Top -lt $other.Bottom -and $groupRect.Bottom -gt $other.Top) { throw '小視窗群組選擇器與按鈕重疊' }
    }
    Click (Button '新增')
    $editorWindow = New-Object TelnetGui+Rect
    $trustButton = New-Object TelnetGui+Rect
    Wait-For { ([TelnetGui]::GetWindowLong((Button '移除此主機的一次性信任'), -16) -band 0x10000000) -ne 0 -and [TelnetGui]::GetWindowRect($window, [ref]$editorWindow) -and [TelnetGui]::GetWindowRect((Button '移除此主機的一次性信任'), [ref]$trustButton) -and $trustButton.Right -le $editorWindow.Right -and $trustButton.Bottom -le $editorWindow.Bottom } '窄視窗開啟編輯器時信任按鈕不在視窗內'
    if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 400, 400, 0x0006)) { throw '無法嘗試縮小編輯器' }
    Wait-For { [TelnetGui]::GetWindowRect($window, [ref]$editorWindow) -and [TelnetGui]::GetWindowRect((Button '儲存並連線'), [ref]$trustButton) -and $trustButton.Right -le $editorWindow.Right -and $trustButton.Bottom -le $editorWindow.Bottom } '編輯器縮小後儲存按鈕不在視窗內'
    Click (Button '取消')
    if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 400, 550, 0x0006)) { throw '關閉編輯器後無法縮小視窗' }
    Wait-For { [TelnetGui]::GetWindowRect($window, [ref]$editorWindow) -and ($editorWindow.Right - $editorWindow.Left) -eq 400 } '關閉編輯器後視窗仍受最小尺寸限制'
    if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, ($libraryOriginal.Right - $libraryOriginal.Left), ($libraryOriginal.Bottom - $libraryOriginal.Top), 0x0006)) { throw '無法還原連線庫視窗' }
    if ($env:CRACAMOLE_TEST_LIBRARY_RESIZE -eq '1') {
        $before = New-Object TelnetGui+Rect
        $after = New-Object TelnetGui+Rect
        if (-not [TelnetGui]::GetWindowRect($list, [ref]$before)) { throw '無法讀取清單尺寸' }
        if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 900, 700, 0x0006)) { throw '無法調整視窗' }
        Wait-For {
            [TelnetGui]::GetWindowRect($list, [ref]$after) -and
            ($after.Right - $after.Left) -eq 260 -and
            ($after.Bottom - $after.Top) -gt ($before.Bottom - $before.Top)
        } '雙欄模式未保持側欄寬度或擴大清單高度'
        $stage = @($children | Where-Object { [TelnetGui]::Text($_).StartsWith('選擇左側連線並按 Enter') })
        if ($stage.Count -ne 1 -or ([TelnetGui]::GetWindowLong($stage[0], -16) -band 0x10000000) -eq 0) { throw '雙欄工作台指引未顯示' }
        $stageRect = New-Object TelnetGui+Rect
        if (-not [TelnetGui]::GetWindowRect($stage[0], [ref]$stageRect) -or $stageRect.Left -le $after.Right) { throw '工作台指引與側欄重疊' }
        foreach ($label in @('重新載入', '連線', '登出', '新增', '編輯', '刪除', '核對身分', '複製', '釘選／取消')) {
            $actionRect = New-Object TelnetGui+Rect
            if (-not [TelnetGui]::GetWindowRect((Button $label), [ref]$actionRect) -or
                $actionRect.Left -le $after.Right -or $actionRect.Bottom -gt $stageRect.Top) {
                throw "寬視窗管理按鈕未位於右側工作區：$label"
            }
        }
        if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 700, 700, 0x0006)) { throw '無法切換窄工作台' }
        Wait-For {
            ([TelnetGui]::GetWindowLong($stage[0], -16) -band 0x10000000) -eq 0 -and
            [TelnetGui]::GetWindowRect($list, [ref]$after) -and ($after.Right - $after.Left) -gt 260
        } '窄視窗未切換至單欄清單'
        if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 900, 700, 0x0006)) { throw '無法切回寬工作台' }
        Wait-For {
            ([TelnetGui]::GetWindowLong($stage[0], -16) -band 0x10000000) -ne 0 -and
            [TelnetGui]::GetWindowRect($list, [ref]$after) -and ($after.Right - $after.Left) -eq 260
        } '重新放大後未還原雙欄工作台'
    }
    if ($env:CRACAMOLE_TEST_SAVE_CONNECT -eq '1') {
        Set-Text $inputs[14] 'alice@127.0.0.1:2222'
        [void][TelnetGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::EditText($inputs[4]) -eq '127.0.0.1' -and [TelnetGui]::EditText($inputs[5]) -eq '2222' -and [TelnetGui]::EditText($inputs[6]) -eq 'alice' } 'quick endpoint not populated'
        if ([IO.File]::Exists((Join-Path $testDir 'connections.json')) -or [TelnetGui]::EditText($inputs[7]) -ne '') { throw 'quick endpoint unexpectedly saved a connection or secret' }
        Click (Button '取消')
        Set-Text $inputs[14] 'alice:secret@host:22'
        [void][TelnetGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
        Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '主機格式無效，請使用 user@host:port 或 [IPv6]:port。' }).Count -eq 1 } 'invalid quick endpoint was not rejected'
        if (([TelnetGui]::GetWindowLong($inputs[3], -16) -band 0x10000000) -ne 0 -or [IO.File]::Exists((Join-Path $testDir 'connections.json'))) { throw 'invalid quick endpoint opened or saved a connection' }
        Set-Text $inputs[14] ''
        Click (Button '新增')
        Set-Text $inputs[3] '既有連線'
        Set-Text $inputs[4] '127.0.0.1'
        Set-Text $inputs[5] '1'
        Click (Button '儲存')
        Set-Text $inputs[14] 'not-a-real-host'
        Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]::Zero } '搜尋未篩選不符合的連線'
        foreach ($label in @('連線', '編輯', '刪除', '核對身分', '複製', '複製設定')) {
            if ([TelnetGui]::IsWindowEnabled((Button $label))) { throw "搜尋無結果時操作仍啟用：$label" }
        }
        Set-Text $inputs[14] '127.0.0.1'
        Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]1 } '搜尋未顯示符合的連線'
        if (-not [TelnetGui]::IsWindowEnabled((Button '連線'))) { throw '有選取結果時連線仍停用' }
        Set-Text $inputs[14] ''
    }
    if ($SwitchPort) {
        Click (Button '新增')
        Set-Text $inputs[3] '切換目標'
        Set-Text $inputs[13] '機房'
        [void][TelnetGui]::SendMessageInt($combos[0], 0x014E, [IntPtr]1, [IntPtr]::Zero)
        $change = [IntPtr]([TelnetGui]::GetDlgCtrlID($combos[0]) -bor (1 -shl 16))
        [void][TelnetGui]::SendMessageInt($window, 0x0111, $change, $combos[0])
        Set-Text $inputs[4] '127.0.0.1'; Set-Text $inputs[5] ([string]$SwitchPort)
        Set-Text $inputs[7] 'fixture-group-secret'
        Click (Button '儲存')
        $beforeRename = (Get-Content -Raw -Encoding utf8 (Join-Path $testDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value | Where-Object { $_.params.port -eq [string]$SwitchPort }
        if ($beforeRename.params.password -ne 'fixture-group-secret') { throw '更名前未保存測試機密' }
        Click (Button '新增')
        Set-Text $inputs[3] '合併測試暫存'
        Set-Text $inputs[13] '機房暫存'
        Set-Text $inputs[4] '127.0.0.1'
        Set-Text $inputs[5] '1'
        Click (Button '儲存')
        $beforeMerge = (Get-Content -Raw -Encoding utf8 (Join-Path $testDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value | Where-Object { $_.name -eq '機房暫存 / 合併測試暫存' }
        if (-not $beforeMerge.id) { throw '未建立合併目標群組' }
        foreach ($names in @(@('機房', '機房暫存'), @('機房暫存', '機房'))) {
            if ([TelnetGui]::SendMessage($combos[3], 0x014D, [IntPtr](-1), ('群組：' + $names[0])) -eq [IntPtr](-1)) { throw '找不到待更名群組' }
            Click (Button '群組更名')
            Wait-For { [TelnetGui]::EditText($inputs[16]) -eq $names[0] } '群組更名未載入原名稱'
            Set-Text $inputs[16] $names[1]
            [void][TelnetGui]::PostMessage((Button '確認更名'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
            Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } '群組更名確認未顯示'
            [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]6, [IntPtr]::Zero)
            Wait-For { ([TelnetGui]::GetWindowLong($inputs[16], -16) -band 0x10000000) -eq 0 } '群組更名未完成'
            $renamed = (Get-Content -Raw -Encoding utf8 (Join-Path $testDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value | Where-Object { $_.params.port -eq [string]$SwitchPort }
            if ($renamed.id -ne $beforeRename.id -or $renamed.name -ne ($names[1] + ' / 切換目標') -or (ConvertTo-Json $renamed.params -Compress) -ne (ConvertTo-Json $beforeRename.params -Compress)) { throw '群組更名改變連線 ID 或設定' }
            if ($names[1] -eq '機房暫存') {
                $merged = (Get-Content -Raw -Encoding utf8 (Join-Path $testDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value | Where-Object { $_.name -like '機房暫存 / *' }
                if (@($merged).Count -ne 2 -or @($merged | Where-Object { $_.id -eq $beforeMerge.id }).Count -ne 1) { throw '合併後未保留目標群組連線' }
                $existing = $merged | Where-Object { $_.id -eq $beforeMerge.id }
                if ((ConvertTo-Json $existing.params -Compress) -ne (ConvertTo-Json $beforeMerge.params -Compress)) { throw '合併改變目標群組連線設定' }
                Set-Text $inputs[14] '合併測試暫存'
                Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]1 } '找不到合併測試連線'
                [void][TelnetGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
                [void][TelnetGui]::PostMessage((Button '刪除'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
                Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } '刪除合併測試連線未確認'
                [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]6, [IntPtr]::Zero)
                Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]::Zero } '合併測試連線未刪除'
                Set-Text $inputs[14] ''
            }
        }
        if ([TelnetGui]::SendMessage($combos[3], 0x014D, [IntPtr](-1), '群組：機房') -eq [IntPtr](-1)) { throw '找不到待移至未分類的群組' }
        Click (Button '群組更名')
        Set-Text $inputs[16] ''
        [void][TelnetGui]::PostMessage((Button '確認更名'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } '移至未分類確認未顯示'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]6, [IntPtr]::Zero)
        Wait-For { ([TelnetGui]::GetWindowLong($inputs[16], -16) -band 0x10000000) -eq 0 } '移至未分類未完成'
        $ungrouped = (Get-Content -Raw -Encoding utf8 (Join-Path $testDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value | Where-Object { $_.id -eq $beforeRename.id }
        if ($ungrouped.name -ne '切換目標' -or (ConvertTo-Json $ungrouped.params -Compress) -ne (ConvertTo-Json $beforeRename.params -Compress)) { throw '移至未分類改變連線設定或機密' }
        Set-Text $inputs[14] '切換目標'
        Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]1 } '找不到已移至未分類的連線'
        [void][TelnetGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
        Click (Button '編輯')
        Wait-For { ([TelnetGui]::GetWindowLong($inputs[13], -16) -band 0x10000000) -ne 0 } '編輯器未顯示群組欄位'
        Set-Text $inputs[13] '機房'
        Click (Button '儲存')
        Wait-For { ([TelnetGui]::GetWindowLong($inputs[13], -16) -band 0x10000000) -eq 0 } '還原群組未完成'
        $restored = (Get-Content -Raw -Encoding utf8 (Join-Path $testDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value | Where-Object { $_.id -eq $beforeRename.id }
        if ($restored.name -ne '機房 / 切換目標' -or (ConvertTo-Json $restored.params -Compress) -ne (ConvertTo-Json $beforeRename.params -Compress)) { throw '還原群組改變連線設定或機密' }
        Set-Text $inputs[14] ''
        [void][TelnetGui]::SendMessageInt($combos[3], 0x014E, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    Click (Button '新增')
    if ($env:CRACAMOLE_TEST_CONNECT_ONCE -eq '1') {
        Set-Text $inputs[3] '一次性 Telnet'
        [void][TelnetGui]::SendMessageInt($combos[0], 0x014E, [IntPtr]1, [IntPtr]::Zero)
        $change = [IntPtr]([TelnetGui]::GetDlgCtrlID($combos[0]) -bor (1 -shl 16))
        [void][TelnetGui]::SendMessageInt($window, 0x0111, $change, $combos[0])
        Set-Text $inputs[4] '127.0.0.1'
        Set-Text $inputs[5] '0'
        Set-Text $inputs[7] 'temporary-test-secret'
        Click (Button '免保存連線')
        if (([TelnetGui]::GetWindowLong($inputs[3], -16) -band 0x10000000) -eq 0) { throw '無效的一次性連線關閉編輯器' }
        if ([IO.File]::Exists((Join-Path $testDir 'connections.json'))) { throw '無效的一次性連線寫入設定' }
        Set-Text $inputs[5] ([string]$Port)
        Click (Button '免保存連線')
        Wait-For { @($children | Where-Object { [TelnetGui]::Class($_) -eq 'Static' -and [TelnetGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -gt 0 } '一次性 Telnet 畫面未顯示'
        if ([IO.File]::Exists((Join-Path $testDir 'connections.json'))) { throw '一次性連線寫入設定檔' }
        if ([TelnetGui]::EditText($inputs[7]) -ne '') { throw '一次性連線後密碼欄位未清除' }
        [void][TelnetGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]122, [IntPtr]::Zero)
        [void][TelnetGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]0x4e2d, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 300
        Click (Button '登出')
        Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'closed')) } '一次性連線登出後未關閉'
        if ([IO.File]::Exists((Join-Path $testDir 'connections.json'))) { throw '登出後一次性連線被寫入設定檔' }
        [void][TelnetGui]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        if (-not $process.WaitForExit(10000)) { throw '一次性連線測試程式未結束' }
        $process.Refresh(); if ($process.ExitCode -ne 0) { throw '一次性連線測試程式異常結束' }
        return
    }
    if ($env:CRACAMOLE_TEST_DESKTOP_KEYBOARD) {
        Set-Text $inputs[3] '應取消的連線'
        [void][TelnetGui]::SendMessageInt($inputs[3], 0x0100, [IntPtr]27, [IntPtr]::Zero)
        Wait-For { ([TelnetGui]::GetWindowLong($inputs[3], -16) -band 0x10000000) -eq 0 } 'Escape 未關閉編輯器'
        Click (Button '新增')
    }
    if ($env:CRACAMOLE_TEST_SAVE_CONNECT -eq '1') {
        Click (Button '儲存並連線')
        if (([TelnetGui]::GetWindowLong($inputs[3], -16) -band 0x10000000) -eq 0) { throw '無效設定卻關閉編輯器' }
        $savedCount = ((Get-Content -Raw -Encoding utf8 (Join-Path $testDir 'connections.json') | ConvertFrom-Json).PSObject.Properties | Measure-Object).Count
        if ($savedCount -ne 1) { throw '無效設定不應儲存' }
    }
    Set-Text $inputs[3] $(if ($env:CRACAMOLE_TEST_SAVE_CONNECT -eq '1') { '' } else { 'Telnet fixture' })
    [void][TelnetGui]::SendMessageInt($combos[0], 0x014E, [IntPtr]1, [IntPtr]::Zero)
    $selectionChanged = [IntPtr]([TelnetGui]::GetDlgCtrlID($combos[0]) -bor (1 -shl 16))
    [void][TelnetGui]::SendMessageInt($window, 0x0111, $selectionChanged, $combos[0])
    if ($env:CRACAMOLE_TEST_SAVE_CONNECT -eq '1') {
        Set-Text $inputs[4] 'alice@host.example:2222'
        Set-Text $inputs[5] ''
        [void][TelnetGui]::SendMessageInt($inputs[4], 0x0008, [IntPtr]::Zero, [IntPtr]::Zero)
        Set-Text $inputs[5] '3333'
        Start-Sleep -Milliseconds 250
        if ([TelnetGui]::EditText($inputs[5]) -ne '3333') { throw 'delayed endpoint parsing overwrote a newer port edit' }
        Set-Text $inputs[4] 'alice@[2001:db8::1]:2222'
        [void][TelnetGui]::SendMessageInt($inputs[4], 0x0008, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::EditText($inputs[4]) -eq '2001:db8::1' -and [TelnetGui]::EditText($inputs[5]) -eq '2222' -and [TelnetGui]::EditText($inputs[6]) -eq 'alice' } 'IPv6 endpoint was not split into the editor fields'
        Set-Text $inputs[4] 'alice:secret@host:22'
        [void][TelnetGui]::SendMessageInt($inputs[4], 0x0008, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '主機格式無效，請使用 user@host:port 或 [IPv6]:port。' }).Count -eq 1 } 'invalid endpoint warning missing'
        if ([TelnetGui]::EditText($inputs[4]) -ne 'alice:secret@host:22' -or [TelnetGui]::EditText($inputs[5]) -ne '2222' -or [TelnetGui]::EditText($inputs[6]) -ne 'alice') { throw 'invalid endpoint changed editor fields' }
        Set-Text $inputs[7] 'old-telnet-password'
        Set-Text $inputs[4] 'ssh -p 2200 bob@host.example'
        [void][TelnetGui]::SendMessageInt($inputs[4], 0x0008, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::EditText($inputs[4]) -eq 'host.example' -and [TelnetGui]::EditText($inputs[5]) -eq '2200' -and [TelnetGui]::EditText($inputs[6]) -eq 'bob' } 'SSH command was not split into the editor fields'
        if ([TelnetGui]::SendMessageInt($combos[0], 0x0147, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero -or [TelnetGui]::EditText($inputs[7]) -ne '') { throw 'SSH command did not select SSH and clear the previous protocol password' }
        [void][TelnetGui]::SendMessageInt($combos[0], 0x014E, [IntPtr]1, [IntPtr]::Zero)
        [void][TelnetGui]::SendMessageInt($window, 0x0111, $selectionChanged, $combos[0])
        Set-Text $inputs[6] ''
        Set-Text $inputs[4] "127.0.0.1:$Port"
        Set-Text $inputs[5] ''
        [void][TelnetGui]::SendMessageInt($inputs[4], 0x0008, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::EditText($inputs[4]) -eq '127.0.0.1' -and [TelnetGui]::EditText($inputs[5]) -eq [string]$Port } 'leaving the host field did not split the endpoint'
    } else {
        Set-Text $inputs[4] '127.0.0.1'
        Set-Text $inputs[5] ([string]$Port)
    }
    if ($env:CRACAMOLE_TEST_SAVE_CONNECT -eq '1') {
        Click (Button '儲存並連線')
    } elseif ($env:CRACAMOLE_TEST_DESKTOP_KEYBOARD) {
        [void][TelnetGui]::SendMessageInt($inputs[4], 0x0100, [IntPtr]13, [IntPtr]::Zero)
    } else { Click (Button '儲存') }
    if (-not $SwitchPort -and -not $env:CRACAMOLE_TEST_SAVE_CONNECT -and -not $env:CRACAMOLE_TEST_DESKTOP_KEYBOARD) {
        Click (Button '釘選／取消')
        Wait-For { [TelnetGui]::ListText($list, 0).Contains('★ ') } '釘選後清單未顯示標記'
        if (-not [IO.File]::Exists((Join-Path $testDir 'pinned-connections.json'))) { throw '釘選資料未保存' }
        Click (Button '重新載入')
        Wait-For { [TelnetGui]::ListText($list, 0).Contains('★ ') } '重新載入後釘選未保留'
        if (-not $RemoteClose -and -not $Expire) {
            Click (Button '登出')
            Wait-For { ([TelnetGui]::GetWindowLong($inputs[1], -16) -band 0x10000000) -ne 0 } '登出後登入欄位未顯示'
            Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
            Click (Button '登入')
            Wait-For { [TelnetGui]::ListText($list, 0).Contains('★ ') } '重新登入後釘選未保留'
            $tree = @($children | Where-Object { [TelnetGui]::Class($_) -eq 'SysTreeView32' })[0]
            Click (Button '樹狀檢視')
            Wait-For { [TelnetGui]::SendMessageInt($tree, 0x1105, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]4 } '釘選樹狀清單未顯示常用與原群組'
            $root = [TelnetGui]::SendMessageInt($tree, 0x110A, [IntPtr]0, [IntPtr]::Zero)
            $child = [TelnetGui]::SendMessageInt($tree, 0x110A, [IntPtr]4, $root)
            if ($child -eq [IntPtr]::Zero) { throw '常用樹狀群組沒有連線' }
            [void][TelnetGui]::SendMessageInt($tree, 0x110B, [IntPtr]9, $child)
            Wait-For { [TelnetGui]::SendMessageInt($list, 0x0188, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]::Zero } '常用樹狀節點未同步清單選取'
            Click (Button '清單檢視')
        }
        Click (Button '釘選／取消')
        Wait-For { -not [TelnetGui]::ListText($list, 0).Contains('★ ') } '取消釘選後標記仍顯示'
        Click (Button '重新載入')
        Wait-For { -not [TelnetGui]::ListText($list, 0).Contains('★ ') } '重新載入後已取消的釘選又出現'
        if (-not [TelnetGui]::IsWindowEnabled((Button '複製設定'))) { throw '選取連線時複製設定未啟用' }
        [void][TelnetGui]::PostMessage((Button '複製設定'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } '複製設定確認未顯示'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]2, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -eq [IntPtr]::Zero } '取消複製設定後確認未關閉'
        [void][TelnetGui]::PostMessage((Button '複製設定'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } '複製範圍選擇未顯示'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]6, [IntPtr]::Zero)
        Wait-For { $dialog = [TelnetGui]::FindDialog([uint32]$process.Id); $dialog -ne [IntPtr]::Zero -and [TelnetGui]::Text($dialog) -eq '選擇交換格式' } '交換格式選擇未顯示'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]2, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -eq [IntPtr]::Zero } '取消交換格式後確認未關閉'
        [void][TelnetGui]::PostMessage((Button '複製設定'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } '單筆匯出選擇未顯示'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]7, [IntPtr]::Zero)
        Wait-For { $dialog = [TelnetGui]::FindDialog([uint32]$process.Id); $dialog -ne [IntPtr]::Zero -and [TelnetGui]::Text($dialog) -eq '選擇交換格式' } '單筆匯出格式選擇未顯示'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]6, [IntPtr]::Zero)
        Wait-For { ([TelnetGui]::GetWindowLong($inputs[15], -16) -band 0x10000000) -ne 0 } '匯出預覽未顯示'
        if (-not [TelnetGui]::EditText($inputs[15]).Contains('127.0.0.1')) { throw '匯出預覽缺少公開目的地' }
        Click (Button '取消匯出')
        Wait-For { ([TelnetGui]::GetWindowLong($inputs[15], -16) -band 0x10000000) -eq 0 } '取消匯出後預覽未關閉'
    }
    if ($env:CRACAMOLE_TEST_SAVE_CONNECT -eq '1') {
        if (-not [IO.File]::Exists((Join-Path $testDir 'connections.json'))) { throw '儲存並連線未儲存設定' }
        $saved = (Get-Content -Raw -Encoding utf8 (Join-Path $testDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value | Where-Object { $_.params.port -eq [string]$Port }
        if ($saved.name -ne '127.0.0.1' -or $saved.params.hostname -ne '127.0.0.1') { throw '主機位址拆解或自動命名失敗' }
    } elseif ($env:CRACAMOLE_TEST_DESKTOP_KEYBOARD) {
        if ([TelnetGui]::SendMessageInt($list, 0x0188, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero) { throw '連線清單未預設選取首筆' }
        if ($env:CRACAMOLE_TEST_DESKTOP_KEYBOARD -eq 'double') {
            $doubleClick = [IntPtr]([TelnetGui]::GetDlgCtrlID($list) -bor (2 -shl 16))
            [void][TelnetGui]::SendMessageInt($window, 0x0111, $doubleClick, $list)
        } else { [void][TelnetGui]::SendMessageInt($list, 0x0100, [IntPtr]13, [IntPtr]::Zero) }
    } else {
        if ($SwitchPort) {
            Set-Text $inputs[14] '找不到的主機'
            [void][TelnetGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
            [void][TelnetGui]::PostMessage($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
            Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '沒有符合的連線，請調整搜尋條件。' }).Count -eq 1 } '搜尋無結果時未提示'
            Start-Sleep -Milliseconds 200
            if (([TelnetGui]::GetWindowLong($inputs[3], -16) -band 0x10000000) -ne 0) { throw '重複 Enter 意外開啟新增連線編輯器' }
            Set-Text $inputs[14] 'user:secret@host:22'
            [void][TelnetGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
            Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '主機格式無效，請使用 user@host:port 或 [IPv6]:port。' }).Count -eq 1 } '搜尋框未拒絕包含密碼的位址'
            Set-Text $inputs[14] '未分類'
            if ([TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1 -or -not [TelnetGui]::ListText($list, 0).Contains('Telnet fixture')) { throw '未分類群組搜尋錯誤' }
            Set-Text $inputs[14] '機房'
            if ([TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1) { throw '群組搜尋未找到連線' }
            Click (Button '收折群組')
            if ([TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1) { throw '搜尋中收折群組不應隱藏符合項目' }
            Set-Text $inputs[14] ''
            if ([TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1 -or [TelnetGui]::ListText($list, 0).Contains('切換目標')) { throw '清除搜尋後群組未收折' }
            Click (Button '收折群組')
            if ([TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero) { throw '未分類群組未收折' }
            if ([TelnetGui]::SendMessage($combos[3], 0x014D, [IntPtr](-1), '群組：機房') -eq [IntPtr](-1)) { throw '群組選單缺少已收折的機房' }
            Click (Button '展開群組')
            if ([TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1 -or -not [TelnetGui]::ListText($list, 0).Contains('切換目標')) { throw '個別展開機房時影響其他群組' }
            if ([TelnetGui]::SendMessage($combos[3], 0x014D, [IntPtr](-1), '未分類（無群組）') -eq [IntPtr](-1)) { throw '群組選單缺少未分類項目' }
            Click (Button '展開群組')
            if ([TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]2) { throw '展開群組未還原所有連線' }
            [void][TelnetGui]::SendMessageInt($combos[3], 0x014E, [IntPtr]::Zero, [IntPtr]::Zero)
            [void][TelnetGui]::SendMessageInt($list, 0x0186, [IntPtr]1, [IntPtr]::Zero)
            Set-Text $inputs[14] '127.0.0.1'
            if ([TelnetGui]::SendMessageInt($list, 0x0188, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1) { throw '搜尋後未保留所選連線' }
            [void][TelnetGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]0x26, [IntPtr]::Zero)
            if ([TelnetGui]::SendMessageInt($list, 0x0188, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero) { throw '搜尋框向上鍵未選擇上一筆連線' }
            [void][TelnetGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]0x28, [IntPtr]::Zero)
            if ([TelnetGui]::SendMessageInt($list, 0x0188, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1) { throw '搜尋框向下鍵未選擇下一筆連線' }
            Set-Text $inputs[14] 'Telnet fixture'
        }
        [void][TelnetGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
        if ($SwitchPort) { [void][TelnetGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero) }
        else { Click (Button '連線') }
    }
    Wait-For { @($children | Where-Object { [TelnetGui]::Class($_) -eq 'Static' -and [TelnetGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -gt 0 } 'Telnet 畫面未顯示'
    if ($SwitchPort) {
        Set-Text $inputs[14] ''
        $trees = @($children | Where-Object { [TelnetGui]::Class($_) -eq 'SysTreeView32' })
        if ($trees.Count -ne 1) { throw '原生群組樹控制項數量錯誤' }
        $tree = $trees[0]
        Click (Button '樹狀檢視')
        Wait-For { ([TelnetGui]::GetWindowLong($tree, -16) -band 0x10000000) -ne 0 -and [TelnetGui]::SendMessageInt($tree, 0x1105, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]4 } '樹狀檢視未顯示兩個群組與主機'
        Start-Sleep -Milliseconds 600
        $root = [TelnetGui]::SendMessageInt($tree, 0x110A, [IntPtr]0, [IntPtr]::Zero)
        if ($root -eq [IntPtr]::Zero) { throw '群組樹沒有根節點' }
        [void][TelnetGui]::SendMessageInt($tree, 0x110B, [IntPtr]9, $root)
        Wait-For { [TelnetGui]::SendMessageInt($list, 0x0188, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr](-1) } '選擇群組時仍保留主機選取'
        [void][TelnetGui]::SendMessageInt($tree, 0x0100, [IntPtr]13, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 200
        if ([TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero) { throw '群組節點不可觸發主機切換' }
        [void][TelnetGui]::SendMessageInt($tree, 0x0100, [IntPtr]0x25, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]1 } '收折樹節點未同步清單'
        Wait-For { [TelnetGui]::SendMessageInt($tree, 0x1105, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]4 } '收折後群組樹未重建'
        $root = [TelnetGui]::SendMessageInt($tree, 0x110A, [IntPtr]0, [IntPtr]::Zero)
        [void][TelnetGui]::SendMessageInt($tree, 0x110B, [IntPtr]9, $root)
        [void][TelnetGui]::SendMessageInt($tree, 0x0100, [IntPtr]0x27, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]2 } '展開樹節點未同步清單'
        Start-Sleep -Milliseconds 250
        $root = [TelnetGui]::SendMessageInt($tree, 0x110A, [IntPtr]0, [IntPtr]::Zero)
        $child = [TelnetGui]::SendMessageInt($tree, 0x110A, [IntPtr]4, $root)
        if ($child -eq [IntPtr]::Zero) { throw '群組樹缺少主機節點' }
        [void][TelnetGui]::SendMessageInt($tree, 0x110B, [IntPtr]9, $root)
        [void][TelnetGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]0x28, [IntPtr]::Zero)
        if ([TelnetGui]::SendMessageInt($tree, 0x110A, [IntPtr]9, [IntPtr]::Zero) -ne $child) { throw '搜尋框方向鍵未選取樹狀主機節點' }
        Wait-For {
            $selected = [TelnetGui]::SendMessageInt($list, 0x0188, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
            $selected -ge 0 -and [TelnetGui]::ListText($list, $selected).Contains('切換目標')
        } '樹節點選取未同步至切換目標'
        [void][TelnetGui]::SendMessageInt($tree, 0x0100, [IntPtr]13, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } '樹節點切換未顯示確認'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]7, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -eq [IntPtr]::Zero } '樹節點切換取消失敗'
        if ([IO.File]::Exists((Join-Path $SyncDir 'closed'))) { throw '取消樹節點切換中斷了原連線' }
        Click (Button '清單檢視')
        Wait-For { ([TelnetGui]::GetWindowLong($tree, -16) -band 0x10000000) -eq 0 } '切回清單後樹狀檢視仍顯示'
    }
    $heading = @($children | Where-Object { [TelnetGui]::Text($_) -eq 'cracamole Windows 原生介面' })
    if ($heading.Count -ne 1 -or ([TelnetGui]::GetWindowLong($heading[0], -16) -band 0x10000000) -ne 0) { throw '工作階段未收合連線庫標題' }
    if ([TelnetGui]::IsWindowEnabled((Button '上一個連線'))) { throw '沒有上一個連線時按鈕應停用' }
    if ([TelnetGui]::IsWindowEnabled((Button '最近'))) { throw '沒有最近連線時選單應停用' }
    Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -match 'TELNET.*UTF-8.*畫面 \d+×\d+.*剩餘 \d\d:\d\d:\d\d' }).Count -eq 1 } '工作階段狀態缺少協定、編碼、畫面尺寸或剩餘時間'
    Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -match '^已連線 · TELNET · 未加密 · .+' }).Count -eq 1 } '工作階段標頭缺少名稱、協定或未加密標記'
    $sessionHeader = @($children | Where-Object { [TelnetGui]::Text($_) -match '^已連線 · TELNET · 未加密 · .+' })[0]
    Set-Text $inputs[14] 'not-a-real-host'
    Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]::Zero } '工作階段搜尋未篩選連線'
    Set-Text $inputs[14] ''
    Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ge 1 } '清除搜尋後未還原連線'
    Wait-For { @((0..([TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() - 1)) | Where-Object { [TelnetGui]::ListText($list, $_).Contains('目前工作階段（已連線）') }).Count -eq 1 } '搜尋後未保留已連線狀態'
    $rows = [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $marked = @((0..($rows - 1)) | Where-Object { [TelnetGui]::ListText($list, $_).Contains('目前工作階段') })
    if ($marked.Count -ne 1 -or -not [TelnetGui]::ListText($list, $marked[0]).Contains('127.0.0.1')) { throw '目前工作階段標記錯誤' }
    $sessionFrame = @($children | Where-Object { [TelnetGui]::Class($_) -eq 'Static' -and [TelnetGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero })[0]
    if (-not $RemoteClose -and -not $Expire -and -not $SwitchPort -and -not $env:CRACAMOLE_TEST_CONNECT_ONCE -and -not $env:CRACAMOLE_TEST_SAVE_CONNECT -and -not $env:CRACAMOLE_TEST_DESKTOP_KEYBOARD) {
        [void][TelnetGui]::SendMessageInt($list, 0x0186, [IntPtr]$marked[0], [IntPtr]::Zero)
        Click (Button '釘選／取消')
        Wait-For { [TelnetGui]::ListText($list, 0).Contains('★ ') -and [TelnetGui]::ListText($list, 0).Contains('目前工作階段（已連線）') } '連線中釘選未保留工作階段狀態'
        if ([IO.File]::Exists((Join-Path $SyncDir 'closed')) -or [TelnetGui]::SendMessageInt($sessionFrame, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]::Zero) { throw '連線中釘選中斷了工作階段或清除畫布' }
        Click (Button '釘選／取消')
        Wait-For { -not [TelnetGui]::ListText($list, 0).Contains('★ ') -and [TelnetGui]::ListText($list, 0).Contains('目前工作階段（已連線）') } '連線中取消釘選未保留工作階段狀態'
        if ([IO.File]::Exists((Join-Path $SyncDir 'closed'))) { throw '連線中取消釘選中斷了工作階段' }
    }
    $originalWindow = New-Object TelnetGui+Rect
    $smallWindow = New-Object TelnetGui+Rect
    if (-not [TelnetGui]::GetWindowRect($window, [ref]$originalWindow) -or -not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 1400, 700, 0x0006)) { throw '無法調整工作階段寬視窗' }
    $headerRect = New-Object TelnetGui+Rect
    if (-not [TelnetGui]::GetWindowRect($sessionHeader, [ref]$headerRect)) { throw '無法取得工作階段標頭' }
    foreach ($label in @('中斷連線', '貼上', '全螢幕', '登出', '收折側欄', '上一個連線', '最近')) {
        $rect = New-Object TelnetGui+Rect
        if (-not [TelnetGui]::GetWindowRect((Button $label), [ref]$rect) -or [Math]::Abs($rect.Top - $headerRect.Top) -gt 2 -or $rect.Left -lt $headerRect.Right) { throw "寬視窗工具列未與標頭同列：$label" }
    }
    if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 400, 550, 0x0006) -or -not [TelnetGui]::GetWindowRect($window, [ref]$smallWindow)) { throw '無法調整工作階段窄視窗' }
    Wait-For { ([TelnetGui]::GetWindowLong($sessionHeader, -16) -band 0x10000000) -eq 0 } '窄視窗仍顯示工作階段標頭'
    if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 900, 700, 0x0006)) { throw '無法調整工作階段寬視窗' }
    Wait-For { ([TelnetGui]::GetWindowLong($sessionHeader, -16) -band 0x10000000) -ne 0 } '寬視窗未顯示工作階段標頭'
    if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 400, 550, 0x0006)) { throw '無法還原工作階段窄視窗' }
    $buttonRects = @()
    foreach ($label in @('中斷連線', '貼上', '全螢幕', '登出', '收折側欄', '更多', '收折群組', '展開群組', '樹狀檢視', '釘選／取消')) {
        $r = New-Object TelnetGui+Rect
        if (-not [TelnetGui]::GetWindowRect((Button $label), [ref]$r) -or $r.Left -lt $smallWindow.Left -or $r.Top -lt $smallWindow.Top -or $r.Right -gt $smallWindow.Right -or $r.Bottom -gt $smallWindow.Bottom) { throw "小視窗控制項超出視窗：$label" }
        foreach ($other in $buttonRects) {
            if ($r.Left -lt $other.Right -and $r.Right -gt $other.Left -and $r.Top -lt $other.Bottom -and $r.Bottom -gt $other.Top) { throw "小視窗控制項重疊：$label" }
        }
        $buttonRects += $r
    }
    $smallFrame = New-Object TelnetGui+Rect
    if (-not [TelnetGui]::GetWindowRect($sessionFrame, [ref]$smallFrame)) { throw '無法讀取遠端畫布位置' }
    if (($smallFrame.Top - $smallWindow.Top) -gt 240) { throw '工作階段頂部留白過大' }
    foreach ($label in @('中斷連線', '貼上', '全螢幕', '登出', '收折側欄', '更多')) {
        $r = New-Object TelnetGui+Rect
        if (-not [TelnetGui]::GetWindowRect((Button $label), [ref]$r) -or $r.Bottom -gt $smallFrame.Top) { throw "工作階段工具列不在畫布上方：$label" }
    }
    $footer = @($children | Where-Object { [TelnetGui]::Text($_) -match 'TELNET.*畫面 .*剩餘 ' })
    $footerRect = New-Object TelnetGui+Rect
    if ($footer.Count -ne 1 -or -not [TelnetGui]::GetWindowRect($footer[0], [ref]$footerRect) -or $footerRect.Top -lt $smallFrame.Bottom -or $footerRect.Bottom -gt $smallWindow.Bottom) { throw '工作階段狀態列未位於畫布下方' }
    if (-not [TelnetGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, ($originalWindow.Right - $originalWindow.Left), ($originalWindow.Bottom - $originalWindow.Top), 0x0006)) { throw '無法還原工作階段視窗尺寸' }
    $expandedFrame = New-Object TelnetGui+Rect
    $collapsedFrame = New-Object TelnetGui+Rect
    if (([TelnetGui]::GetWindowLong($list, -16) -band 0x10000000) -eq 0 -or -not [TelnetGui]::GetWindowRect($sessionFrame, [ref]$expandedFrame)) { throw '工作階段未顯示連線側欄' }
    $clientRect = New-Object TelnetGui+Rect
    if (-not [TelnetGui]::GetClientRect($window, [ref]$clientRect) -or
        ($expandedFrame.Right - $expandedFrame.Left) -ne ($clientRect.Right - $clientRect.Left)) {
        throw '遠端畫布未填滿視窗可用寬度'
    }
    Click (Button '收折側欄')
    Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '展開側欄' }).Count -eq 1 } '收折後側欄按鈕未更新'
    if (([TelnetGui]::GetWindowLong($list, -16) -band 0x10000000) -ne 0 -or -not [TelnetGui]::GetWindowRect($sessionFrame, [ref]$collapsedFrame) -or
        $collapsedFrame.Left -ne $expandedFrame.Left -or $collapsedFrame.Top -ne $expandedFrame.Top -or
        $collapsedFrame.Right -ne $expandedFrame.Right -or $collapsedFrame.Bottom -ne $expandedFrame.Bottom) { throw '收折側欄不應變更遠端畫布尺寸' }
    if (([TelnetGui]::GetWindowLong($combos[3], -16) -band 0x10000000) -ne 0) { throw '收折側欄後群組選擇器仍顯示' }
    Click (Button '展開側欄')
    if (([TelnetGui]::GetWindowLong($list, -16) -band 0x10000000) -eq 0) { throw '無法重新展開側欄' }
    if (([TelnetGui]::GetWindowLong($combos[3], -16) -band 0x10000000) -eq 0) { throw '展開側欄後群組選擇器未顯示' }
    [void][TelnetGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]0x1b, [IntPtr]::Zero)
    Wait-For { ([TelnetGui]::GetWindowLong($list, -16) -band 0x10000000) -eq 0 } 'Escape 未收折工作階段側欄'
    Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '展開側欄' }).Count -eq 1 } 'Escape 收折後側欄按鈕未更新'
    Click (Button '展開側欄')
    Wait-For { ([TelnetGui]::GetWindowLong($list, -16) -band 0x10000000) -ne 0 } 'Escape 收折後無法重新展開側欄'
    if ($env:CRACAMOLE_TEST_VISIBLE_TELNET -eq '1') {
        $frame = @($children | Where-Object { [TelnetGui]::Class($_) -eq 'Static' -and [TelnetGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero })[0]
        Click (Button '收折側欄')
        try {
            [void][TelnetGui]::ShowWindow($window, 4) # SW_SHOWNOACTIVATE
            if (-not [TelnetGui]::SetWindowPos($window, [IntPtr](-1), 0, 0, 0, 0, 0x0011)) { throw '無法顯示測試視窗' }
            try { Wait-For { [TelnetGui]::VisibleTerminalInk($frame, [uint32]$process.Id) } '未偵測到 Telnet 文字及背景' }
            catch { throw "未偵測到 Telnet 文字及背景：$([TelnetGui]::PixelDiagnostic)" }
            Write-Output '已確認 Windows 螢幕上的 Telnet 測試文字與背景。'
        } finally { [void][TelnetGui]::ShowWindow($window, 0) }
        Click (Button '展開側欄')
    }
    [void][TelnetGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]122, [IntPtr]::Zero)
    [void][TelnetGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]0x4e2d, [IntPtr]::Zero)
    if ($SwitchPort) {
        Set-Text $inputs[14] '切換目標'
        [void][TelnetGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
        $doubleClick = [IntPtr]([TelnetGui]::GetDlgCtrlID($list) -bor (2 -shl 16))
        [void][TelnetGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } '切換確認對話框未顯示'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]7, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -eq [IntPtr]::Zero } '取消後確認對話框未關閉'
        if ([IO.File]::Exists((Join-Path $SyncDir 'closed')) -or [IO.File]::Exists((Join-Path $SyncDir 'second-connected'))) { throw '取消切換後工作階段已改變' }
        [void][TelnetGui]::PostMessage($window, 0x0111, $doubleClick, $list)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } '再次切換未顯示確認對話框'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]6, [IntPtr]::Zero)
        Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'closed')) -and [IO.File]::Exists((Join-Path $SyncDir 'second-connected')) } '確認後未切換至第二個工作階段'
        Wait-For { [TelnetGui]::ListText($list, 0).Contains('目前工作階段') -and [TelnetGui]::ListText($list, 0).Contains('切換目標') } '切換後未更新工作階段標記'
        Click (Button '收折群組')
        Set-Text $inputs[14] ''
        if ([TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]1 -or [TelnetGui]::ListText($list, 0).Contains('切換目標')) { throw 'active group did not collapse' }
        Click (Button '展開群組')
        $rows = [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if (@((0..($rows - 1)) | Where-Object { [TelnetGui]::ListText($list, $_).Contains('目前工作階段') -and [TelnetGui]::ListText($list, $_).Contains('切換目標') }).Count -ne 1) { throw 'expanding lost active session marker' }
        Wait-For { [TelnetGui]::IsWindowEnabled((Button '上一個連線')) } 'previous connection button not enabled'
        if (-not [TelnetGui]::IsWindowEnabled((Button '最近'))) { throw '最近連線選單未啟用' }
        [void][TelnetGui]::PostMessage((Button '上一個連線'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } 'recent connection confirmation missing'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]6, [IntPtr]::Zero)
        Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'second-closed')) -and [IO.File]::Exists((Join-Path $SyncDir 'returned-connected')) } 'recent connection did not return to first server'
        Wait-For { @((0..([TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() - 1)) | Where-Object { [TelnetGui]::ListText($list, $_).Contains('目前工作階段（已連線）') -and [TelnetGui]::ListText($list, $_).Contains('Telnet fixture') }).Count -eq 1 } '返回主機尚未完成連線'
        Click (Button '中斷連線')
        Wait-For { ([TelnetGui]::GetWindowLong($heading[0], -16) -band 0x10000000) -ne 0 } '中斷後未返回連線庫'
        $rows = [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if (@((0..($rows - 1)) | Where-Object { [TelnetGui]::ListText($list, $_).Contains('Telnet fixture') -and [TelnetGui]::ListText($list, $_).Contains('已中斷') }).Count -ne 1) { throw '手動中斷狀態未保留' }
        Click (Button '重新載入')
        Wait-For { [TelnetGui]::ListText($list, 0).Contains('Telnet fixture') } '重新載入後最近使用的連線未排序在前'
        if (-not [TelnetGui]::ListText($list, 0).Contains('已中斷')) { throw '重新載入後手動中斷狀態消失' }
        if (-not [IO.File]::Exists((Join-Path $testDir 'recent-connections.json'))) { throw '最近連線資料未保存' }
        Click (Button '登出')
        Wait-For { ([TelnetGui]::GetWindowLong($inputs[1], -16) -band 0x10000000) -ne 0 } '最近連線測試登出後未顯示登入欄位'
        Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
        Click (Button '登入')
        try {
            Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]2 -and [TelnetGui]::ListText($list, 0).Contains('Telnet fixture') } '重新登入後最近連線排序未保留'
        } catch {
            $count = [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
            $row = if ($count -gt 0) { [TelnetGui]::ListText($list, 0) } else { '' }
            throw "重新登入後最近連線排序未保留：筆數=$count、首列=$row、搜尋=$([TelnetGui]::EditText($inputs[14]))"
        }
        Set-Text $inputs[14] 'Telnet fixture'
        Set-Text $inputs[14] '切換目標'
        [void][TelnetGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
        [void][TelnetGui]::PostMessage((Button '刪除'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::FindDialog([uint32]$process.Id) -ne [IntPtr]::Zero } '刪除確認對話框未顯示'
        [void][TelnetGui]::PostMessage([TelnetGui]::FindDialog([uint32]$process.Id), 0x0111, [IntPtr]6, [IntPtr]::Zero)
        Wait-For { [TelnetGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]::Zero } '刪除後連線仍在清單中'
        Set-Text $inputs[14] 'Telnet fixture'
        [void][TelnetGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
        Click (Button '連線')
        Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'reconnected-after-delete')) } '刪除最近連線後無法重新連線'
        Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -match 'TELNET.*畫面 \d+×\d+' }).Count -eq 1 } '重新連線後未顯示畫面狀態'
        if ([TelnetGui]::IsWindowEnabled((Button '上一個連線')) -or [TelnetGui]::IsWindowEnabled((Button '最近'))) { throw '已刪除的連線仍使最近連線按鈕啟用' }
    }
    if ($Expire) {
        Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '已登出，請重新登入。' }).Count -eq 1 } '工作階段未到期'
        Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'closed')) } '工作階段到期後 Telnet 連線未正常關閉'
        if (@($children | Where-Object { [TelnetGui]::Class($_) -eq 'Static' -and [TelnetGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -ne 0) { throw '工作階段到期後仍顯示舊影格' }
    } elseif ($RemoteClose) {
        Wait-For { @($children | Where-Object { [TelnetGui]::Text($_) -eq '連線已結束或失敗。' }).Count -eq 1 } '遠端關閉後狀態未更新'
        if (([TelnetGui]::GetWindowLong($heading[0], -16) -band 0x10000000) -eq 0) { throw '遠端關閉後未還原連線庫標題' }
        if ([TelnetGui]::ListText($list, 0).Contains('目前工作階段')) { throw '遠端關閉後仍顯示工作階段標記' }
        if (-not [TelnetGui]::ListText($list, 0).Contains('連線失敗或異常中斷')) { throw '遠端關閉後清單未保留異常中斷狀態' }
        Click (Button '重新載入')
        if (-not [TelnetGui]::ListText($list, 0).Contains('連線失敗或異常中斷')) { throw '重新載入後異常中斷狀態消失' }
        if (@($children | Where-Object { [TelnetGui]::Class($_) -eq 'Static' -and [TelnetGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -ne 0) { throw '遠端關閉後仍顯示舊影格' }
    } else {
        Start-Sleep -Milliseconds 500
        Click (Button '登出')
        Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'closed')) } '登出後 Telnet 連線未正常關閉'
    }
    if (@($children | Where-Object { [TelnetGui]::Text($_) -match '畫面 .*剩餘 ' -and ([TelnetGui]::GetWindowLong($_, -16) -band 0x10000000) -ne 0 }).Count -ne 0) { throw '工作階段結束後仍顯示舊狀態' }
    [void][TelnetGui]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $process.WaitForExit(10000)) { throw '桌面程式未結束' }
    $process.Refresh(); if ($process.ExitCode -ne 0) { throw '桌面程式異常結束' }
} finally {
    if ($process -and -not $process.HasExited) { Stop-Process -Id $process.Id -Force; [void]$process.WaitForExit(10000) }
    if ([IO.Directory]::Exists($testDir)) {
        [IO.File]::Delete((Join-Path $testDir 'connections.json'))
        [IO.File]::Delete((Join-Path $testDir 'pinned-connections.json'))
        [IO.File]::Delete((Join-Path $testDir 'recent-connections.json'))
        [IO.File]::Delete((Join-Path $testDir 'desktop.lock'))
        [IO.Directory]::Delete($testDir)
    }
}
