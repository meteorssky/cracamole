#![cfg(windows)]

use std::{os::windows::process::CommandExt, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test(flavor = "multi_thread")]
async fn native_telnet_connect_input_and_logout() {
    exercise(false, false, false, false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_telnet_remote_close_clears_display() {
    exercise(true, false, false, false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_telnet_session_expiry_closes_connection() {
    exercise(false, true, false, false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_telnet_save_and_connect() {
    exercise(false, false, true, false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_telnet_switch_requires_confirmation() {
    exercise(false, false, false, true, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_telnet_connect_once_without_persistence() {
    exercise(false, false, false, false, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_ssh_handshake_timeout_shows_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(20), socket.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert!(!bytes
            .windows(b"fixture-secret".len())
            .any(|part| part == b"fixture-secret"));
    });
    let mut command = tokio::process::Command::new("powershell.exe");
    command.as_std_mut().creation_flags(0x08000000);
    let output = tokio::time::timeout(
        Duration::from_secs(40),
        command
            .kill_on_drop(true)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/windows_telnet.ps1"),
                "-Binary",
                env!("CARGO_BIN_EXE_cracamole-desktop"),
                "-Port",
                &port.to_string(),
            ])
            .env("CRACAMOLE_TEST_TIMEOUT_STATUS", "1")
            .output(),
    )
    .await
    .expect("逾時狀態測試未結束")
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn native_ssh_unknown_hostname_shows_status() {
    let mut command = tokio::process::Command::new("powershell.exe");
    command.as_std_mut().creation_flags(0x08000000);
    let output = tokio::time::timeout(
        Duration::from_secs(35),
        command
            .kill_on_drop(true)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/windows_telnet.ps1"),
                "-Binary",
                env!("CARGO_BIN_EXE_cracamole-desktop"),
                "-Port",
                "22",
            ])
            .env("CRACAMOLE_TEST_NOT_FOUND_STATUS", "1")
            .output(),
    )
    .await
    .expect("找不到主機狀態測試未結束")
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn exercise(
    remote_close: bool,
    expire: bool,
    save_connect: bool,
    switch: bool,
    connect_once: bool,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let sync_dir = tempfile::tempdir().unwrap();
    let closed_marker = sync_dir.path().join("closed");
    let returned_marker = sync_dir.path().join("returned-connected");
    let reconnected_marker = sync_dir.path().join("reconnected-after-delete");
    let second = if switch {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let marker = sync_dir.path().join("second-connected");
        let closed = sync_dir.path().join("second-closed");
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            std::fs::write(marker, b"connected").unwrap();
            socket.write_all(b"second session\r\n").await.unwrap();
            let mut buffer = [0; 1024];
            while socket.read(&mut buffer).await.unwrap() != 0 {}
            std::fs::write(closed, b"closed").unwrap();
        });
        Some((port, task))
    } else {
        None
    };
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
        if switch {
            let (mut socket, _) = listener.accept().await?;
            std::fs::write(returned_marker, b"connected")?;
            socket.write_all(b"returned session\r\n").await?;
            while socket.read(&mut bytes).await? != 0 {}
            let (mut socket, _) = listener.accept().await?;
            std::fs::write(reconnected_marker, b"connected")?;
            socket.write_all(b"after delete\r\n").await?;
            while socket.read(&mut bytes).await? != 0 {}
        }
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
    if save_connect {
        command.env("CRACAMOLE_TEST_SAVE_CONNECT", "1");
    }
    if connect_once {
        command.env("CRACAMOLE_TEST_CONNECT_ONCE", "1");
    }
    if let Some((port, _)) = second.as_ref() {
        command.args(["-SwitchPort", &port.to_string()]);
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
    if let Some((_, task)) = second {
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("第二個工作階段未結束")
            .unwrap();
    }
}
