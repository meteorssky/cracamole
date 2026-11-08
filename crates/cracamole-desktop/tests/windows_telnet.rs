#![cfg(windows)]

use std::{os::windows::process::CommandExt, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test(flavor = "multi_thread")]
async fn native_telnet_connect_input_and_logout() {
    exercise(false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_telnet_remote_close_clears_display() {
    exercise(true, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_telnet_session_expiry_closes_connection() {
    exercise(false, true).await;
}

async fn exercise(remote_close: bool, expire: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let sync_dir = tempfile::tempdir().unwrap();
    let closed_marker = sync_dir.path().join("closed");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket
            .write_all(b"\x1b[37;41mMMMMMMMMMMMMMMMMMMMM\r\nMMMMMMMMMMMMMMMMMMMM\r\nMMMMMMMMMMMMMMMMMMMM\x1b[0m\r\n")
            .await
            .unwrap();
        let mut received = Vec::new();
        let mut bytes = [0; 1024];
        loop {
            let count = socket.read(&mut bytes).await?;
            if count == 0 {
                break;
            }
            received.extend_from_slice(&bytes[..count]);
            assert!(received.len() < 1024 * 1024);
            if remote_close
                && received
                    .windows("z中".len())
                    .any(|part| part == "z中".as_bytes())
            {
                socket.shutdown().await?;
                break;
            }
        }
        std::fs::write(closed_marker, b"closed")?;
        Ok::<_, std::io::Error>(received)
    });
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/windows_telnet.ps1");
    let mut command = tokio::process::Command::new("powershell.exe");
    command.as_std_mut().creation_flags(0x08000000);
    command
        .kill_on_drop(true)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            script,
            "-Binary",
            env!("CARGO_BIN_EXE_cracamole-desktop"),
            "-Port",
            &port.to_string(),
        ])
        .arg("-SyncDir")
        .arg(sync_dir.path());
    if remote_close {
        command.arg("-RemoteClose");
    }
    if expire {
        command.arg("-Expire");
    }
    let output = tokio::time::timeout(Duration::from_secs(60), command.output())
        .await
        .expect("Telnet 原生視窗測試逾時")
        .expect("無法啟動 PowerShell");
    assert!(
        output.status.success(),
        "Telnet 原生視窗測試失敗：\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if std::env::var("CRACAMOLE_TEST_VISIBLE_TELNET").as_deref() == Ok("1") {
        println!("已確認可見的 Telnet 測試文字與背景。");
    }
    let received = tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("登出後 Telnet 連線未關閉")
        .unwrap()
        .expect("Telnet 伺服器讀取連線失敗");
    assert!(
        received
            .windows("z中".len())
            .any(|window| window == "z中".as_bytes()),
        "Telnet 伺服器未收到預期輸入"
    );
}
