#![cfg(feature = "telnet")]

use guac_protocol::Instruction;
use guac_session::*;
use guac_testkit::{DisplayReplay, SimClient};
use guacd::{serve, Registry, ServerConfig};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

const IAC: u8 = 255;
const DO: u8 = 253;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

#[tokio::test]
async fn tel_legacy_encoding_covers_split_prompts_credentials_and_input() {
    let (gaddr, shutdown) = start_guacd().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let wire = vec![0xa4, 0xa4, 0xa4, 0xe5]; // Big5 中文
    let expected = wire.clone();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(&[IAC, WILL, 1]).await.unwrap();
        let mut acc = vec![];
        read_until(&mut stream, &mut acc, &[IAC, DO, 1]).await;
        for byte in &expected {
            stream.write_all(&[*byte]).await.unwrap();
        }
        stream.write_all(b":").await.unwrap();
        acc.clear();
        let credential = [expected.clone(), b"\r".to_vec()].concat();
        read_until(&mut stream, &mut acc, &credential).await;
        acc.clear();
        stream.write_all(b"Password:").await.unwrap();
        read_until(&mut stream, &mut acc, &credential).await;
        acc.clear();
        stream.write_all(b"OK\r\n").await.unwrap();
        read_until(&mut stream, &mut acc, &expected).await;
    });
    let mut client = SimClient::new(TcpStream::connect(gaddr).await.unwrap());
    client
        .handshake(
            "telnet",
            &[
                ("hostname", "127.0.0.1"),
                ("port", &port),
                ("username", "中文"),
                ("password", "中文"),
                ("username-regex", "中文:"),
                ("login-success-regex", "OK"),
                ("login-failure-regex", "incorrect"),
                ("character-encoding", "Big5"),
            ],
            800,
            480,
        )
        .await;
    let mut replay = DisplayReplay::new();
    drain_frames(&mut client, &mut replay, 200).await;
    use base64::Engine;
    client
        .send(Instruction::new("clipboard", ["1", "text/plain"]))
        .await
        .unwrap();
    client
        .send(Instruction::new(
            "blob",
            [
                "1".to_string(),
                base64::engine::general_purpose::STANDARD.encode("中文"),
            ],
        ))
        .await
        .unwrap();
    client.send(Instruction::new("end", ["1"])).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), peer)
        .await
        .unwrap()
        .unwrap();
    shutdown.cancel();
}

async fn start_guacd() -> (SocketAddr, CancellationToken) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let cfg = ServerConfig {
        session: SessionConfig {
            shutdown_grace: Duration::from_millis(300),
            ..Default::default()
        },
        ..Default::default()
    };
    tokio::spawn(serve(
        listener,
        std::sync::Arc::new(Registry::builtin()),
        cfg,
        shutdown.clone(),
    ));
    (addr, shutdown)
}

/// Read from the socket until `needle` has been seen in the accumulated bytes.
async fn read_until(s: &mut TcpStream, acc: &mut Vec<u8>, needle: &[u8]) {
    let mut buf = [0u8; 1024];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if acc.windows(needle.len()).any(|w| w == needle) {
            return;
        }
        let n = tokio::time::timeout_at(deadline, s.read(&mut buf))
            .await
            .unwrap_or_else(|_| panic!("timeout waiting for {needle:?}; got {acc:?}"))
            .unwrap();
        assert!(n > 0, "eof while waiting for {needle:?}; got {acc:?}");
        acc.extend_from_slice(&buf[..n]);
    }
}

fn has_ink(r: &DisplayReplay, rows: usize) -> bool {
    (0..rows).any(|y| {
        (0..300).any(|x| {
            r.pixel(0, x, y)
                .is_some_and(|p| p != [0, 0, 0, 255] && p != [0, 0, 0, 0])
        })
    })
}

async fn drain_frames(c: &mut SimClient, r: &mut DisplayReplay, ms: u64) {
    while let Some(i) = c.recv_within(Duration::from_millis(ms)).await {
        if i.opcode == "sync" {
            c.send(Instruction::new("sync", [i.args[0].clone()]))
                .await
                .unwrap();
        }
        r.apply(&i);
    }
}

