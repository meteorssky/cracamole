# 啟動現代化的 Windows 桌面介面（GPUI 版，位於 desktop-gpui/）。
#
# 在 PowerShell 中隱藏輸入管理員密碼（至少 12 字元），只設定在啟動程序的環境變數；
# 程式結束後還原原本的值。第一次執行或加上 -Rebuild 時會建置（約數分鐘）。
#
#   .\scripts\run_desktop_gpui.ps1
#   .\scripts\run_desktop_gpui.ps1 -Release
param(
    [switch]$Release,
    [switch]$Rebuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$profileDir = if ($Release) { 'release' } else { 'debug' }
$binary = Join-Path $root "desktop-gpui\target\$profileDir\cracamole-desktop-gpui.exe"

if ($Rebuild -or -not (Test-Path $binary)) {
    $arguments = @('build', '--manifest-path', (Join-Path $root 'desktop-gpui\Cargo.toml'))
    if ($Release) { $arguments += '--release' }
    # GPUI 的元件樹很深，rustc 預設堆疊可能不足。
    $env:RUST_MIN_STACK = '536870912'
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw '建置失敗' }
}

$secure = Read-Host '請輸入管理員密碼（至少 12 字元）' -AsSecureString
$previous = [Environment]::GetEnvironmentVariable('CRACAMOLE_ADMIN_PASSWORD', 'Process')
$pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
try {
    $env:CRACAMOLE_ADMIN_PASSWORD = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer)
    & $binary
}
finally {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer)
    [Environment]::SetEnvironmentVariable('CRACAMOLE_ADMIN_PASSWORD', $previous, 'Process')
}
