#![cfg(windows)]

use std::os::windows::process::CommandExt;
use std::process::Command;

#[test]
fn native_rdp_certificate_confirmation() {
    let output = Command::new("powershell.exe")
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/windows_rdp.ps1"),
            "-Binary",
            env!("CARGO_BIN_EXE_cracamole-desktop"),
        ])
        .output()
        .expect("啟動 Windows 原生 RDP 憑證測試");
    assert!(
        output.status.success(),
        "RDP 憑證測試失敗：\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
