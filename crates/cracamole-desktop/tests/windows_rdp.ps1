param([string]$Binary)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Net;
using System.Net.Sockets;
using System.Net.Security;
using System.Runtime.InteropServices;
using System.Security.Authentication;
using System.Security.Cryptography.X509Certificates;
using System.Text;
using System.Threading.Tasks;
public static class RdpGui {
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
    [DllImport("user32.dll", EntryPoint="GetWindowLongW")] public static extern int GetWindowLong(IntPtr h, int index);
    public static bool OwnVisible(IntPtr h) { return (GetWindowLong(h, -16) & 0x10000000) != 0; }
    public static IntPtr Find(uint pid, string cls) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((h,p) => { uint owner; GetWindowThreadProcessId(h, out owner); if (owner == pid && (String.IsNullOrEmpty(cls) ? Text(h).StartsWith("cracamole — Windows") : Class(h) == cls)) { result = h; return false; } return true; }, IntPtr.Zero);
        return result;
    }
    public static IntPtr[] Children(IntPtr parent) { var result = new List<IntPtr>(); EnumChildWindows(parent, (h,p) => { result.Add(h); return true; }, IntPtr.Zero); return result.ToArray(); }
    public static string Text(IntPtr h) { var s = new StringBuilder(4096); GetWindowText(h,s,s.Capacity); return s.ToString(); }
    public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassName(h,s,s.Capacity); return s.ToString(); }
}
public sealed class CertificateServer : IDisposable {
    private TcpListener listener;
    private X509Certificate2 cert;
    public int Port { get; private set; }
    public volatile uint LastProtocols;
    public volatile int Handshakes;
    public Task Finished;
    public CertificateServer(X509Certificate2 cert) {
        this.cert = cert;
        listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        Port = ((IPEndPoint)listener.LocalEndpoint).Port;
        Finished = Task.Run(async () => {
            for (int i = 0; i < 5; i++) {
                using (var client = await listener.AcceptTcpClientAsync())
                using (var stream = client.GetStream()) {
                    var header = new byte[4];
                    int got = 0;
                    while (got < 4) { int n = await stream.ReadAsync(header, got, 4 - got); if (n == 0) throw new Exception("short X224 header"); got += n; }
                    int remaining = ((int)header[2] << 8 | header[3]) - 4;
                    if (remaining < 0 || remaining > 8192) throw new Exception("invalid X224 size");
                    var body = new byte[remaining]; got = 0;
                    while (got < remaining) { int n = await stream.ReadAsync(body, got, remaining - got); if (n == 0) throw new Exception("short X224 body"); got += n; }
                    if (remaining < 8) throw new Exception("missing RDP negotiation request");
                    LastProtocols = BitConverter.ToUInt32(body, remaining - 4);
                    var reply = new byte[] {3,0,0,19,14,0xd0,0,0,0,0,0,2,0,8,0,1,0,0,0};
                    await stream.WriteAsync(reply, 0, reply.Length);
                    using (var tls = new SslStream(stream, false)) {
                        await tls.AuthenticateAsServerAsync(cert, false, SslProtocols.Tls12, false);
                        Handshakes++;
                    }
                }
            }
        });
    }
    public void Dispose() { listener.Stop(); }
}
'@
function Wait-For([scriptblock]$Condition, [string]$Message, [int]$Seconds = 10) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (-not (& $Condition)) { if ([DateTime]::UtcNow -ge $deadline) { throw $Message }; Start-Sleep -Milliseconds 50 }
}
function Click([IntPtr]$Handle) { [void][RdpGui]::SendMessage($Handle, 0x00F5, [IntPtr]::Zero, $null) }
function Set-Text([IntPtr]$Handle, [string]$Value) { [void][RdpGui]::SendMessage($Handle, 0x000C, [IntPtr]::Zero, $Value) }
function Button([string]$Text) {
    $buttons = @($children | Where-Object { [RdpGui]::Text($_) -eq $Text })
    if ($buttons.Count -ne 1) { throw "button not found: $Text" }; return $buttons[0]
}
$savedEnv = @{}
foreach ($name in @('CRACAMOLE_ADMIN_USERNAME', 'CRACAMOLE_ADMIN_PASSWORD', 'CRACAMOLE_DATA_DIR', 'CRACAMOLE_DESKTOP_DATA_DIR', 'CRACAMOLE_DESKTOP_SESSION_MAX_SECS')) { $savedEnv[$name] = [Environment]::GetEnvironmentVariable($name) }
$env:CRACAMOLE_ADMIN_USERNAME = 'test-admin'
$env:CRACAMOLE_ADMIN_PASSWORD = 'desktop-test-password-123'
$env:CRACAMOLE_DESKTOP_SESSION_MAX_SECS = '60'
$testDir = Join-Path ([IO.Path]::GetTempPath()) ('cracamole-rdp-gui-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testDir)
$desktopDir = Join-Path $testDir 'desktop'
$env:CRACAMOLE_DESKTOP_DATA_DIR = $desktopDir
$process = $null
$rsa = [Security.Cryptography.RSA]::Create(2048)
$request = [Security.Cryptography.X509Certificates.CertificateRequest]::new('CN=cracamole-rdp-fixture', $rsa, [Security.Cryptography.HashAlgorithmName]::SHA256, [Security.Cryptography.RSASignaturePadding]::Pkcs1)
$generated = $request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddDays(-1), [DateTimeOffset]::UtcNow.AddDays(1))
$pfx = $generated.Export([Security.Cryptography.X509Certificates.X509ContentType]::Pfx)
$cert = [Security.Cryptography.X509Certificates.X509Certificate2]::new($pfx, '', [Security.Cryptography.X509Certificates.X509KeyStorageFlags]::UserKeySet)
[Array]::Clear($pfx, 0, $pfx.Length)
$generated.Dispose()
$sha = [Security.Cryptography.SHA256]::Create()
$fingerprint = ([BitConverter]::ToString($sha.ComputeHash($cert.RawData))).Replace('-', '').ToLowerInvariant()
$sha.Dispose()
$fixture = [CertificateServer]::new($cert)
try {
    $process = Start-Process -FilePath $Binary -WindowStyle Hidden -PassThru
    Wait-For { [RdpGui]::Find([uint32]$process.Id, $null) -ne [IntPtr]::Zero } 'window not found' 15
    $window = [RdpGui]::Find([uint32]$process.Id, $null)
    if (-not $process.WaitForInputIdle(10000)) { throw 'controls timed out' }
    $children = [RdpGui]::Children($window)
    $inputs = @($children | Where-Object { [RdpGui]::Class($_) -eq 'Edit' })
    $list = @($children | Where-Object { [RdpGui]::Class($_) -eq 'ListBox' })[0]
    $combos = @($children | Where-Object { [RdpGui]::Class($_) -eq 'ComboBox' })
    if ($combos.Count -ne 4 -or $inputs.Count -ne 17) { throw 'RDP editor controls missing' }
    $protocol = $combos[0]
    $security = $combos[1]
    Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
    Click (Button '登入')
    Click (Button '新增')
    Set-Text $inputs[3] 'RDP fixture'
    [void][RdpGui]::SendMessageInt($protocol, 0x014E, [IntPtr]2, [IntPtr]::Zero)
    [void][RdpGui]::SendMessageInt($window, 0x0111, [IntPtr](0x10000 -bor [RdpGui]::GetDlgCtrlID($protocol)), $protocol)
    if (-not [RdpGui]::OwnVisible($inputs[10]) -or -not [RdpGui]::OwnVisible($security) -or [RdpGui]::OwnVisible($inputs[8])) { throw 'RDP protocol fields visibility incorrect' }
    if ([RdpGui]::SendMessageInt($security, 0x0147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 0) { throw 'RDP security did not default to NLA' }
    [void][RdpGui]::SendMessageInt($security, 0x014E, [IntPtr]1, [IntPtr]::Zero)
    Set-Text $inputs[10] 'EXAMPLE'
    Set-Text $inputs[4] '127.0.0.1'
    Set-Text $inputs[5] ([string]$fixture.Port)
    Set-Text $inputs[7] 'secret'
    Click (Button '儲存')
    $created = (Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value
    if ($created.params.security -ne 'tls' -or $created.params.domain -ne 'EXAMPLE') { throw 'RDP domain or security not saved' }
    [void][RdpGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    $verify = Button '核對身分'
    foreach ($decision in @(7, 6)) {
        [void][RdpGui]::PostMessage($verify, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-For {
            if ($fixture.Finished.IsFaulted) { throw $fixture.Finished.Exception.ToString() }
            [RdpGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero
        } 'certificate dialog not found'
        $dialog = [RdpGui]::Find([uint32]$process.Id, '#32770')
        Wait-For { (([RdpGui]::Children($dialog) | ForEach-Object { [RdpGui]::Text($_) }) -join "`n").Contains($fingerprint) } 'certificate dialog did not show expected fingerprint'
        $text = ([RdpGui]::Children($dialog) | ForEach-Object { [RdpGui]::Text($_) }) -join "`n"
        if (-not $text.Contains($fingerprint) -or -not $text.Contains('cracamole-rdp-fixture')) { throw 'certificate dialog did not show expected identity' }
        [void][RdpGui]::PostMessage($dialog, 0x0111, [IntPtr]$decision, [IntPtr]::Zero)
        Wait-For { [RdpGui]::Find([uint32]$process.Id, '#32770') -eq [IntPtr]::Zero } 'certificate dialog did not close'
        if ($decision -eq 7) {
            $saved = (Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value
            if ($saved.params.'server-cert-sha256') { throw 'rejected certificate was saved' }
        }
    }
    Wait-For { @($children | Where-Object { [RdpGui]::Text($_) -eq '已儲存主機身分。' }).Count -eq 1 } 'certificate was not saved'
    $saved = (Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value
    if ($saved.params.'server-cert-sha256' -ne $fingerprint -or $saved.params.password -ne 'secret' -or $saved.params.domain -ne 'EXAMPLE') { throw 'certificate, domain or password was not saved correctly' }
    [void][RdpGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    Click (Button '連線')
    Wait-For {
        if ($fixture.Finished.IsFaulted) { throw $fixture.Finished.Exception.ToString() }
        $fixture.Handshakes -ge 3
    } 'RDP TLS connection did not reach server'
    if ($fixture.LastProtocols -ne 1) { throw "TLS-only RDP requested unexpected protocols: $($fixture.LastProtocols)" }
    Wait-For { @($children | Where-Object { [RdpGui]::Text($_) -eq '遠端連線錯誤。請檢查連線設定；SSH 請使用「核對身分」確認主機金鑰。' }).Count -eq 1 } 'fixture connection did not report the expected upstream error'
    Click (Button '中斷連線')
    [void][RdpGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    Click (Button '編輯')
    $restoredDomain = [RdpGui]::EditText($inputs[10])
    $restoredSecurity = [RdpGui]::SendMessageInt($security, 0x0147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    if ($restoredDomain -ne 'EXAMPLE' -or $restoredSecurity -ne 1) { throw "RDP settings not restored: domain=$restoredDomain security=$restoredSecurity" }
    [void][RdpGui]::SendMessageInt($security, 0x014E, [IntPtr]2, [IntPtr]::Zero)
    Click (Button '儲存')
    $saved = (Get-Content -Raw -Encoding utf8 (Join-Path $desktopDir 'connections.json') | ConvertFrom-Json).PSObject.Properties.Value
    if ($saved.params.security -ne 'any' -or $saved.params.password -or $saved.params.'server-cert-sha256' -ne $fingerprint) { throw 'RDP security change did not clear the secret and preserve the certificate' }
    [void][RdpGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    Click $verify
    Wait-For { @($children | Where-Object { [RdpGui]::Text($_) -eq 'RDP 憑證與現有指紋相符。' }).Count -eq 1 } 'saved certificate did not match'
    [void][RdpGui]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $process.WaitForExit(10000)) { throw 'process failed to exit' }
    $process.Refresh()
    if ($process.ExitCode -ne 0) { throw 'process exited abnormally' }
    $file = Join-Path $desktopDir 'connections.json'
    $database = Get-Content -Raw -Encoding utf8 $file | ConvertFrom-Json
    $entry = $database.PSObject.Properties.Value
    $otherFingerprint = if ($fingerprint -eq ('0' * 64)) { '1' * 64 } else { '0' * 64 }
    $entry.params.'server-cert-sha256' = $otherFingerprint
    [IO.File]::WriteAllText($file, ($database | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $process = Start-Process -FilePath $Binary -WindowStyle Hidden -PassThru
    Wait-For { [RdpGui]::Find([uint32]$process.Id, $null) -ne [IntPtr]::Zero } 'restart window not found' 15
    $window = [RdpGui]::Find([uint32]$process.Id, $null)
    if (-not $process.WaitForInputIdle(10000)) { throw 'restart controls timed out' }
    $children = [RdpGui]::Children($window)
    $inputs = @($children | Where-Object { [RdpGui]::Class($_) -eq 'Edit' })
    $list = @($children | Where-Object { [RdpGui]::Class($_) -eq 'ListBox' })[0]
    Set-Text $inputs[1] $env:CRACAMOLE_ADMIN_PASSWORD
    Click (Button '登入')
    [void][RdpGui]::SendMessageInt($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    Click (Button '核對身分')
    Wait-For { @($children | Where-Object { [RdpGui]::Text($_) -eq 'RDP 憑證與現有指紋不符，已拒絕取代。' }).Count -eq 1 } 'mismatched certificate was not rejected'
    if ([RdpGui]::Find([uint32]$process.Id, '#32770') -ne [IntPtr]::Zero) { throw 'mismatched certificate prompted for replacement' }
    $saved = (Get-Content -Raw -Encoding utf8 $file | ConvertFrom-Json).PSObject.Properties.Value
    if ($saved.params.'server-cert-sha256' -ne $otherFingerprint) { throw 'mismatched certificate pin was overwritten' }
    if (-not $fixture.Finished.Wait(3000)) { throw 'certificate server did not complete five handshakes' }
    if ($fixture.Finished.IsFaulted) { throw $fixture.Finished.Exception }
    [void][RdpGui]::PostMessage($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $process.WaitForExit(10000)) { throw 'restart process failed to exit' }
    $process.Refresh()
    if ($process.ExitCode -ne 0) { throw 'restart process exited abnormally' }
} finally {
    if ($process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        if (-not $process.WaitForExit(10000)) { throw 'test process cleanup timed out' }
    }
    $fixture.Dispose(); $cert.Dispose(); $rsa.Dispose()
    foreach ($name in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnv[$name]) }
    if ([IO.Directory]::Exists($desktopDir)) {
        [IO.File]::Delete((Join-Path $desktopDir 'connections.json'))
        [IO.File]::Delete((Join-Path $desktopDir 'pinned-connections.json'))
        [IO.File]::Delete((Join-Path $desktopDir 'recent-connections.json'))
        [IO.File]::Delete((Join-Path $desktopDir 'desktop.lock'))
        [IO.Directory]::Delete($desktopDir)
    }
    if ([IO.Directory]::Exists($testDir)) { [IO.Directory]::Delete($testDir) }
}
