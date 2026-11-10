use cracamole_desktop::driver::{self, Input};
use cracamole_store::Connection;
use std::{collections::BTreeMap, time::Duration};

#[tokio::test]
async fn unknown_ssh_host_reports_upstream_not_found() {
    let connection = Connection {
        id: "missing".into(),
        name: "missing".into(),
        protocol: "ssh".into(),
        params: BTreeMap::from([
            ("hostname".into(), "no-such-host.invalid".into()),
            ("username".into(), "fixture".into()),
            ("password".into(), "fixture-secret".into()),
        ]),
    };
    let (_input, receiver) = tokio::sync::mpsc::channel(4);
    let (frames, _) = tokio::sync::watch::channel(None);
    let (clipboard, _) = tokio::sync::watch::channel(None);
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        driver::run(
            &connection,
            640,
            480,
            receiver,
            frames,
            clipboard,
            tokio_util::sync::CancellationToken::new(),
        ),
    )
    .await
    .expect("主機名稱解析未完成");
    assert!(
        matches!(
            result,
            Err(driver::DriverError::Remote(
                guac_protocol::Status::UpstreamNotFound
            ))
        ),
        "unexpected result: {result:?}"
    );
}
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, watch},
};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn native_driver_renders_terminal_forwards_key_and_cancels_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (observed, observed_rx) = tokio::sync::oneshot::channel();
    let (paste_observed, paste_rx) = tokio::sync::oneshot::channel();
    let fixture = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket
            .write_all(b"\x1b[?2004hDesktop input test\r\n")
            .await
            .unwrap();
        let mut seen = Some(observed);
        let mut paste_seen = Some(paste_observed);
        let mut collected = Vec::new();
        let mut bytes = [0u8; 1024];
        loop {
            let count = socket.read(&mut bytes).await.unwrap();
            if count == 0 {
                break;
            }
            collected.extend_from_slice(&bytes[..count]);
            let expected = b"\x1b[200~alpha[201~HACK\r\x1b[201~";
            if collected
                .windows(expected.len())
                .any(|window| window == expected)
            {
                if let Some(sender) = paste_seen.take() {
                    let _ = sender.send(());
                }
            }
            if bytes[..count].contains(&b'a') {
                if let Some(sender) = seen.take() {
                    let _ = sender.send(());
                }
            }
        }
    });
    let connection = Connection {
        id: "test".into(),
        name: "test".into(),
        protocol: "telnet".into(),
        params: BTreeMap::from([
            ("hostname".into(), "127.0.0.1".into()),
            ("port".into(), port.to_string()),
        ]),
    };
    let (input, receiver) = mpsc::channel(32);
    let (frames, mut output) = watch::channel(None);
    let (clipboard, _clipboard_output) = watch::channel(None);
    let cancel = CancellationToken::new();
    let driver = tokio::spawn({
        let cancel = cancel.clone();
        async move { driver::run(&connection, 640, 480, receiver, frames, clipboard, cancel).await }
    });
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            output.changed().await.unwrap();
            if output.borrow().as_ref().is_some_and(|frame| {
                frame
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[0] > 100 && p[1] > 100 && p[2] > 100)
            }) {
                break;
            }
        }
    })
    .await
    .expect("未收到終端機畫面");
    input
        .send(Input::Key {
            keysym: u32::from(b'a'),
            pressed: true,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), observed_rx)
        .await
        .expect("按鍵未送達")
        .unwrap();
    input
        .send(Input::Paste {
            text: zeroize::Zeroizing::new("alpha\x1b[201~HACK\n".into()),
            permit: std::sync::Arc::new(tokio::sync::Semaphore::new(1))
                .try_acquire_owned()
                .unwrap(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), paste_rx)
        .await
        .expect("安全貼上未送達")
        .unwrap();
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(8), driver)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(8), fixture)
        .await
        .expect("遠端連線未關閉")
        .unwrap();
}

#[tokio::test]
async fn oversized_paste_never_sends_a_prefix() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fixture = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.write_all(b"ready\r\n").await.unwrap();
        let mut data = Vec::new();
        socket.read_to_end(&mut data).await.unwrap();
        data
    });
    let connection = Connection {
        id: "test".into(),
        name: "test".into(),
        protocol: "telnet".into(),
        params: BTreeMap::from([
            ("hostname".into(), "127.0.0.1".into()),
            ("port".into(), port.to_string()),
        ]),
    };
    let (input, receiver) = mpsc::channel(4);
    let (frames, mut output) = watch::channel(None);
    let (clipboard, _clipboard_output) = watch::channel(None);
    let driver = tokio::spawn(async move {
        driver::run(
            &connection,
            640,
            480,
            receiver,
            frames,
            clipboard,
            CancellationToken::new(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(10), output.changed())
        .await
        .unwrap()
        .unwrap();
    input
        .send(Input::Paste {
            text: zeroize::Zeroizing::new("Q".repeat(256 * 1024 + 1)),
            permit: std::sync::Arc::new(tokio::sync::Semaphore::new(1))
                .try_acquire_owned()
                .unwrap(),
        })
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(8), driver)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    let bytes = tokio::time::timeout(Duration::from_secs(8), fixture)
        .await
        .unwrap()
        .unwrap();
    assert!(!bytes.contains(&b'Q'));
}
