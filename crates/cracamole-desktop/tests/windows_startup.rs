#![cfg(windows)]

use std::{os::windows::process::CommandExt, time::Duration};

#[tokio::test]
async fn invalid_session_lifetime_reports_startup_failure_without_creating_data() {
    let mut command = tokio::process::Command::new("powershell.exe");
    command.as_std_mut().creation_flags(0x08000000);
    command.kill_on_drop(true).args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/windows_startup.ps1"),
        "-Binary",
        env!("CARGO_BIN_EXE_cracamole-desktop-win32"),
    ]);
    let output = tokio::time::timeout(Duration::from_secs(40), command.output())
        .await
        .expect("原生啟動設定測試逾時")
        .expect("無法啟動 PowerShell");
    assert!(
        output.status.success(),
        "原生啟動設定測試失敗：\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
