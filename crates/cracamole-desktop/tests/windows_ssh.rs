#![cfg(windows)]

#[path = "../../proto-ssh/tests/common/mod.rs"]
mod ssh_fixture;

use russh::keys::HashAlg;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需啟動專用 Rust Web 服務並安裝 Playwright"]
async fn browser_shared_ssh_composited_terminal() {
    let url = std::env::var("CRACAMOLE_URL").expect("請指定專用本機 Web 服務 URL");
    assert!(
        url.starts_with("http://127.0.0.1:"),
        "僅允許本機 Web 測試服務"
    );
    let server = ssh_fixture::start_server(ssh_fixture::Behaviour::default()).await;
    let key = server.host_key.public_key().to_openssh().unwrap();
    let test_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/e2e");
    let mut command = tokio::process::Command::new("node.exe");
    command.as_std_mut().creation_flags(0x08000000);
    command
        .kill_on_drop(true)
        .current_dir(test_dir)
        .args([
            "node_modules/@playwright/test/cli.js",
            "test",
            "local-ssh.spec.js",
            "--project=chromium",
            "--timeout=30000",
        ])
        .env("CRACAMOLE_URL", url)
        .env("CRACAMOLE_LOCAL_SSH_PORT", server.addr.port().to_string())
        .env("CRACAMOLE_LOCAL_SSH_KEY", key);
    let output = tokio::time::timeout(Duration::from_secs(90), command.output())
        .await
        .expect("瀏覽器 SSH 測試逾時")
        .expect("無法啟動 Playwright");
    assert!(
        output.status.success(),
        "瀏覽器 SSH 測試失敗：\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
    let received = &server.rec.lock().unwrap().received;
    assert!(
        received
            .windows(b"browser-test".len())
            .any(|bytes| bytes == b"browser-test"),
        "瀏覽器輸入未送達 SSH fixture"
    );
    let mobile = "行動輸入🙂".as_bytes();
    for text in [
        b"fixture-quick-paste".as_slice(),
        b"fixture-panel-paste".as_slice(),
    ] {
        assert!(received.windows(text.len()).any(|window| window == text));
    }
    assert_eq!(
        received
            .windows(mobile.len())
            .filter(|bytes| *bytes == mobile)
            .count(),
        1,
        "合成輸入法文字未送達或重複送達 SSH fixture"
    );
    assert_eq!(received.iter().filter(|&&byte| byte == 0x7f).count(), 1);
    assert!(
        !received.contains(&0x0b),
        "Ctrl+K 不應傳送至遠端 SSH 工作階段"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_ssh_trust_and_connection() {
    exercise(false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_ssh_private_key_connection() {
    exercise(true, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_ssh_big5_connection() {
    exercise(false, true).await;
}

async fn exercise(private_key_auth: bool, big5: bool) {
    let client_key = ssh_fixture::host_key();
    let mut behaviour = ssh_fixture::Behaviour::default();
    if std::env::var("CRACAMOLE_TEST_VISIBLE_SSH").as_deref() == Ok("1") {
        behaviour.greeting = "\x1b[37;41mMMMMMMMMMMMMMMMMMMMM\r\nMMMMMMMMMMMMMMMMMMMM\r\nMMMMMMMMMMMMMMMMMMMM\x1b[0m\r\n".into();
    }
    if private_key_auth {
        behaviour
            .authorized_keys
            .push(client_key.public_key().clone());
        behaviour.password = None;
    }
    let server = ssh_fixture::start_server(behaviour).await;
    let public = server.host_key.public_key();
    let fingerprint = public.fingerprint(HashAlg::Sha256).to_string();
    let key = public.to_openssh().unwrap();
    let mismatched_key = ssh_fixture::host_key().public_key().to_openssh().unwrap();
    let port = server.addr.port().to_string();
    let binary = std::env::var_os("CRACAMOLE_TEST_DESKTOP_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_cracamole-desktop").into())
        .canonicalize()
        .expect("找不到指定的原生桌面測試執行檔");
    let synchronization = tempfile::tempdir().unwrap();
    let sync_dir = synchronization.path().to_path_buf();
    let private_key_path = sync_dir.join("client-key");
    if private_key_auth {
        let encrypted = client_key
            .encrypt(&mut rand::rng(), "fixture-passphrase")
            .unwrap();
        let encoded = encrypted
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .unwrap();
        std::fs::write(&private_key_path, encoded.as_bytes()).unwrap();
    }
    let script_sync_dir = sync_dir.clone();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/windows_ssh.ps1");
    let task = tokio::task::spawn_blocking(move || {
        Command::new("powershell.exe")
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                script,
            ])
            .arg("-Binary")
            .arg(binary)
            .arg("-Port")
            .arg(port)
            .arg("-Fingerprint")
            .arg(fingerprint)
            .arg("-PublicKey")
            .arg(key)
            .arg("-MismatchedKey")
            .arg(mismatched_key)
            .arg("-PrivateKeyPath")
            .arg(if private_key_auth {
                private_key_path.into_os_string()
            } else {
                Default::default()
            })
            .arg("-SyncDir")
            .arg(script_sync_dir)
            .arg("-EncodingName")
            .arg(if big5 { "Big5" } else { "UTF-8" })
            .output()
            .expect("啟動 Windows 原生 SSH 測試")
    });
    let once_rejected = sync_dir.join("once-untrusted-rejected");
    tokio::time::timeout(Duration::from_secs(30), async {
        while !once_rejected.exists() && !task.is_finished() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("等待一次性 SSH 指紋拒絕逾時");
    if once_rejected.exists() {
        assert!(
            server.rec.lock().unwrap().auth_methods_tried.is_empty(),
            "核對前不應傳送驗證資訊"
        );
        std::fs::write(sync_dir.join("continue-once"), b"").unwrap();
    }
    let once_trusted = sync_dir.join("once-trusted");
    tokio::time::timeout(Duration::from_secs(30), async {
        while !once_trusted.exists() && !task.is_finished() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("等待一次性 SSH 已信任連線逾時");
    if once_trusted.exists() {
        tokio::time::timeout(Duration::from_secs(15), async {
            while server.rec.lock().unwrap().auth_methods_tried.is_empty() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("核對後的一次性 SSH 未驗證");
        std::fs::write(sync_dir.join("continue-after-once"), b"").unwrap();
    }
    let after_once = server.rec.lock().unwrap().auth_methods_tried.clone();
    let ready = sync_dir.join("untrusted-rejected");
    tokio::time::timeout(Duration::from_secs(30), async {
        while !ready.exists() && !task.is_finished() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("等待未知主機測試逾時");
    if ready.exists() {
        assert!(
            server.rec.lock().unwrap().auth_methods_tried == after_once,
            "信任前不應傳送驗證資訊"
        );
        std::fs::write(sync_dir.join("continue"), b"").unwrap();
    }
    let trusted_done = sync_dir.join("trusted-complete");
    tokio::time::timeout(Duration::from_secs(30), async {
        while !trusted_done.exists() && !task.is_finished() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("等待已信任連線測試逾時");
    let authenticated_before_mismatch = server.rec.lock().unwrap().auth_methods_tried.clone();
    if trusted_done.exists() {
        std::fs::write(sync_dir.join("continue-mismatch"), b"").unwrap();
    }
    let output = tokio::time::timeout(Duration::from_secs(60), task)
        .await
        .expect("原生 SSH 測試逾時")
        .expect("原生 SSH 測試工作失敗");
    assert!(
        output.status.success(),
        "PowerShell 測試失敗：\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if std::env::var("CRACAMOLE_TEST_VISIBLE_SSH").as_deref() == Ok("1") {
        println!("已確認可見的 SSH 測試文字與背景。");
    }
    let recorded = server.rec.lock().unwrap();
    assert_eq!(
        recorded.auth_methods_tried, authenticated_before_mismatch,
        "金鑰不符時不應傳送驗證資訊"
    );
    let expected_method = if private_key_auth {
        "publickey"
    } else {
        "password"
    };
    assert!(
        recorded
            .auth_methods_tried
            .iter()
            .any(|method| method == expected_method),
        "未使用預期方法登入 SSH 測試伺服器：{expected_method}"
    );
    assert!(recorded.shell, "未開啟 SSH shell");
    assert!(
        recorded.pty.as_ref().is_some_and(|(_, cols, rows)| recorded
            .window_changes
            .iter()
            .any(|(new_cols, new_rows)| new_cols > cols && new_rows > rows)),
        "放大視窗後未將更大的終端機尺寸送至 SSH 伺服器"
    );
    assert!(
        recorded
            .window_changes
            .windows(2)
            .any(|changes| changes[1].0 < changes[0].0 && changes[1].1 < changes[0].1),
        "縮小視窗後未將較小的終端機尺寸送至 SSH 伺服器"
    );
    assert!(
        recorded.received.contains(&b'z'),
        "原生輸入未送達 SSH shell"
    );
    let expected: &[u8] = if big5 {
        &[0xa4, 0xa4]
    } else {
        "中🙂".as_bytes()
    };
    assert!(
        recorded
            .received
            .windows(expected.len())
            .any(|bytes| bytes == expected),
        "所選文字編碼未正確送達 SSH shell"
    );
}
