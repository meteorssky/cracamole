//! 使用獨立版本的 Rust RDP 測試伺服器驗證實際影格、輸入與關閉。
#![cfg(windows)]

use cracamole_desktop::driver::{self, Input};
use cracamole_store::Connection;
use std::{
    collections::BTreeMap,
    os::windows::process::CommandExt,
    process::{Child, Command},
    time::Duration,
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

struct Fixture(Child);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
#[ignore = "需先建置獨立的 tests/rdp-fixture 並設定 CRACAMOLE_RDP_FIXTURE"]
async fn nla_fixed_clipboard_fixture_records_only_outcomes() {
    let binary = std::env::var_os("CRACAMOLE_RDP_FIXTURE").expect("請指定 RDP fixture binary");
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture(
        Command::new(binary)
            .arg(directory.path())
            .arg("nla-frames-clipboard")
            .creation_flags(0x08000000)
            .spawn()
            .unwrap(),
    );
    let (port, pin) = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(directory.path().join("ready")) {
                let mut lines = text.lines();
                if let (Some(Ok(port)), Some(pin)) =
                    (lines.next().map(str::parse::<u16>), lines.next())
                {
                    break (port, pin.to_owned());
                }
            }
            assert!(fixture.0.try_wait().unwrap().is_none(), "fixture 提前結束");
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("fixture 啟動逾時");
    let connection = Connection {
        id: "fixture".into(),
        name: "fixture".into(),
        protocol: "rdp".into(),
        params: BTreeMap::from([
            ("hostname".into(), "127.0.0.1".into()),
            ("port".into(), port.to_string()),
            ("security".into(), "nla".into()),
            ("server-cert-sha256".into(), pin),
            ("username".into(), "fixture".into()),
            ("password".into(), "fixture-secret".into()),
        ]),
    };
    let (input, input_rx) = mpsc::channel(8);
    let (frames, mut frame_rx) = watch::channel(None);
    let (clipboard, _clipboard_rx) = watch::channel(None);
    let cancel = CancellationToken::new();
    let mut task = tokio::spawn({
        let cancel = cancel.clone();
        async move { driver::run(&connection, 320, 240, input_rx, frames, clipboard, cancel).await }
    });
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            tokio::select! {
                result = &mut task => panic!("NLA fixture 提前結束：{result:?}"),
                changed = frame_rx.changed() => {
                    changed.expect("畫面通道提前關閉");
                    if frame_rx.borrow().is_some() && directory.path().join("clipboard-ready").exists() { break; }
                }
            }
        }
    }).await.expect("NLA fixture 未完成剪貼簿初始化");
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    for (text, marker) in [
        ("unexpected fixture data", "clipboard-unexpected"),
        ("遠端測試🙂\n第二行", "clipboard-matched"),
    ] {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        input
            .send(Input::Paste {
                text: zeroize::Zeroizing::new(text.into()),
                permit,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while !directory.path().join(marker).exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fixture 未收到測試文字");
        assert!(
            !directory.path().join("clipboard").exists(),
            "fixture 不應保存實際剪貼簿文字"
        );
    }
    assert_eq!(
        std::fs::read(directory.path().join("clipboard-unexpected")).unwrap(),
        b"mismatch"
    );
    assert_eq!(
        std::fs::read(directory.path().join("clipboard-matched")).unwrap(),
        b"ok"
    );
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("NLA 工作階段未結束")
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[ignore = "需先建置獨立的 tests/rdp-fixture 並設定 CRACAMOLE_RDP_FIXTURE"]
async fn rdp_nla_authenticates_and_renders_with_pinned_certificate() {
    let binary = std::env::var_os("CRACAMOLE_RDP_FIXTURE").expect("請指定 RDP fixture binary");
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture(
        Command::new(binary)
            .arg(directory.path())
            .arg("nla-clipboard")
            .creation_flags(0x08000000)
            .spawn()
            .unwrap(),
    );
    let (port, fingerprint) = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(directory.path().join("ready")) {
                let mut lines = text.lines();
                if let (Some(Ok(port)), Some(pin)) =
                    (lines.next().map(str::parse::<u16>), lines.next())
                {
                    break (port, pin.to_owned());
                }
            }
            assert!(
                fixture.0.try_wait().unwrap().is_none(),
                "NLA 測試伺服器提前結束"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("NLA 測試伺服器啟動逾時");
    let connection = Connection {
        id: "nla-test".into(),
        name: "nla-test".into(),
        protocol: "rdp".into(),
        params: BTreeMap::from([
            ("hostname".into(), "127.0.0.1".into()),
            ("port".into(), port.to_string()),
            ("security".into(), "nla".into()),
            ("server-cert-sha256".into(), fingerprint),
            ("username".into(), "fixture".into()),
            ("password".into(), "fixture-secret".into()),
        ]),
    };
    for (index, wrong_pin) in [true, false].into_iter().enumerate() {
        let mut rejected = connection.clone();
        if wrong_pin {
            let pin = rejected.params.get_mut("server-cert-sha256").unwrap();
            pin.replace_range(..1, if pin.starts_with('0') { "1" } else { "0" });
        } else {
            rejected
                .params
                .insert("password".into(), "wrong-secret".into());
        }
        let (_input, input_rx) = mpsc::channel(4);
        let (frames, rejected_frames) = watch::channel(None);
        let (clipboard, _) = watch::channel(None);
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            driver::run(
                &rejected,
                320,
                240,
                input_rx,
                frames,
                clipboard,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("無效的 NLA 連線未結束");
        assert!(result.is_err(), "不符的指紋或密碼必須拒絕");
        assert!(rejected_frames.borrow().is_none(), "拒絕的連線不應收到畫面");
        let expected = (index + 1).to_string();
        tokio::time::timeout(Duration::from_secs(5), async {
            while std::fs::read_to_string(directory.path().join("completed"))
                .ok()
                .as_deref()
                != Some(expected.as_str())
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("測試伺服器未結束拒絕的 NLA 連線");
        assert!(
            !directory.path().join("closed").exists(),
            "無效連線不應建立桌面工作階段"
        );
    }
    let (input, input_rx) = mpsc::channel(8);
    let (frames_tx, mut frames) = watch::channel(None);
    let (clipboard, mut clipboard_rx) = watch::channel(None);
    let cancel = CancellationToken::new();
    let session_connection = connection.clone();
    let mut task = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            driver::run(
                &session_connection,
                320,
                240,
                input_rx,
                frames_tx,
                clipboard,
                cancel,
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            tokio::select! {
                result = &mut task => panic!("NLA 連線提前結束：{result:?}"),
                changed = frames.changed() => {
                    changed.expect("NLA 畫面通道提前關閉");
                    if frames.borrow().as_ref().is_some_and(|frame| frame.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel == &[0xd3, 0x71, 0x19, 0xff])) { break; }
                }
            }
        }
    }).await.expect("未收到 NLA 測試畫面");
    input
        .send(Input::Resize {
            width: 641,
            height: 481,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                result = &mut task => panic!("NLA 尺寸變更時連線提前結束：{result:?}"),
                changed = frames.changed() => {
                    changed.expect("NLA 畫面通道提前關閉");
                    if frames.borrow().as_ref().is_some_and(|frame| frame.width == 640 && frame.height == 481 && frame.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel == &[0x53, 0x71, 0x19, 0xff])) { break; }
                }
            }
        }
    }).await.expect("未收到 NLA 調整尺寸後的畫面");
    for pressed in [true, false] {
        input
            .send(Input::Key {
                keysym: 0xffc5,
                pressed,
            })
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            clipboard_rx
                .changed()
                .await
                .expect("NLA 剪貼簿通道提前關閉");
            if clipboard_rx.borrow().as_deref() == Some("調整後遠端🙂\n第二行") {
                break;
            }
        }
    })
    .await
    .expect("未收到 NLA 尺寸變更後的遠端剪貼簿");
    let permit = std::sync::Arc::new(tokio::sync::Semaphore::new(1))
        .acquire_owned()
        .await
        .unwrap();
    input
        .send(Input::Paste {
            text: zeroize::Zeroizing::new("本機測試🙂\n第二行".into()),
            permit,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while std::fs::read_to_string(directory.path().join("clipboard"))
            .ok()
            .as_deref()
            != Some("本機測試🙂\r\n第二行")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("NLA 伺服器未收到本機 Unicode 剪貼簿");
    input
        .send(Input::Key {
            keysym: u32::from(b'z'),
            pressed: true,
        })
        .await
        .unwrap();
    input
        .send(Input::Key {
            keysym: u32::from(b'z'),
            pressed: false,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !directory.path().join("key").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("NLA 測試伺服器未收到鍵盤輸入");
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("NLA 工作階段未結束")
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !directory.path().join("closed").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("NLA 連線未關閉");
    exercise_restrictions(&connection, &directory).await;
    exercise_auto_security(&connection, &directory).await;
}

#[tokio::test]
#[ignore = "需先建置獨立的 tests/rdp-fixture 並設定 CRACAMOLE_RDP_FIXTURE"]
async fn rdp_driver_renders_and_sends_input_to_loopback_server() {
    let binary = std::env::var_os("CRACAMOLE_RDP_FIXTURE").expect("請指定 RDP fixture binary");
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture(
        Command::new(binary)
            .arg(directory.path())
            .arg("clipboard")
            .creation_flags(0x08000000)
            .spawn()
            .unwrap(),
    );
    let ready = directory.path().join("ready");
    let (port, fingerprint) = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&ready) {
                let mut lines = text.lines();
                if let (Some(port), Some(pin)) = (lines.next(), lines.next()) {
                    if let Ok(port) = port.parse::<u16>() {
                        break (port, pin.to_string());
                    }
                }
            }
            assert!(
                fixture.0.try_wait().unwrap().is_none(),
                "RDP 測試伺服器提前結束"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("RDP 測試伺服器啟動逾時");
    let connection = Connection {
        id: "rdp-test".into(),
        name: "rdp-test".into(),
        protocol: "rdp".into(),
        params: BTreeMap::from([
            ("hostname".into(), "127.0.0.1".into()),
            ("port".into(), port.to_string()),
            ("security".into(), "tls".into()),
            ("server-cert-sha256".into(), fingerprint),
            ("username".into(), "fixture".into()),
            ("password".into(), "fixture-secret".into()),
        ]),
    };
    let mut untrusted = connection.clone();
    let pin = untrusted.params.get_mut("server-cert-sha256").unwrap();
    pin.replace_range(..1, if pin.starts_with('0') { "1" } else { "0" });
    let (_untrusted_input, untrusted_rx) = mpsc::channel(4);
    let (untrusted_frames, _) = watch::channel(None);
    let (untrusted_clipboard, _) = watch::channel(None);
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        driver::run(
            &untrusted,
            320,
            240,
            untrusted_rx,
            untrusted_frames,
            untrusted_clipboard,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("錯誤憑證指紋的連線未結束");
    assert!(result.is_err(), "RDP 客戶端應拒絕不符的憑證指紋");
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::fs::read_to_string(directory.path().join("completed"))
            .ok()
            .as_deref()
            != Some("1")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("伺服器未完成處理不受信任的連線");
    assert!(
        !directory.path().join("authenticated").exists()
            && !directory.path().join("rejected").exists(),
        "憑證指紋不符時不可將帳密送到伺服器"
    );
    let mut wrong = connection.clone();
    wrong
        .params
        .insert("password".into(), "wrong-secret".into());
    let (_wrong_input, wrong_rx) = mpsc::channel(4);
    let (wrong_frames, _) = watch::channel(None);
    let (wrong_clipboard, _) = watch::channel(None);
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        driver::run(
            &wrong,
            320,
            240,
            wrong_rx,
            wrong_frames,
            wrong_clipboard,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("錯誤密碼的連線未結束");
    assert!(result.is_err(), "RDP 測試伺服器應拒絕錯誤密碼");
    assert!(
        directory.path().join("rejected").exists(),
        "錯誤密碼未到達測試伺服器驗證器"
    );
    assert!(!directory.path().join("authenticated").exists());
    let (input, input_rx) = mpsc::channel(16);
    let (frames_tx, mut frames) = watch::channel(None);
    let (clipboard, mut clipboard_rx) = watch::channel(None);
    let cancel = CancellationToken::new();
    let session_connection = connection.clone();
    let mut driver = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            driver::run(
                &session_connection,
                320,
                240,
                input_rx,
                frames_tx,
                clipboard,
                cancel,
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            tokio::select! {
                result = &mut driver => panic!("RDP 驅動提早結束：{result:?}"),
                changed = frames.changed() => {
                    changed.expect("RDP 畫面通道提前關閉");
                    if frames.borrow().as_ref().is_some_and(|frame| frame.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel == &[0xd3, 0x71, 0x19, 0xff])) { break; }
                }
            }
        }
    }).await.expect("未收到已解碼的 RDP 測試影格");
    assert!(
        directory.path().join("authenticated").exists(),
        "RDP 測試伺服器未驗證帳密"
    );
    input
        .send(Input::Resize {
            width: 641,
            height: 481,
        })
        .await
        .unwrap();
    input
        .send(Input::Resize {
            width: 803,
            height: 601,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while std::fs::read_to_string(directory.path().join("layout"))
            .ok()
            .as_deref()
            != Some("802x601")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("伺服器未收到最終 RDP 視窗尺寸");
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                result = &mut driver => panic!("RDP 尺寸重新協商時連線結束：{result:?}"),
                changed = frames.changed() => {
                    changed.expect("RDP 畫面通道提前關閉");
                    if frames.borrow().as_ref().is_some_and(|frame| frame.width == 802 && frame.height == 601 && frame.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel == &[0x53, 0x71, 0x19, 0xff])) { break; }
                }
            }
        }
    }).await.expect("未收到重新協商後的新尺寸 RDP 畫面");
    input
        .send(Input::Resize {
            width: 401,
            height: 301,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                result = &mut driver => panic!("RDP 畫面縮小時連線結束：{result:?}"),
                changed = frames.changed() => {
                    changed.expect("RDP 畫面通道提前關閉");
                    if frames.borrow().as_ref().is_some_and(|frame| frame.width == 400 && frame.height == 301 && frame.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel == &[0x73, 0x71, 0x19, 0xff])) { break; }
                }
            }
        }
    }).await.expect("未收到縮小後的新尺寸 RDP 畫面");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            clipboard_rx.changed().await.expect("剪貼簿通道提前關閉");
            if clipboard_rx.borrow().as_deref() == Some("遠端測試🙂\n第二行") {
                break;
            }
        }
    })
    .await
    .expect("未收到遠端 Unicode 剪貼簿");
    input
        .send(Input::Key {
            keysym: 0xffc5,
            pressed: true,
        })
        .await
        .unwrap();
    input
        .send(Input::Key {
            keysym: 0xffc5,
            pressed: false,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            clipboard_rx.changed().await.expect("剪貼簿通道提前關閉");
            if clipboard_rx.borrow().as_deref() == Some("調整後遠端🙂\n第二行") {
                break;
            }
        }
    })
    .await
    .expect("重新協商後未收到新的遠端剪貼簿");
    let permit = std::sync::Arc::new(tokio::sync::Semaphore::new(1))
        .acquire_owned()
        .await
        .unwrap();
    input
        .send(Input::Paste {
            text: zeroize::Zeroizing::new("本機測試🙂\n第二行".into()),
            permit,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if std::fs::read_to_string(directory.path().join("clipboard"))
                .ok()
                .as_deref()
                == Some("本機測試🙂\r\n第二行")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("伺服器未收到本機 Unicode 剪貼簿");
    input
        .send(Input::Key {
            keysym: u32::from(b'z'),
            pressed: true,
        })
        .await
        .unwrap();
    input
        .send(Input::Key {
            keysym: u32::from(b'z'),
            pressed: false,
        })
        .await
        .unwrap();
    input
        .send(Input::Mouse {
            x: 42,
            y: 31,
            mask: 1,
        })
        .await
        .unwrap();
    input
        .send(Input::Mouse {
            x: 42,
            y: 31,
            mask: 0,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !directory.path().join("key").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("RDP 伺服器未收到按鍵");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !directory.path().join("mouse").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("RDP 伺服器未收到滑鼠移動及按下／放開");
    cancel.cancel();
    assert!(tokio::time::timeout(Duration::from_secs(5), driver)
        .await
        .unwrap()
        .unwrap()
        .is_ok());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !directory.path().join("closed").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("取消後 RDP socket 未關閉");
    exercise_restrictions(&connection, &directory).await;
    exercise_auto_security(&connection, &directory).await;
}

async fn exercise_auto_security(connection: &Connection, directory: &tempfile::TempDir) {
    std::fs::remove_file(directory.path().join("closed")).unwrap();
    let mut automatic = connection.clone();
    automatic.params.insert("security".into(), "any".into());
    let (_input, input_rx) = mpsc::channel(8);
    let (frames_tx, mut frames) = watch::channel(None);
    let (clipboard, _) = watch::channel(None);
    let cancel = CancellationToken::new();
    let mut task = tokio::spawn({
        let cancel = cancel.clone();
        async move { driver::run(&automatic, 320, 240, input_rx, frames_tx, clipboard, cancel).await }
    });
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                result = &mut task => panic!("自動協商連線提前結束：{result:?}"),
                result = frames.changed() => {
                    result.expect("自動協商畫面通道提前關閉");
                    if frames.borrow().as_ref().is_some_and(|frame| frame.width == 640 && frame.height == 480 && frame.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel == &[0x53, 0x71, 0x19, 0xff])) { break; }
                }
            }
        }
    }).await.expect("自動協商未收到伺服器畫面");
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("自動協商工作階段未結束")
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !directory.path().join("closed").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("自動協商連線未關閉");
}

