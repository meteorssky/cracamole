param([string]$Binary = "$PSScriptRoot/../../../target/debug/cracamole-desktop.exe")
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Net;
using System.Net.Sockets;
using System.Threading.Tasks;
public static class DesktopLoginTest {
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
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetKeyboardLayout(uint threadId);
    [DllImport("user32.dll", EntryPoint="MapVirtualKeyExW")] public static extern uint MapVirtualKeyEx(uint code, uint kind, IntPtr layout);
    public static uint LayoutChar(IntPtr h, uint code) { uint pid; var thread = GetWindowThreadProcessId(h, out pid); return MapVirtualKeyEx(code, 2, GetKeyboardLayout(thread)); }
    [DllImport("user32.dll", EntryPoint="GetWindowLongW")] public static extern int GetWindowLong(IntPtr h, int index);
    public static bool OwnVisible(IntPtr h) { return (GetWindowLong(h, -16) & 0x10000000) != 0; }
    public static IntPtr Window(uint pid) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((h,p) => { uint owner; GetWindowThreadProcessId(h, out owner); if (owner == pid && Text(h).StartsWith("cracamole — Windows")) { result = h; return false; } return true; }, IntPtr.Zero);
        return result;
    }
    public static IntPtr Dialog(uint pid) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((h,p) => { uint owner; GetWindowThreadProcessId(h, out owner); if (owner == pid && Class(h) == "#32770") { result = h; return false; } return true; }, IntPtr.Zero);
        return result;
    }
    public static IntPtr[] Children(IntPtr parent) {
        var result = new List<IntPtr>();
        EnumChildWindows(parent, (h,p) => { result.Add(h); return true; }, IntPtr.Zero);
        return result.ToArray();
    }
    public static string Text(IntPtr h) { var s = new StringBuilder(2048); GetWindowText(h,s,s.Capacity); return s.ToString(); }
    public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassName(h,s,s.Capacity); return s.ToString(); }
}
public sealed class DesktopTelnetFixture : IDisposable {
    private TcpListener listener;
    public Task Closed;
    public TaskCompletionSource<bool> Received = new TaskCompletionSource<bool>();
    public TaskCompletionSource<bool> ReceivedCtrl = new TaskCompletionSource<bool>();
    public TaskCompletionSource<bool> ReceivedCtrlDigits = new TaskCompletionSource<bool>();
    public TaskCompletionSource<bool> ReceivedCtrlBracket = new TaskCompletionSource<bool>();
    public TaskCompletionSource<bool> ReceivedTab = new TaskCompletionSource<bool>();
    public TaskCompletionSource<bool> ReceivedBig5 = new TaskCompletionSource<bool>();
    public int Port { get; private set; }
    public DesktopTelnetFixture() {
        listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        Port = ((IPEndPoint)listener.LocalEndpoint).Port;
        Closed = Task.Run(async () => {
            using (var client = await listener.AcceptTcpClientAsync())
            using (var stream = client.GetStream()) {
                var banner = Encoding.ASCII.GetBytes("Windows native preview\r\n");
                await stream.WriteAsync(banner, 0, banner.Length);
                var buf = new byte[1024];
                int previous = -1;
                bool sawZ = false;
                int count;
                while ((count = await stream.ReadAsync(buf, 0, buf.Length)) != 0) {
                    for (int i = 0; i < count; i++) {
                        if (buf[i] == (byte)'z') Received.TrySetResult(true);
                        if (buf[i] == (byte)'z') sawZ = true;
                        if (sawZ && buf[i] == 9) ReceivedTab.TrySetResult(true);
                        if (buf[i] == 3) ReceivedCtrl.TrySetResult(true);
                        if (previous == 0 && buf[i] == 27) ReceivedCtrlDigits.TrySetResult(true);
                        if (previous == 27 && buf[i] == 27) ReceivedCtrlBracket.TrySetResult(true);
                        if (previous == 0xa4 && buf[i] == 0xa4) ReceivedBig5.TrySetResult(true);
                        previous = buf[i];
                    }
                }
            }
        });
    }
    public void Dispose() { listener.Stop(); }
}
'@
$previousPassword = $env:CRACAMOLE_ADMIN_PASSWORD
$previousUsername = $env:CRACAMOLE_ADMIN_USERNAME
$previousDataDir = $env:CRACAMOLE_DATA_DIR
$previousDesktopDataDir = $env:CRACAMOLE_DESKTOP_DATA_DIR
$previousLifetime = $env:CRACAMOLE_DESKTOP_SESSION_MAX_SECS
$env:CRACAMOLE_ADMIN_USERNAME = 'test-admin'
$env:CRACAMOLE_ADMIN_PASSWORD = 'desktop-test-password-123'
$env:CRACAMOLE_DESKTOP_SESSION_MAX_SECS = '6'
$fixture = [DesktopTelnetFixture]::new()
$testDir = Join-Path ([System.IO.Path]::GetTempPath()) ("cracamole-gui-test-" + [Guid]::NewGuid().ToString('N'))
[void][System.IO.Directory]::CreateDirectory($testDir)
$desktopDir = Join-Path $testDir 'desktop'
$env:CRACAMOLE_DESKTOP_DATA_DIR = $desktopDir
$testProcess = $null
try {
    $testProcess = Start-Process -FilePath (Resolve-Path $Binary).Path -WorkingDirectory (Get-Location).Path -WindowStyle Hidden -PassThru
    $window = [IntPtr]::Zero
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    while ($window -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $deadline -and -not $testProcess.HasExited) {
        $window = [DesktopLoginTest]::Window([uint32]$testProcess.Id)
        if ($window -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100 }
    }
    if ($window -eq [IntPtr]::Zero) { throw '找不到原生視窗' }
    if (-not $testProcess.WaitForInputIdle(10000)) { throw '原生控制項初始化逾時' }
    $children = [DesktopLoginTest]::Children($window)
    $inputs = @($children | Where-Object { [DesktopLoginTest]::Class($_) -eq 'Edit' })
    $login = @($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '登入' })
    if ($inputs.Count -ne 13 -or $login.Count -ne 1) { throw "找不到登入控制項（Edit 數量：$($inputs.Count)）" }
    [void][DesktopLoginTest]::SendMessage($inputs[1], 0x000C, [IntPtr]::Zero, 'wrong-password')
    [void][DesktopLoginTest]::SendMessage($login[0], 0x00F5, [IntPtr]::Zero, $null)
    if (-not ($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '帳號或密碼不正確。' })) { throw '錯誤密碼未被拒絕' }
    if ([DesktopLoginTest]::EditText($inputs[1]) -ne '') { throw '錯誤密碼未清除' }
    [void][DesktopLoginTest]::SendMessage($inputs[1], 0x000C, [IntPtr]::Zero, $env:CRACAMOLE_ADMIN_PASSWORD)
    [void][DesktopLoginTest]::SendMessage($login[0], 0x00F5, [IntPtr]::Zero, $null)
    if (-not ($children | Where-Object { [DesktopLoginTest]::Text($_) -like '已載入 0 筆桌面連線。*' })) { throw '登入後未載入桌面連線清單' }
    if ([DesktopLoginTest]::EditText($inputs[1]) -ne '') { throw '登入密碼未清除' }
    $list = @($children | Where-Object { [DesktopLoginTest]::Class($_) -eq 'ListBox' })
    $logout = @($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '登出' })
    if ($list.Count -ne 1 -or $logout.Count -ne 1) { throw '找不到連線清單或登出控制項' }
    $expiryDeadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not ($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '已登出，請重新登入。' }) -and [DateTime]::UtcNow -lt $expiryDeadline) {
        Start-Sleep -Milliseconds 100
    }
    if (-not ($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '已登出，請重新登入。' })) { throw '工作階段未到期' }
    [void][DesktopLoginTest]::SendMessage($inputs[1], 0x000C, [IntPtr]::Zero, $env:CRACAMOLE_ADMIN_PASSWORD)
    [void][DesktopLoginTest]::SendMessage($login[0], 0x00F5, [IntPtr]::Zero, $null)
    if (-not ($children | Where-Object { [DesktopLoginTest]::Text($_) -like '已載入 0 筆桌面連線。*' })) { throw '到期後無法重新登入' }
    $add = @($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '新增' })
    $save = @($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '儲存' })
    $protocol = @($children | Where-Object { [DesktopLoginTest]::Class($_) -eq 'ComboBox' })
    if ($add.Count -ne 1 -or $save.Count -ne 1 -or $protocol.Count -ne 3) { throw '找不到連線編輯控制項' }
    [void][DesktopLoginTest]::SendMessage($add[0], 0x00F5, [IntPtr]::Zero, $null)
    [void][DesktopLoginTest]::SendMessage($inputs[3], 0x000C, [IntPtr]::Zero, 'test-server')
    [void][DesktopLoginTest]::SendMessageInt($protocol[0], 0x014E, [IntPtr]1, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($window, 0x0111, [IntPtr](0x10000 -bor [DesktopLoginTest]::GetDlgCtrlID($protocol[0])), $protocol[0])
    if (-not [DesktopLoginTest]::OwnVisible($inputs[11]) -or -not [DesktopLoginTest]::OwnVisible($inputs[12]) -or -not [DesktopLoginTest]::OwnVisible($protocol[2]) -or [DesktopLoginTest]::OwnVisible($inputs[8])) { throw 'Telnet 專用欄位顯示錯誤' }
    [void][DesktopLoginTest]::SendMessage($inputs[4], 0x000C, [IntPtr]::Zero, '127.0.0.1')
    [void][DesktopLoginTest]::SendMessage($inputs[5], 0x000C, [IntPtr]::Zero, [string]$fixture.Port)
    [void][DesktopLoginTest]::SendMessage($inputs[7], 0x000C, [IntPtr]::Zero, 'fixture-secret')
    [void][DesktopLoginTest]::SendMessageInt($protocol[2], 0x014E, [IntPtr]1, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessage($inputs[11], 0x000C, [IntPtr]::Zero, 'Windows native preview')
    [void][DesktopLoginTest]::SendMessage($save[0], 0x00F5, [IntPtr]::Zero, $null)
    if (-not ($children | Where-Object { [DesktopLoginTest]::Text($_) -eq 'Telnet 登入成功及失敗的正規表示式須同時填寫。' })) { throw '未拒絕缺少失敗表示式' }
    [void][DesktopLoginTest]::SendMessage($inputs[12], 0x000C, [IntPtr]::Zero, '[')
    [void][DesktopLoginTest]::SendMessage($save[0], 0x00F5, [IntPtr]::Zero, $null)
    if (-not ($children | Where-Object { [DesktopLoginTest]::Text($_) -eq 'Telnet 登入正規表示式無效。' })) { throw '未拒絕無效表示式' }
    [void][DesktopLoginTest]::SendMessage($inputs[12], 0x000C, [IntPtr]::Zero, 'incorrect')
    [void][DesktopLoginTest]::SendMessage($save[0], 0x00F5, [IntPtr]::Zero, $null)
    if ([DesktopLoginTest]::SendMessageInt($list[0], 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 1) { throw '新增連線失敗' }
    $edit = @($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '編輯' })
    if ($edit.Count -ne 1) { throw '找不到編輯按鈕' }
    [void][DesktopLoginTest]::SendMessageInt($list[0], 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessage($edit[0], 0x00F5, [IntPtr]::Zero, $null)
    if ([DesktopLoginTest]::EditText($inputs[7]) -ne '') { throw '編輯時洩露已保存密碼' }
    if ([DesktopLoginTest]::EditText($inputs[11]) -ne 'Windows native preview' -or [DesktopLoginTest]::EditText($inputs[12]) -ne 'incorrect' -or [DesktopLoginTest]::SendMessageInt($protocol[2], 0x0147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 1) { throw '編輯未還原 Telnet 設定' }
    [void][DesktopLoginTest]::SendMessage($inputs[3], 0x000C, [IntPtr]::Zero, 'updated-server')
    [void][DesktopLoginTest]::SendMessage($save[0], 0x00F5, [IntPtr]::Zero, $null)
    $saved = (Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value
    if ($saved.name -ne 'updated-server' -or $saved.params.password -ne 'fixture-secret' -or $saved.params.'character-encoding' -ne 'Big5' -or $saved.params.'login-success-regex' -ne 'Windows native preview' -or $saved.params.'login-failure-regex' -ne 'incorrect') { throw '編輯未保留既有機密或 Telnet 設定' }
    $clear = @($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '清除已保存的密碼、私鑰和密語' })
    if ($clear.Count -ne 1) { throw '找不到清除機密選項' }
    [void][DesktopLoginTest]::SendMessageInt($list[0], 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessage($edit[0], 0x00F5, [IntPtr]::Zero, $null)
    [void][DesktopLoginTest]::SendMessageInt($clear[0], 0x00F1, [IntPtr]1, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessage($save[0], 0x00F5, [IntPtr]::Zero, $null)
    $saved = (Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value
    if ($saved.params.password -ne '') { throw '明確清除機密失敗' }
    $connect = @($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '連線' })
    if ($connect.Count -ne 1) { throw '找不到連線按鈕' }
    [void][DesktopLoginTest]::SendMessageInt($list[0], 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessage($connect[0], 0x00F5, [IntPtr]::Zero, $null)
    $frameDeadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $frame = @($children | Where-Object { [DesktopLoginTest]::Class($_) -eq 'Static' -and [DesktopLoginTest]::SendMessageInt($_, 0x0173, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero })
        if ($frame.Count -eq 0) { Start-Sleep -Milliseconds 100 }
    } while ($frame.Count -eq 0 -and [DateTime]::UtcNow -lt $frameDeadline)
    if ($frame.Count -eq 0) { throw '原生畫面控制項未顯示遠端影格' }
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0102, [IntPtr]122, [IntPtr]::Zero)
    if (-not $fixture.Received.Task.Wait(3000)) { throw '原生輸入未送達 Telnet 伺服器' }
    $dialogCode = [DesktopLoginTest]::SendMessageInt($inputs[2], 0x0087, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    if (($dialogCode -band 6) -ne 6) { throw '遠端輸入未要求接收 Tab 與對話框按鍵' }
    [void][DesktopLoginTest]::PostMessage($inputs[2], 0x0100, [IntPtr]9, [IntPtr]::Zero)
    [void][DesktopLoginTest]::PostMessage($inputs[2], 0x0101, [IntPtr]9, [IntPtr]::Zero)
    if (-not $fixture.ReceivedTab.Task.Wait(3000)) { throw '排入訊息佇列的 Tab 未送達 Telnet 伺服器' }
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0102, [IntPtr]0x4e2d, [IntPtr]::Zero)
    if (-not $fixture.ReceivedBig5.Task.Wait(3000)) { throw '繁體中文字未以 Big5 送達 Telnet 伺服器' }
    if ([DesktopLoginTest]::EditText($inputs[2]) -ne '') { throw '遠端輸入欄位未清除' }
    $fixture.ReceivedCtrl = [System.Threading.Tasks.TaskCompletionSource[bool]]::new()
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0100, [IntPtr]0x11, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0100, [IntPtr]0x43, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0102, [IntPtr]3, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0101, [IntPtr]0x43, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0101, [IntPtr]0x11, [IntPtr]::Zero)
    if (-not $fixture.ReceivedCtrl.Task.Wait(3000)) { throw 'Ctrl+C 未送達 Telnet 伺服器' }
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0100, [IntPtr]0x11, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0100, [IntPtr]0x32, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0102, [IntPtr]0, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0101, [IntPtr]0x32, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0100, [IntPtr]0x33, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0102, [IntPtr]27, [IntPtr]::Zero)
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0101, [IntPtr]0x33, [IntPtr]::Zero)
    $testBracket = [DesktopLoginTest]::LayoutChar($inputs[2], 0xdb) -eq 0x5b
    if ($testBracket) {
        [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0100, [IntPtr]0xdb, [IntPtr]::Zero)
        [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0102, [IntPtr]27, [IntPtr]::Zero)
        [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0101, [IntPtr]0xdb, [IntPtr]::Zero)
    }
    [void][DesktopLoginTest]::SendMessageInt($inputs[2], 0x0101, [IntPtr]0x11, [IntPtr]::Zero)
    if (-not $fixture.ReceivedCtrlDigits.Task.Wait(3000)) { throw 'Ctrl+2／Ctrl+3 未正確送達 Telnet 伺服器' }
    if ($testBracket -and -not $fixture.ReceivedCtrlBracket.Task.Wait(3000)) { throw 'Ctrl+[ 未正確送達 Telnet 伺服器' }
    if ($testBracket) { Write-Output '已驗證 Ctrl+[。' } else { Write-Output 'SKIP: 目前鍵盤配置的指定按鍵不是左方括號。' }
    [void][DesktopLoginTest]::SendMessage($logout[0], 0x00F5, [IntPtr]::Zero, $null)
    if ([DesktopLoginTest]::SendMessageInt($list[0], 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 0) { throw '登出未清空連線清單' }
    if (-not ($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '已登出，請重新登入。' })) { throw '登出狀態不正確' }
    if (-not $fixture.Closed.Wait(8000)) { throw '登出後遠端連線未關閉' }
    if ($fixture.Closed.IsFaulted) { throw '遠端測試服務發生錯誤' }
    [void][DesktopLoginTest]::SendMessage($inputs[1], 0x000C, [IntPtr]::Zero, $env:CRACAMOLE_ADMIN_PASSWORD)
    [void][DesktopLoginTest]::SendMessage($login[0], 0x00F5, [IntPtr]::Zero, $null)
    if ([DesktopLoginTest]::SendMessageInt($list[0], 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 1) { throw '重新登入後連線未保存' }
    $delete = @($children | Where-Object { [DesktopLoginTest]::Text($_) -eq '刪除' })
    if ($delete.Count -ne 1) { throw '找不到刪除按鈕' }
    [void][DesktopLoginTest]::SendMessageInt($list[0], 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][DesktopLoginTest]::PostMessage($delete[0], 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    $dialog = [IntPtr]::Zero
    $dialogDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ($dialog -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $dialogDeadline) {
        $dialog = [DesktopLoginTest]::Dialog([uint32]$testProcess.Id)
        if ($dialog -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 50 }
    }
    if ($dialog -eq [IntPtr]::Zero) { throw '找不到刪除確認視窗' }
    [void][DesktopLoginTest]::PostMessage($dialog, 0x0111, [IntPtr]7, [IntPtr]::Zero)
    $dismissDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DesktopLoginTest]::Dialog([uint32]$testProcess.Id) -ne [IntPtr]::Zero -and [DateTime]::UtcNow -lt $dismissDeadline) { Start-Sleep -Milliseconds 50 }
    if ([DesktopLoginTest]::SendMessageInt($list[0], 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 1) { throw '取消刪除後連線消失' }
    [void][DesktopLoginTest]::PostMessage($delete[0], 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    $dialog = [IntPtr]::Zero
    $dialogDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ($dialog -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $dialogDeadline) {
        $dialog = [DesktopLoginTest]::Dialog([uint32]$testProcess.Id)
        if ($dialog -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 50 }
    }
    if ($dialog -eq [IntPtr]::Zero) { throw '找不到再次刪除確認視窗' }
    [void][DesktopLoginTest]::PostMessage($dialog, 0x0111, [IntPtr]6, [IntPtr]::Zero)
    $deleteDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DesktopLoginTest]::SendMessageInt($list[0], 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 0 -and [DateTime]::UtcNow -lt $deleteDeadline) { Start-Sleep -Milliseconds 50 }
    if ([DesktopLoginTest]::SendMessageInt($list[0], 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 0) { throw '刪除連線失敗' }
    [void][DesktopLoginTest]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $testProcess.WaitForExit(10000)) { throw '關閉逾時' }
    $testProcess.Refresh()
    if ($testProcess.ExitCode -ne 0) { throw '關閉失敗' }
    Write-Output '原生登入、連線管理、機密保留、工作階段到期、Telnet 表示式與 Big5 輸入及登出關閉均通過。'
} finally {
    if ($testProcess -and -not $testProcess.HasExited) {
        Stop-Process -Id $testProcess.Id -Force
        if (-not $testProcess.WaitForExit(10000)) { throw 'test process cleanup timed out' }
    }
    $env:CRACAMOLE_ADMIN_PASSWORD = $previousPassword
    $env:CRACAMOLE_ADMIN_USERNAME = $previousUsername
    $env:CRACAMOLE_DATA_DIR = $previousDataDir
    $env:CRACAMOLE_DESKTOP_DATA_DIR = $previousDesktopDataDir
    $env:CRACAMOLE_DESKTOP_SESSION_MAX_SECS = $previousLifetime
    $fixture.Dispose()
    if ([System.IO.Directory]::Exists($desktopDir)) {
        [System.IO.File]::Delete((Join-Path $desktopDir 'connections.json'))
        [System.IO.File]::Delete((Join-Path $desktopDir 'desktop.lock'))
        [System.IO.Directory]::Delete($desktopDir)
    }
    if ([System.IO.Directory]::Exists($testDir)) { [System.IO.Directory]::Delete($testDir) }
}
