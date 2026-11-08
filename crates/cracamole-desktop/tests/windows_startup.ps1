param([string]$Binary)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class StartupDialogTest {
    public delegate bool Callback(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Callback c, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, Callback c, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    public static string Text(IntPtr h) { var s = new StringBuilder(2048); GetWindowText(h,s,s.Capacity); return s.ToString(); }
    public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassName(h,s,s.Capacity); return s.ToString(); }
    public static IntPtr Dialog(uint pid) {
        IntPtr result=IntPtr.Zero;
        EnumWindows((h,p) => { uint owner; GetWindowThreadProcessId(h,out owner); if (owner == pid && Class(h) == "#32770") { result=h; return false; } return true; },IntPtr.Zero);
        return result;
    }
    public static string ChildText(IntPtr parent) {
        var values = new List<string>();
        EnumChildWindows(parent,(h,p) => { values.Add(Text(h)); return true; },IntPtr.Zero);
        return String.Join("\n",values);
    }
}
'@
$env:CRACAMOLE_ADMIN_USERNAME = 'test-admin'
$env:CRACAMOLE_ADMIN_PASSWORD = 'desktop-test-password-123'
$env:CRACAMOLE_DESKTOP_SESSION_MAX_SECS = 'typo'
$testDir = Join-Path ([IO.Path]::GetTempPath()) ('cracamole-startup-' + [Guid]::NewGuid().ToString('N'))
$env:CRACAMOLE_DESKTOP_DATA_DIR = $testDir
$process = $null
try {
    $process = Start-Process -FilePath $Binary -WindowStyle Hidden -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    $dialog = [IntPtr]::Zero
    while ($dialog -eq [IntPtr]::Zero -and -not $process.HasExited -and [DateTime]::UtcNow -lt $deadline) {
        $dialog = [StartupDialogTest]::Dialog([uint32]$process.Id)
        if ($dialog -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 50 }
    }
    if ($dialog -eq [IntPtr]::Zero) { throw 'startup error dialog missing' }
    $message = [StartupDialogTest]::ChildText($dialog)
    if (-not $message.Contains('CRACAMOLE_DESKTOP_SESSION_MAX_SECS') -or -not $message.Contains('28800')) { throw 'startup error dialog has incorrect content' }
    if ([IO.Directory]::Exists($testDir)) { throw 'invalid configuration created the data directory' }
    [void][StartupDialogTest]::PostMessage($dialog, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $process.WaitForExit(10000)) { throw 'startup failure did not exit' }
    $process.Refresh()
    if ($process.ExitCode -ne 1) { throw "startup failure returned exit code $($process.ExitCode)" }
} finally {
    if ($process -and -not $process.HasExited) { Stop-Process -Id $process.Id -Force; [void]$process.WaitForExit(10000) }
}