async fn exercise_restrictions(connection: &Connection, directory: &tempfile::TempDir) {
    for (disable_copy, disable_paste, read_only, width, height, red) in [
        (true, false, false, 640, 480, 0x53),
        (false, true, false, 400, 301, 0x73),
        (false, false, true, 640, 480, 0x53),
    ] {
        for name in [
            "clipboard",
            "clipboard-ready",
            "closed",
            "key",
            "mouse",
            "copy-trigger",
        ] {
            let _ = std::fs::remove_file(directory.path().join(name));
        }
        let mut restricted = connection.clone();
        restricted
            .params
            .insert("disable-copy".into(), disable_copy.to_string());
        restricted
            .params
            .insert("disable-paste".into(), disable_paste.to_string());
        restricted
            .params
            .insert("read-only".into(), read_only.to_string());
        let (input, input_rx) = mpsc::channel(16);
        let (frames_tx, mut frames) = watch::channel(None);
        let (clipboard_tx, mut clipboard_rx) = watch::channel(None);
        let cancel = CancellationToken::new();
        let mut driver = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                driver::run(
                    &restricted,
                    320,
                    240,
                    input_rx,
                    frames_tx,
                    clipboard_tx,
                    cancel,
                )
                .await
            }
        });
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                tokio::select! {
                    result = &mut driver => panic!("受限 RDP 連線提早結束：{result:?}"),
                    result = frames.changed() => {
                        result.expect("畫面通道提前關閉");
                        if frames.borrow().is_some() { break; }
                    }
                }
            }
            while !directory.path().join("clipboard-ready").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("受限 RDP 連線未完成剪貼簿協商");
        input.send(Input::Resize { width, height }).await.unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                tokio::select! {
                    result = &mut driver => panic!("受限 RDP 重新協商時連線結束：{result:?}"),
                    result = frames.changed() => {
                        result.expect("畫面通道提前關閉");
                        if frames.borrow().as_ref().is_some_and(|frame| frame.width == width as usize && frame.height == height as usize && frame.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel == &[red, 0x71, 0x19, 0xff])) { break; }
                    }
                }
            }
        }).await.expect("受限 RDP 未收到重新協商後的畫面");
        if !read_only {
            input
                .send(Input::Key {
                    keysym: 0xffc5,
                    pressed: true,
                })
                .await
                .unwrap();
            input
                .send(Input::Key {
                    keysym: 0xffc5,
                    pressed: false,
                })
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !directory.path().join("copy-trigger").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("伺服器未觸發重新協商後的遠端複製");
        }
        let permit = std::sync::Arc::new(tokio::sync::Semaphore::new(1))
            .acquire_owned()
            .await
            .unwrap();
        input
            .send(Input::Paste {
                text: zeroize::Zeroizing::new("受限測試".into()),
                permit,
            })
            .await
            .unwrap();
        input
            .send(Input::Key {
                keysym: u32::from(b'z'),
                pressed: true,
            })
            .await
            .unwrap();
        input
            .send(Input::Key {
                keysym: u32::from(b'z'),
                pressed: false,
            })
            .await
            .unwrap();
        input
            .send(Input::Mouse {
                x: 42,
                y: 31,
                mask: 1,
            })
            .await
            .unwrap();
        input
            .send(Input::Mouse {
                x: 42,
                y: 31,
                mask: 0,
            })
            .await
            .unwrap();
        if disable_copy {
            assert!(
                tokio::time::timeout(Duration::from_secs(1), clipboard_rx.changed())
                    .await
                    .is_err(),
                "停用複製仍收到遠端文字"
            );
        } else {
            let expected = if read_only {
                "遠端測試🙂\n第二行"
            } else {
                "調整後遠端🙂\n第二行"
            };
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    clipboard_rx.changed().await.expect("剪貼簿通道提前關閉");
                    if clipboard_rx.borrow().as_deref() == Some(expected) {
                        break;
                    }
                }
            })
            .await
            .expect("受限連線未收到允許的遠端剪貼簿");
        }
        if disable_paste || read_only {
            tokio::time::sleep(Duration::from_secs(1)).await;
            assert!(
                !directory.path().join("clipboard").exists(),
                "停用貼上仍送出本機文字"
            );
        } else {
            tokio::time::timeout(Duration::from_secs(5), async {
                while std::fs::read_to_string(directory.path().join("clipboard"))
                    .ok()
                    .as_deref()
                    != Some("受限測試")
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("啟用貼上時伺服器未收到文字");
        }
        if read_only {
            assert!(
                !directory.path().join("key").exists(),
                "唯讀模式仍送出鍵盤輸入"
            );
            assert!(
                !directory.path().join("mouse").exists(),
                "唯讀模式仍送出滑鼠輸入"
            );
        } else {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !directory.path().join("key").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("一般模式未送出鍵盤輸入");
            tokio::time::timeout(Duration::from_secs(5), async {
                while !directory.path().join("mouse").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("一般模式未送出滑鼠輸入");
        }
        cancel.cancel();
        assert!(tokio::time::timeout(Duration::from_secs(5), driver)
            .await
            .unwrap()
            .unwrap()
            .is_ok());
        tokio::time::timeout(Duration::from_secs(5), async {
            while !directory.path().join("closed").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("受限 RDP socket 未關閉");
    }
}
