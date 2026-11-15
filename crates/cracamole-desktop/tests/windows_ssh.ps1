param([string]$Binary, [int]$Port, [string]$Fingerprint, [string]$PublicKey, [string]$SyncDir, [string]$MismatchedKey, [string]$PrivateKeyPath, [string]$EncodingName)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class SshGui {
    public static string PixelDiagnostic = "not sampled";
    public delegate bool Callback(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Callback c, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, Callback c, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", EntryPoint="GetWindowLongW")] public static extern int GetWindowLong(IntPtr h, int index);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, string s);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] public static extern IntPtr SendMessageInt(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageText(IntPtr h, uint msg, IntPtr w, StringBuilder l);
    public static string EditText(IntPtr h) { var s = new StringBuilder(4096); SendMessageText(h, 0x000D, (IntPtr)s.Capacity, s); return s.ToString(); }
    public static string ListText(IntPtr h, int index) { var s = new StringBuilder(4096); SendMessageText(h, 0x0189, (IntPtr)index, s); return s.ToString(); }
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int command);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(Point point);
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
    public static int Width(IntPtr h) { Rect r; return GetClientRect(h, out r) ? r.Right - r.Left : 0; }
    public static IntPtr Find(uint pid, string cls) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((h,p) => { uint owner; GetWindowThreadProcessId(h, out owner); if (owner == pid && (String.IsNullOrEmpty(cls) ? Text(h).StartsWith("cracamole — Windows") : Class(h) == cls)) { result = h; return false; } return true; }, IntPtr.Zero);
        return result;
    }
    public static IntPtr[] Children(IntPtr parent) {
        var result = new List<IntPtr>(); EnumChildWindows(parent, (h,p) => { result.Add(h); return true; }, IntPtr.Zero); return result.ToArray();
    }
    public static string Text(IntPtr h) { var s = new StringBuilder(4096); GetWindowText(h,s,s.Capacity); return s.ToString(); }
    public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassName(h,s,s.Capacity); return s.ToString(); }
}
'@
function Wait-For([scriptblock]$Condition, [string]$Message, [int]$Seconds = 10) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (-not (& $Condition)) {
        if ([DateTime]::UtcNow -ge $deadline) { throw $Message }
        Start-Sleep -Milliseconds 50
    }
}
function Click([IntPtr]$Handle) { [void][SshGui]::SendMessage($Handle, 0x00F5, [IntPtr]::Zero, $null) }
function Set-Text([IntPtr]$Handle, [string]$Value) { [void][SshGui]::SendMessage($Handle, 0x000C, [IntPtr]::Zero, $Value) }
function Select-First([IntPtr]$Handle) { [void][SshGui]::SendMessageInt($Handle, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero) }
$savedEnv = @{}
foreach ($name in @('CRACAMOLE_ADMIN_USERNAME', 'CRACAMOLE_ADMIN_PASSWORD', 'CRACAMOLE_DATA_DIR', 'CRACAMOLE_DESKTOP_DATA_DIR', 'CRACAMOLE_DESKTOP_SESSION_MAX_SECS')) { $savedEnv[$name] = [Environment]::GetEnvironmentVariable($name) }
$env:CRACAMOLE_ADMIN_USERNAME = 'test-admin'
$env:CRACAMOLE_ADMIN_PASSWORD = 'desktop-test-password-123'
$env:CRACAMOLE_DESKTOP_SESSION_MAX_SECS = '60'
$testDir = Join-Path ([IO.Path]::GetTempPath()) ('cracamole-ssh-gui-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testDir)
$desktopDir = Join-Path $testDir 'desktop'
$env:CRACAMOLE_DESKTOP_DATA_DIR = $desktopDir
$process = $null
try {
    $process = Start-Process -FilePath $Binary -WindowStyle Hidden -PassThru
    Wait-For { [SshGui]::Find([uint32]$process.Id, $null) -ne [IntPtr]::Zero } 'window not found' 15
    $window = [SshGui]::Find([uint32]$process.Id, $null)
    if (-not $process.WaitForInputIdle(10000)) { throw '原生控制項初始化逾時' }
    $children = [SshGui]::Children($window)
    $inputs = @($children | Where-Object { [SshGui]::Class($_) -eq 'Edit' })
    if ($inputs.Count -ne 17) { throw '找不到預期的輸入控制項' }
    $combos = @($children | Where-Object { [SshGui]::Class($_) -eq 'ComboBox' })
    if ($combos.Count -ne 4) { throw '找不到文字編碼控制項' }
    function Button([string]$Text) {
        $buttons = @($children | Where-Object { [SshGui]::Text($_) -eq $Text })
        if ($buttons.Count -ne 1) { throw "找不到按鈕：$Text" }
        return $buttons[0]
    }
    $list = @($children | Where-Object { [SshGui]::Class($_) -eq 'ListBox' })[0]
    Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
    Click (Button '登入')
    Click (Button '新增')
    Set-Text $inputs[3] '一次性未知 SSH'
    Set-Text $inputs[4] '127.0.0.1'
    Set-Text $inputs[5] ([string]$Port)
    Set-Text $inputs[6] 'alice'
    if ($PrivateKeyPath) { Set-Text $inputs[8] $PrivateKeyPath; Set-Text $inputs[9] 'fixture-passphrase' }
    else { Set-Text $inputs[7] 'secret' }
    Click (Button '免保存連線')
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } '一次性 SSH 未顯示指紋核對'
    $onceDialog = [SshGui]::Find([uint32]$process.Id, '#32770')
    Wait-For { (([SshGui]::Children($onceDialog) | ForEach-Object { [SshGui]::Text($_) }) -join "`n").Contains($Fingerprint) } '一次性 SSH 指紋不符'
    [void][SshGui]::PostMessage($onceDialog, 0x0111, [IntPtr]7, [IntPtr]::Zero)
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -eq [IntPtr]::Zero } '一次性 SSH 拒絕核對後對話框未關閉'
    if ([IO.File]::Exists((Join-Path $desktopDir 'connections.json')) -or [IO.File]::Exists((Join-Path $desktopDir 'ssh-identities.json'))) { throw '拒絕一次性 SSH 後寫入資料' }
    Click (Button '免保存連線')
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } '變更目的地測試未顯示指紋核對'
    Set-Text $inputs[4] 'localhost'
    [void][SshGui]::PostMessage([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0111, [IntPtr]6, [IntPtr]::Zero)
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -eq [IntPtr]::Zero } '變更目的地測試未關閉核對視窗'
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '連線設定已變更，請重新核對。' }).Count -eq 1 } '變更目的地後未拒絕過期的指紋'
    if ([IO.File]::Exists((Join-Path $desktopDir 'ssh-identities.json'))) { throw '目的地變更後保存了過期指紋' }
    Set-Text $inputs[4] '127.0.0.1'
    [IO.File]::WriteAllText((Join-Path $SyncDir 'once-untrusted-rejected'), '')
    Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'continue-once')) } 'coordinator did not continue one-time SSH'
    Click (Button '免保存連線')
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } '一次性 SSH 再次核對未顯示'
    [void][SshGui]::PostMessage([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0111, [IntPtr]6, [IntPtr]::Zero)
    Wait-For { @($children | Where-Object { [SshGui]::Class($_) -eq 'Static' -and [SshGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -gt 0 } '核對後一次性 SSH 未連線' 15
    if ([IO.File]::Exists((Join-Path $desktopDir 'connections.json')) -or -not [IO.File]::Exists((Join-Path $desktopDir 'ssh-identities.json'))) { throw '一次性 SSH 信任資料保存位置錯誤' }
    $identityText = [IO.File]::ReadAllText((Join-Path $desktopDir 'ssh-identities.json'))
    if (-not $identityText.Contains($PublicKey) -or $identityText.Contains('fixture-passphrase') -or $identityText.Contains('secret')) { throw '一次性 SSH 信任資料包含錯誤內容' }
    if ([SshGui]::EditText($inputs[7]) -ne '' -or [SshGui]::EditText($inputs[8]) -ne '' -or [SshGui]::EditText($inputs[9]) -ne '') { throw '一次性 SSH 後未清除機密欄位' }
    [IO.File]::WriteAllText((Join-Path $SyncDir 'once-trusted'), '')
    Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'continue-after-once')) } 'coordinator did not continue saved SSH'
    Click (Button '中斷連線')
    Click (Button '新增')
    Set-Text $inputs[4] '127.0.0.1'; Set-Text $inputs[5] ([string]$Port)
    [void][SshGui]::PostMessage((Button '移除此主機的一次性信任'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } '未顯示信任移除確認'
    if (([SshGui]::SendMessageInt([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0400, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -band 0xffff) -ne 7) { throw '信任移除確認未預設取消' }
    [void][SshGui]::PostMessage([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0111, [IntPtr]7, [IntPtr]::Zero)
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -eq [IntPtr]::Zero } '取消信任移除後對話框未關閉'
    if (-not [IO.File]::ReadAllText((Join-Path $desktopDir 'ssh-identities.json')).Contains($PublicKey)) { throw '取消後信任遭移除' }
    [void][SshGui]::PostMessage((Button '移除此主機的一次性信任'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } '未顯示第二次信任移除確認'
    Set-Text $inputs[4] 'localhost'
    [void][SshGui]::PostMessage([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0111, [IntPtr]6, [IntPtr]::Zero)
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '連線設定已變更，未移除 SSH 信任。請重新確認。' }).Count -eq 1 } '目的地變更後未拒絕移除'
    if (-not [IO.File]::ReadAllText((Join-Path $desktopDir 'ssh-identities.json')).Contains($PublicKey)) { throw '目的地變更後移除了信任' }
    Set-Text $inputs[4] '127.0.0.1'
    [void][SshGui]::PostMessage((Button '移除此主機的一次性信任'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } '未顯示第三次信任移除確認'
    [void][SshGui]::PostMessage([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0111, [IntPtr]6, [IntPtr]::Zero)
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '已移除此目的地的一次性 SSH 信任；其他信任來源不受影響。' }).Count -eq 1 } '未完成信任移除'
    if ([IO.File]::ReadAllText((Join-Path $desktopDir 'ssh-identities.json')).Contains($PublicKey)) { throw '信任未移除' }
    if ($env:CRACAMOLE_TEST_FILE_DIALOG -eq '1') {
        Set-Text $inputs[8] 'C:\fixture\unchanged-key'
        [void][SshGui]::PostMessage((Button '瀏覽…'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } 'file dialog not found'
        $fileDialog = [SshGui]::Find([uint32]$process.Id, '#32770')
        [void][SshGui]::PostMessage($fileDialog, 0x0111, [IntPtr]2, [IntPtr]::Zero)
        Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -eq [IntPtr]::Zero } 'file dialog did not close'
        if ([SshGui]::EditText($inputs[8]) -ne 'C:\fixture\unchanged-key') { throw 'cancel changed private key path' }
        Set-Text $inputs[8] ''
    }
    Set-Text $inputs[3] 'SSH fixture'
    if ($EncodingName -eq 'Big5') { [void][SshGui]::SendMessageInt($combos[2], 0x014E, [IntPtr]1, [IntPtr]::Zero) }
    Set-Text $inputs[4] '127.0.0.1'
    Set-Text $inputs[5] ([string]$Port)
    Set-Text $inputs[6] 'alice'
    if ($PrivateKeyPath) {
        Set-Text $inputs[8] $PrivateKeyPath
        Set-Text $inputs[9] 'fixture-passphrase'
    } else { Set-Text $inputs[7] 'secret' }
    Click (Button '儲存')
    Wait-For { [SshGui]::ListText($list, 0).Contains('未設定主機金鑰') } '未設定 SSH 主機金鑰的提示未顯示'
    Select-First $list
    Click (Button '收折群組')
    if ([SshGui]::SendMessageInt($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero) { throw 'SSH group did not collapse' }
    Set-Text $inputs[14] "alice@127.0.0.1:$Port"
    [void][SshGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '遠端連線錯誤。請檢查連線設定；SSH 請使用「核對身分」確認主機金鑰。' }).Count -eq 1 } 'untrusted SSH connection was not rejected'
    Click (Button '中斷連線')
    [IO.File]::WriteAllText((Join-Path $SyncDir 'untrusted-rejected'), '')
    Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'continue')) } 'test coordinator did not continue'
    Wait-For { [SshGui]::SendMessageInt($list, 0x0188, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]::Zero -and [SshGui]::IsWindowEnabled((Button '核對身分')) } '連線失敗後無法直接核對所選主機'
    $verify = Button '核對身分'
    foreach ($decision in @(7, 6)) {
        [void][SshGui]::PostMessage($verify, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } 'fingerprint dialog not found'
        $dialog = [SshGui]::Find([uint32]$process.Id, '#32770')
        Wait-For { (([SshGui]::Children($dialog) | ForEach-Object { [SshGui]::Text($_) }) -join "`n").Contains($Fingerprint) } 'fingerprint dialog did not show expected fingerprint'
        $dialogText = ([SshGui]::Children($dialog) | ForEach-Object { [SshGui]::Text($_) }) -join "`n"
        if (-not $dialogText.Contains($Fingerprint)) { throw '確認視窗的 SSH 指紋不符' }
        [void][SshGui]::PostMessage($dialog, 0x0111, [IntPtr]$decision, [IntPtr]::Zero)
        Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -eq [IntPtr]::Zero } '指紋確認視窗未關閉'
        if ($decision -eq 7) {
            $saved = (Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value
            if ($saved.params.'host-key') { throw '拒絕指紋後仍保存主機金鑰' }
        }
    }
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '已儲存主機身分。' }).Count -eq 1 } 'identity not saved'
    Wait-For { [SshGui]::ListText($list, 0).Contains('已信任') } 'trusted identity not reflected in connection list'
    $saved = (Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value
    if ($saved.params.'host-key' -ne $PublicKey) { throw '主機金鑰保存錯誤' }
    if ($saved.params.'character-encoding' -ne $EncodingName) { throw 'SSH 文字編碼保存錯誤' }
    if ($PrivateKeyPath) {
        if ($saved.params.'private-key' -ne [IO.File]::ReadAllText($PrivateKeyPath) -or $saved.params.passphrase -ne 'fixture-passphrase') { throw '私鑰或密語保存錯誤' }
    } elseif ($saved.params.password -ne 'secret') { throw '密碼保存錯誤' }
    if (-not $PrivateKeyPath) {
        Set-Text $inputs[14] 'SSH fixture'
        Select-First $list
        Click (Button '編輯')
        Set-Text $inputs[7] 'wrong-saved-password'
        Click (Button '儲存')
        Select-First $list
        Click (Button '連線')
        Wait-For { [SshGui]::ListText($list, 0).Contains('驗證失敗') } '錯誤密碼未顯示驗證失敗狀態' 15
        Click (Button '重新載入')
        if (-not [SshGui]::ListText($list, 0).Contains('驗證失敗')) { throw '重新載入後驗證失敗狀態消失' }
        Select-First $list
        Click (Button '編輯')
        Set-Text $inputs[7] 'secret'
        Click (Button '儲存')
    }
    $beforeOnce = [IO.File]::ReadAllBytes((Join-Path $desktopDir 'connections.json'))
    if (-not $PrivateKeyPath) {
        Click (Button '新增')
        Set-Text $inputs[3] '錯誤密碼的一次性 SSH'
        Set-Text $inputs[4] '127.0.0.1'; Set-Text $inputs[5] ([string]$Port); Set-Text $inputs[6] 'alice'
        Set-Text $inputs[7] 'wrong-temporary-password'
        Click (Button '免保存連線')
        Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '驗證失敗。請檢查遠端帳號及驗證設定。' }).Count -eq 1 } '一次性 SSH 錯誤密碼未被拒絕'
    }
    Click (Button '新增')
    Set-Text $inputs[3] '一次性 SSH'
    Set-Text $inputs[4] '127.0.0.1'; Set-Text $inputs[5] ([string]$Port); Set-Text $inputs[6] 'alice'
    if ($PrivateKeyPath) {
        Set-Text $inputs[8] $PrivateKeyPath; Set-Text $inputs[9] 'fixture-passphrase'
    } else { Set-Text $inputs[7] 'secret' }
    Click (Button '免保存連線')
    Wait-For { @($children | Where-Object { [SshGui]::Class($_) -eq 'Static' -and [SshGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -gt 0 } '已信任的一次性 SSH 未連線' 15
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $desktopDir 'connections.json'))) -ne [Convert]::ToBase64String($beforeOnce)) { throw '一次性 SSH 修改了已保存設定' }
    if ([SshGui]::EditText($inputs[7]) -ne '' -or [SshGui]::EditText($inputs[8]) -ne '' -or [SshGui]::EditText($inputs[9]) -ne '') { throw '一次性 SSH 後機密欄位未清除' }
    Click (Button '中斷連線')
    Set-Text $inputs[14] 'SSH fixture'
    Select-First $list
    Click (Button '複製')
    Set-Text $inputs[3] 'SSH 同目的地'
    Click (Button '儲存')
    $copies = @((Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value)
    $same = @($copies | Where-Object { $_.name -eq 'SSH 同目的地' })
    if ($same.Count -ne 1 -or $same[0].params.'host-key' -ne $PublicKey -or $same[0].params.password -or $same[0].params.'private-key' -or $same[0].params.passphrase) { throw 'same-host duplicate did not preserve identity without secrets' }
    Set-Text $inputs[14] 'SSH fixture'
    Select-First $list
    Click (Button '複製')
    Set-Text $inputs[3] 'SSH 新目的地'
    Set-Text $inputs[4] 'localhost'
    Click (Button '儲存')
    $copies = @((Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value)
    $changed = @($copies | Where-Object { $_.name -eq 'SSH 新目的地' })
    if ($changed.Count -ne 1 -or $changed[0].params.'host-key' -or $changed[0].params.password -or $changed[0].params.'private-key' -or $changed[0].params.passphrase) { throw 'changed-host duplicate retained identity or secrets' }
    foreach ($name in @('SSH 同目的地', 'SSH 新目的地')) {
        Set-Text $inputs[14] $name
        Select-First $list
        if ($name -eq 'SSH 同目的地') {
            [void][SshGui]::PostMessage($list, 0x0100, [IntPtr]0x2e, [IntPtr]::Zero)
            Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } 'Delete key did not open confirmation'
            [void][SshGui]::PostMessage([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0111, [IntPtr]7, [IntPtr]::Zero)
            Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -eq [IntPtr]::Zero } 'cancel deletion dialog did not close'
            if (@((Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value | Where-Object { $_.name -eq $name }).Count -ne 1) { throw 'cancelled Delete key removed the connection' }
            [void][SshGui]::PostMessage($list, 0x0100, [IntPtr]0x2e, [IntPtr]::Zero)
        } else {
            [void][SshGui]::PostMessage((Button '刪除'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        }
        Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } 'duplicate deletion dialog not found'
        [void][SshGui]::PostMessage([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0111, [IntPtr]6, [IntPtr]::Zero)
        Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -eq [IntPtr]::Zero } 'duplicate deletion dialog did not close'
        Wait-For { @((Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value | Where-Object { $_.name -eq $name }).Count -eq 0 } 'duplicate was not deleted'
    }
    Set-Text $inputs[14] 'SSH fixture'
    Select-First $list
    Click (Button '複製')
    Set-Text $inputs[3] 'SSH 切換目標'
    Set-Text $inputs[4] 'localhost'
    Click (Button '儲存')
    Set-Text $inputs[14] ''
    Select-First $list
    Click (Button '核對身分')
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq 'SSH 主機金鑰與現有信任相符。' }).Count -eq 1 } 'identity did not match'
    Set-Text $inputs[14] "alice@127.0.0.1:$Port"
    [void][SshGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
    Wait-For { @($children | Where-Object { [SshGui]::Class($_) -eq 'Static' -and [SshGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -gt 0 } 'SSH frame not displayed' 15
    if (@((Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value).Count -ne 2) { throw 'quick connect created a duplicate connection' }
    Set-Text $inputs[14] "alice@localhost:$Port"
    [void][SshGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } 'active quick switch confirmation missing'
    [void][SshGui]::PostMessage([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0111, [IntPtr]7, [IntPtr]::Zero)
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -eq [IntPtr]::Zero } 'active quick switch cancellation failed'
    Set-Text $inputs[14] "alice@127.0.0.1:$Port"
    [void][SshGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '已連線至此主機。' }).Count -eq 1 } 'active quick connect to current host not recognized'
    Set-Text $inputs[14] 'nobody@unconfigured.invalid:22'
    [void][SshGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '此目的地尚未保存；請先中斷目前連線再新增。' }).Count -eq 1 } 'unknown active quick target did not preserve session'
    Set-Text $inputs[14] ''
    [void][SshGui]::PostMessage($list, 0x0100, [IntPtr]0x2e, [IntPtr]::Zero)
    [void][SshGui]::PostMessage($list, 0x0100, [IntPtr]0x71, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 250
    if ([SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero) { throw 'Delete key opened a deletion dialog during an active session' }
    if (([SshGui]::GetWindowLong($inputs[3], -16) -band 0x10000000) -ne 0) { throw 'F2 opened the editor during an active session' }
    $frame = @($children | Where-Object { [SshGui]::Class($_) -eq 'Static' -and [SshGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero })[0]
    if ($env:CRACAMOLE_TEST_VISIBLE_SSH -eq '1') {
        try {
            [void][SshGui]::ShowWindow($window, 4) # SW_SHOWNOACTIVATE
            if (-not [SshGui]::SetWindowPos($window, [IntPtr](-1), 0, 0, 0, 0, 0x0011)) { throw 'cannot place test window above other windows' }
            try { Wait-For { [SshGui]::VisibleTerminalInk($frame, [uint32]$process.Id) } 'visible SSH terminal text not detected' 10 }
            catch { throw "visible SSH terminal text not detected: $([SshGui]::PixelDiagnostic)" }
            Write-Output 'Windows 螢幕合成後的 SSH 終端機文字與背景已確認。'
        } finally {
            [void][SshGui]::ShowWindow($window, 0) # SW_HIDE
        }
    }
    if (-not [SshGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 1000, 800, 0x0006)) { throw 'window resize failed' }
    Wait-For { [SshGui]::Width($frame) -gt 700 } 'remote frame did not resize'
    Start-Sleep -Milliseconds 700
    if (-not [SshGui]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 700, 600, 0x0006)) { throw 'window shrink failed' }
    Wait-For { [SshGui]::Width($frame) -lt 700 } 'remote frame did not shrink'
    Start-Sleep -Milliseconds 700
    [void][SshGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]122, [IntPtr]::Zero)
    [void][SshGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]0x4e2d, [IntPtr]::Zero)
    if ($EncodingName -eq 'UTF-8') {
        [void][SshGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]0xd83d, [IntPtr]::Zero)
        [void][SshGui]::SendMessageInt($inputs[2], 0x0102, [IntPtr]0xde42, [IntPtr]::Zero)
    }
    Start-Sleep -Milliseconds 500
    Set-Text $inputs[14] "alice@localhost:$Port"
    [void][SshGui]::SendMessageInt($inputs[14], 0x0100, [IntPtr]13, [IntPtr]::Zero)
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } 'confirmed active quick switch dialog missing'
    [void][SshGui]::PostMessage([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0111, [IntPtr]6, [IntPtr]::Zero)
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '遠端連線錯誤。請檢查連線設定；SSH 請使用「核對身分」確認主機金鑰。' }).Count -eq 1 } 'untrusted quick target was not rejected' 15
    $target = @((Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value | Where-Object { $_.name -eq 'SSH 切換目標' })
    if ($target.Count -ne 1 -or $target[0].params.'host-key') { throw 'active quick switch automatically trusted host key' }
    Set-Text $inputs[14] 'SSH 切換目標'
    Select-First $list
    [void][SshGui]::PostMessage((Button '刪除'), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-For { [SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero } 'quick target deletion confirmation missing'
    [void][SshGui]::PostMessage([SshGui]::Find([uint32]$process.Id, '#32770'), 0x0111, [IntPtr]6, [IntPtr]::Zero)
    Wait-For { @((Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value).Count -eq 1 } 'quick target not deleted'
    Click (Button '登出')
    [void][SshGui]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $process.WaitForExit(10000)) { throw '程式關閉逾時' }
    $process.Refresh()
    if ($process.ExitCode -ne 0) { throw '程式異常結束' }
    [IO.File]::WriteAllText((Join-Path $SyncDir 'trusted-complete'), '')
    Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'continue-mismatch')) } 'test coordinator did not continue mismatch test'
    $file = Join-Path $desktopDir 'connections.json'
    $database = Get-Content -Raw -Encoding utf8 $file | ConvertFrom-Json
    $entry = $database.PSObject.Properties.Value
    $entry.params.'host-key' = $MismatchedKey
    [IO.File]::WriteAllText($file, ($database | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $process = Start-Process -FilePath $Binary -WindowStyle Hidden -PassThru
    Wait-For { [SshGui]::Find([uint32]$process.Id, $null) -ne [IntPtr]::Zero } 'restart window not found' 15
    $window = [SshGui]::Find([uint32]$process.Id, $null)
    if (-not $process.WaitForInputIdle(10000)) { throw 'restart controls timed out' }
    $children = [SshGui]::Children($window)
    $inputs = @($children | Where-Object { [SshGui]::Class($_) -eq 'Edit' })
    $list = @($children | Where-Object { [SshGui]::Class($_) -eq 'ListBox' })[0]
    Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
    Click (Button '登入')
    Select-First $list
    Click (Button '核對身分')
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq 'SSH 主機金鑰與現有信任不符，已拒絕取代。' }).Count -eq 1 } 'mismatched SSH host key was not rejected'
    if ([SshGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero) { throw 'mismatched host key prompted for replacement' }
    $saved = (Get-Content -Raw -Encoding utf8 $file | ConvertFrom-Json).PSObject.Properties.Value
    if ($saved.params.'host-key' -ne $MismatchedKey) { throw 'mismatched host key was overwritten' }
    Click (Button '連線')
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '遠端連線錯誤。請檢查連線設定；SSH 請使用「核對身分」確認主機金鑰。' }).Count -eq 1 } 'mismatched SSH connection was not rejected'
    if ($PrivateKeyPath) {
        Click (Button '中斷連線')
        Select-First $list
        [void][SshGui]::PostMessage($list, 0x0100, [IntPtr]0x71, [IntPtr]::Zero)
        Wait-For { ([SshGui]::GetWindowLong($inputs[3], -16) -band 0x10000000) -ne 0 } 'F2 did not open the selected connection editor'
        if ([SshGui]::EditText($inputs[8]) -ne '' -or [SshGui]::EditText($inputs[9]) -ne '') { throw '編輯時洩露私鑰或密語' }
        $tooLarge = Join-Path $SyncDir 'oversized-key'
        [IO.File]::WriteAllText($tooLarge, ('x' * 8193))
        Set-Text $inputs[8] $tooLarge
        Click (Button '儲存')
        if (-not @($children | Where-Object { [SshGui]::Text($_) -eq '私鑰檔案必須是 1 至 8192 位元組的文字。' })) { throw '過大的私鑰未被拒絕' }
        Set-Text $inputs[8] ''
        Click (Button '儲存')
        $saved = (Get-Content -Raw -Encoding utf8 $file | ConvertFrom-Json).PSObject.Properties.Value
        if ($saved.params.'private-key' -ne [IO.File]::ReadAllText($PrivateKeyPath) -or $saved.params.passphrase -ne 'fixture-passphrase') { throw '留空編輯未保留私鑰和密語' }
        Select-First $list
        Click (Button '編輯')
        $clear = @($children | Where-Object { [SshGui]::Text($_) -eq '清除已保存的密碼、私鑰和密語' })[0]
        [void][SshGui]::SendMessageInt($clear, 0x00F1, [IntPtr]1, [IntPtr]::Zero)
        Click (Button '儲存')
        $saved = (Get-Content -Raw -Encoding utf8 $file | ConvertFrom-Json).PSObject.Properties.Value
        if ($saved.params.'private-key' -ne '' -or $saved.params.passphrase -ne '') { throw '清除私鑰和密語失敗' }
    }
    [void][SshGui]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $process.WaitForExit(10000)) { throw 'restart process failed to exit' }
    $process.Refresh()
    if ($process.ExitCode -ne 0) { throw 'restart process exited abnormally' }
} finally {
    if ($process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        if (-not $process.WaitForExit(10000)) { throw 'test process cleanup timed out' }
    }
    foreach ($name in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnv[$name]) }
    if ([IO.Directory]::Exists($desktopDir)) {
        [IO.File]::Delete((Join-Path $desktopDir 'connections.json'))
        [IO.File]::Delete((Join-Path $desktopDir 'ssh-identities.json'))
        [IO.File]::Delete((Join-Path $desktopDir 'pinned-connections.json'))
        [IO.File]::Delete((Join-Path $desktopDir 'recent-connections.json'))
        [IO.File]::Delete((Join-Path $desktopDir 'desktop.lock'))
        [IO.Directory]::Delete($desktopDir)
    }
    if ([IO.Directory]::Exists($testDir)) { [IO.Directory]::Delete($testDir) }
}