#[tokio::test]
async fn tel_s_01_full_login_session_over_real_sockets() {
    let (gaddr, shutdown) = start_guacd().await;
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = server.local_addr().unwrap().port().to_string();

    // ---- fake telnet server ----
    let srv = tokio::spawn(async move {
        let (mut s, _) = server.accept().await.unwrap();
        let mut acc = vec![];
        // offer: DO TTYPE, DO NAWS, WILL ECHO
        s.write_all(&[IAC, DO, 24, IAC, DO, 31, IAC, WILL, 1])
            .await
            .unwrap();
        read_until(&mut s, &mut acc, &[IAC, WILL, 24]).await;
        read_until(&mut s, &mut acc, &[IAC, WILL, 31]).await;
        read_until(&mut s, &mut acc, &[IAC, DO, 1]).await;
        // NAWS payload follows: IAC SB 31 w w h h IAC SE (width cols is whatever the client computed)
        read_until(&mut s, &mut acc, &[IAC, SB, 31]).await;
        // ask for terminal type
        s.write_all(&[IAC, SB, 24, 1, IAC, SE]).await.unwrap();
        let mut is_linux = vec![IAC, SB, 24, 0];
        is_linux.extend(b"linux");
        read_until(&mut s, &mut acc, &is_linux).await;
        // login dialogue
        acc.clear();
        s.write_all(b"Welcome to fake\r\nhost login: ")
            .await
            .unwrap();
        read_until(&mut s, &mut acc, b"alice\r").await;
        acc.clear();
        s.write_all(b"\r\nPassword: ").await.unwrap();
        read_until(&mut s, &mut acc, b"s3cret\r").await;
        acc.clear();
        s.write_all(b"\r\nLast login: today\r\n$ ").await.unwrap();
        // user types "ls\r"; echo it and answer
        read_until(&mut s, &mut acc, b"ls\r").await;
        s.write_all(b"ls\r\nreadme.txt\r\n$ ").await.unwrap();
        // user pastes
        acc.clear();
        read_until(&mut s, &mut acc, b"pasted").await;
        // then the server hangs up
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(s);
    });

    let mut c = SimClient::new(TcpStream::connect(gaddr).await.unwrap());
    c.handshake(
        "telnet",
        &[
            ("hostname", "127.0.0.1"),
            ("port", &port),
            ("username", "alice"),
            ("password", "s3cret"),
            ("login-success-regex", "Last login"),
            ("login-failure-regex", "incorrect"),
        ],
        800,
        480,
    )
    .await;

    let mut replay = DisplayReplay::new();
    // While the login prompt is shown, the terminal stays hidden (no ink).
    tokio::time::sleep(Duration::from_millis(300)).await;
    drain_frames(&mut c, &mut replay, 200).await;
    // After success text appears.
    let mut seen = false;
    for _ in 0..20 {
        drain_frames(&mut c, &mut replay, 200).await;
        if has_ink(&replay, 60) {
            seen = true;
            break;
        }
    }
    assert!(seen, "terminal content must appear after login success");

    // type "ls" + Enter (keysyms 'l', 's', Return)
    for k in ['l' as u32, 's' as u32, 0xFF0D] {
        for p in ["1", "0"] {
            c.send(Instruction::new("key", [k.to_string(), p.to_string()]))
                .await
                .unwrap();
        }
    }
    drain_frames(&mut c, &mut replay, 400).await;

    // paste via clipboard stream
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode("pasted");
    c.send(Instruction::new("clipboard", ["5", "text/plain"]))
        .await
        .unwrap();
    c.send(Instruction::new("blob", ["5".to_string(), b64]))
        .await
        .unwrap();
    c.send(Instruction::new("end", ["5"])).await.unwrap();

    // server hangs up -> client is told to disconnect
    loop {
        let i = c.recv().await.expect("expected disconnect before EOF");
        if i.opcode == "sync" {
            c.send(Instruction::new("sync", [i.args[0].clone()]))
                .await
                .unwrap();
        }
        if i.opcode == "disconnect" {
            break;
        }
    }
    srv.await.unwrap();
    shutdown.cancel();
}

