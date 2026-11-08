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
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, string s);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] public static extern IntPtr SendMessageInt(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageText(IntPtr h, uint msg, IntPtr w, StringBuilder l);
    public static string EditText(IntPtr h) { var s = new StringBuilder(4096); SendMessageText(h, 0x000D, (IntPtr)s.Capacity, s); return s.ToString(); }
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
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
    if ($inputs.Count -ne 13) { throw '找不到預期的輸入控制項' }
    $combos = @($children | Where-Object { [SshGui]::Class($_) -eq 'ComboBox' })
    if ($combos.Count -ne 3) { throw '找不到文字編碼控制項' }
    function Button([string]$Text) {
        $buttons = @($children | Where-Object { [SshGui]::Text($_) -eq $Text })
        if ($buttons.Count -ne 1) { throw "找不到按鈕：$Text" }
        return $buttons[0]
    }
    $list = @($children | Where-Object { [SshGui]::Class($_) -eq 'ListBox' })[0]
    Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
    Click (Button '登入')
    Click (Button '新增')
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
    Select-First $list
    Click (Button '連線')
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '連線已結束或失敗。' }).Count -eq 1 } 'untrusted SSH connection was not rejected'
    Click (Button '中斷連線')
    [IO.File]::WriteAllText((Join-Path $SyncDir 'untrusted-rejected'), '')
    Wait-For { [IO.File]::Exists((Join-Path $SyncDir 'continue')) } 'test coordinator did not continue'
    Select-First $list
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
    $saved = (Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value
    if ($saved.params.'host-key' -ne $PublicKey) { throw '主機金鑰保存錯誤' }
    if ($saved.params.'character-encoding' -ne $EncodingName) { throw 'SSH 文字編碼保存錯誤' }
    if ($PrivateKeyPath) {
        if ($saved.params.'private-key' -ne [IO.File]::ReadAllText($PrivateKeyPath) -or $saved.params.passphrase -ne 'fixture-passphrase') { throw '私鑰或密語保存錯誤' }
    } elseif ($saved.params.password -ne 'secret') { throw '密碼保存錯誤' }
    Select-First $list
    Click (Button '核對身分')
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq 'SSH 主機金鑰與現有信任相符。' }).Count -eq 1 } 'identity did not match'
    Click (Button '連線')
    Wait-For { @($children | Where-Object { [SshGui]::Class($_) -eq 'Static' -and [SshGui]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero }).Count -gt 0 } 'SSH frame not displayed' 15
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
    Wait-For { @($children | Where-Object { [SshGui]::Text($_) -eq '連線已結束或失敗。' }).Count -eq 1 } 'mismatched SSH connection was not rejected'
    if ($PrivateKeyPath) {
        Click (Button '中斷連線')
        Select-First $list
        Click (Button '編輯')
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
        [IO.File]::Delete((Join-Path $desktopDir 'desktop.lock'))
        [IO.Directory]::Delete($desktopDir)
    }
    if ([IO.Directory]::Exists($testDir)) { [IO.Directory]::Delete($testDir) }
}
