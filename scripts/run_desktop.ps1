# 啟動 Windows 桌面介面。預設是現代化的 GPUI 版（desktop-gpui/）；
# 加上 -Legacy 改啟動 Win32 原生版（備援，cracamole-desktop）。
#
# 在 PowerShell 中隱藏輸入管理員密碼（至少 12 字元），只設定在啟動程序的環境變數；
# 程式結束後還原原本的值。第一次執行或加上 -Rebuild 時會建置（約數分鐘）。
#
#   .\scripts\run_desktop.ps1            # GPUI 版
#   .\scripts\run_desktop.ps1 -Release
#   .\scripts\run_desktop.ps1 -Legacy    # Win32 版
param(
    [switch]$Release,
    [switch]$Rebuild,
    [switch]$Legacy
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$profileDir = if ($Release) { 'release' } else { 'debug' }
if ($Legacy) {
    $binary = Join-Path $root "target\$profileDir\cracamole-desktop.exe"
    $build = @('build', '-p', 'cracamole-desktop', '--locked')
} else {
    $binary = Join-Path $root "desktop-gpui\target\$profileDir\cracamole-desktop-gpui.exe"
    $build = @('build', '--manifest-path', (Join-Path $root 'desktop-gpui\Cargo.toml'))
}

if ($Rebuild -or -not (Test-Path $binary)) {
    if ($Release) { $build += '--release' }
    # GPUI 的元件樹很深，rustc 預設堆疊可能不足。
    $env:RUST_MIN_STACK = '536870912'
    Push-Location $root
    try { & cargo @build } finally { Pop-Location }
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