#[tokio::test]
async fn tel_s_02_failed_login_aborts_with_unauthorized() {
    let (gaddr, _sd) = start_guacd().await;
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = server.local_addr().unwrap().port().to_string();
    tokio::spawn(async move {
        let (mut s, _) = server.accept().await.unwrap();
        s.write_all(b"login: ").await.unwrap();
        let mut b = [0u8; 64];
        let _ = s.read(&mut b).await;
        s.write_all(b"\r\nPassword: ").await.unwrap();
        let _ = s.read(&mut b).await;
        s.write_all(b"\r\nLogin incorrect\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let mut c = SimClient::new(TcpStream::connect(gaddr).await.unwrap());
    c.handshake(
        "telnet",
        &[
            ("hostname", "127.0.0.1"),
            ("port", &port),
            ("username", "u"),
            ("password", "p"),
            ("login-success-regex", "Last login"),
            ("login-failure-regex", "incorrect"),
        ],
        640,
        384,
    )
    .await;
    let e = c.expect("error").await;
    assert_eq!(e.args[1], "769"); // CLIENT_UNAUTHORIZED
}

#[tokio::test]
async fn tel_s_03_connection_refused_maps_to_upstream_unavailable() {
    let (gaddr, _sd) = start_guacd().await;
    // reserve a port, then close it so nothing listens
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port().to_string();
    drop(l);
    let mut c = SimClient::new(TcpStream::connect(gaddr).await.unwrap());
    c.handshake(
        "telnet",
        &[
            ("hostname", "127.0.0.1"),
            ("port", &port),
            ("timeout", "10"),
        ],
        640,
        384,
    )
    .await;
    let e = c.expect("error").await;
    assert_eq!(e.args, vec!["Aborted. See logs.", "520"]); // UPSTREAM_UNAVAILABLE 0x0208
}

#[tokio::test]
async fn tel_s_04_missing_hostname_is_a_client_error() {
    let (gaddr, _sd) = start_guacd().await;
    let mut c = SimClient::new(TcpStream::connect(gaddr).await.unwrap());
    c.handshake("telnet", &[], 640, 384).await;
    let e = c.expect("error").await;
    assert_eq!(e.args[1], "768"); // CLIENT_BAD_REQUEST
}

#[tokio::test]
async fn tel_s_05_binary_safe_and_iac_escaping_both_ways() {
    let (gaddr, _sd) = start_guacd().await;
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = server.local_addr().unwrap().port().to_string();
    let srv = tokio::spawn(async move {
        let (mut s, _) = server.accept().await.unwrap();
        // server sends a literal 0xFF (escaped) followed by text
        s.write_all(&[IAC, IAC, b'o', b'k', b'\r', b'\n'])
            .await
            .unwrap();
        // the client types keysym 0xFF-range char (U+00FF 'ÿ' -> UTF-8 C3 BF, no raw 0xFF)
        let mut acc = vec![];
        read_until(&mut s, &mut acc, "ÿ".as_bytes()).await;
        assert!(!acc.contains(&IAC), "no stray IAC on the wire: {acc:?}");
    });
    let mut c = SimClient::new(TcpStream::connect(gaddr).await.unwrap());
    c.handshake(
        "telnet",
        &[("hostname", "127.0.0.1"), ("port", &port)],
        640,
        384,
    )
    .await;
    for p in ["1", "0"] {
        c.send(Instruction::new("key", ["255".to_string(), p.to_string()]))
            .await
            .unwrap();
    }
    let mut rp = DisplayReplay::new();
    drain_frames(&mut c, &mut rp, 300).await;
    srv.await.unwrap();
}

#[tokio::test]
async fn tel_s_06_resize_is_sent_as_naws() {
    let (gaddr, _sd) = start_guacd().await;
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = server.local_addr().unwrap().port().to_string();
    let srv = tokio::spawn(async move {
        let (mut s, _) = server.accept().await.unwrap();
        s.write_all(&[IAC, DO, 31]).await.unwrap();
        let mut acc = vec![];
        read_until(&mut s, &mut acc, &[IAC, WILL, 31]).await;
        read_until(&mut s, &mut acc, &[IAC, SE]).await;
        acc.clear();
        // after the client resizes, a fresh NAWS arrives
        read_until(&mut s, &mut acc, &[IAC, SB, 31]).await;
        read_until(&mut s, &mut acc, &[IAC, SE]).await;
        acc
    });
    let mut c = SimClient::new(TcpStream::connect(gaddr).await.unwrap());
    c.handshake(
        "telnet",
        &[("hostname", "127.0.0.1"), ("port", &port)],
        640,
        384,
    )
    .await;
    let mut rp = DisplayReplay::new();
    drain_frames(&mut c, &mut rp, 300).await;
    c.send(Instruction::new("size", ["1000", "700"]))
        .await
        .unwrap();
    drain_frames(&mut c, &mut rp, 300).await;
    let naws = srv.await.unwrap();
    let i = naws.windows(3).position(|w| w == [IAC, SB, 31]).unwrap();
    let cols = u16::from_be_bytes([naws[i + 3], naws[i + 4]]);
    let rows = u16::from_be_bytes([naws[i + 5], naws[i + 6]]);
    assert!(
        cols > 80 && rows > 25,
        "new NAWS reflects the larger window: {cols}x{rows}"
    );
}
