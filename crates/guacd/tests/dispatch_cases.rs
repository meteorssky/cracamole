use guac_protocol::Instruction;
use guac_session::*;
use guac_testkit::SimClient;
use guacd::{serve, Registry, ServerConfig};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

/// Test protocol: echoes `mouse` as `nop`, optionally panics / errors on request via the hostname.
struct EchoPlugin {
    started: Arc<AtomicUsize>,
}

impl ProtocolPlugin for EchoPlugin {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn arg_names(&self) -> &'static [&'static str] {
        &["hostname", "port"]
    }
    fn run(&self, mut ctx: SessionCtx, params: Params) -> BoxFuture<Result<(), SessionError>> {
        self.started.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match params.get("hostname") {
                Some("panic") => panic!("kaboom"),
                Some("fail") => {
                    return Err(SessionError::plugin(Status::UpstreamError, "secret detail"))
                }
                _ => {}
            }
            loop {
                tokio::select! {
                    _ = ctx.tx.cancelled() => return Ok(()),
                    ev = ctx.events.recv() => match ev {
                        Some(InputEvent::Mouse { x, y, .. }) => {
                            ctx.tx.send(Instruction::new("nop", [x.to_string(), y.to_string()])).await?
                        }
                        Some(_) => {}
                        None => return Ok(()),
                    }
                }
            }
        })
    }
}

struct Srv {
    addr: SocketAddr,
    shutdown: CancellationToken,
    handle: tokio::task::JoinHandle<std::io::Result<()>>,
    started: Arc<AtomicUsize>,
}

async fn start_with(cfg: ServerConfig) -> Srv {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let started = Arc::new(AtomicUsize::new(0));
    let registry = Arc::new(Registry::builtin().with(EchoPlugin {
        started: started.clone(),
    }));
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn(serve(listener, registry, cfg, shutdown.clone()));
    Srv {
        addr,
        shutdown,
        handle,
        started,
    }
}

async fn start() -> Srv {
    start_with(ServerConfig {
        session: SessionConfig {
            shutdown_grace: Duration::from_millis(300),
            ..Default::default()
        },
        ..Default::default()
    })
    .await
}

async fn client(addr: SocketAddr) -> SimClient {
    SimClient::new(TcpStream::connect(addr).await.unwrap())
}

// ---------- GD-DSP: routing ----------

#[tokio::test]
async fn gd_dsp_01_handshake_through_a_real_socket() {
    let s = start().await;
    let mut c = client(s.addr).await;
    let out = c.handshake("echo", &[("hostname", "h")], 800, 600).await;
    assert_eq!(out.args, vec!["VERSION_1_5_0", "hostname", "port"]);
    assert!(out.connection_id.starts_with('$'));
    c.send(Instruction::new("mouse", ["3", "4", "0"]))
        .await
        .unwrap();
    assert_eq!(c.expect("nop").await.args, vec!["3", "4"]);
}

#[cfg(feature = "telnet")]
#[tokio::test]
async fn gd_dsp_02_telnet_args_are_exactly_the_c_list() {
    let s = start().await;
    let mut c = client(s.addr).await;
    c.send(Instruction::new("select", ["telnet"]))
        .await
        .unwrap();
    let args = c.expect("args").await.args;
    assert_eq!(args[0], "VERSION_1_5_0");
    assert_eq!(&args[1..], proto_telnet::ARGS);
    assert_eq!(args.len(), 39);
}

#[tokio::test]
async fn gd_dsp_03_unknown_protocol_is_refused_with_error() {
    let s = start().await;
    let mut c = client(s.addr).await;
    c.send(Instruction::new("select", ["nosuchproto"]))
        .await
        .unwrap();
    let e = c.expect("error").await;
    assert_eq!(e.args[1], "256"); // UNSUPPORTED 0x0100
    assert!(c.recv().await.is_none(), "connection closed");
}

#[tokio::test]
async fn gd_dsp_04_joining_unknown_connection_id() {
    let s = start().await;
    let mut c = client(s.addr).await;
    c.send(Instruction::new(
        "select",
        ["$00000000-0000-0000-0000-000000000000"],
    ))
    .await
    .unwrap();
    let e = c.expect("error").await;
    assert_eq!(e.args, vec!["No such connection.", "516"]); // RESOURCE_NOT_FOUND 0x0204
}

#[tokio::test]
async fn gd_dsp_05_select_must_come_first_and_have_one_arg() {
    let s = start().await;
    let mut c = client(s.addr).await;
    c.send(Instruction::new("size", ["1", "2"])).await.unwrap();
    assert!(c.recv().await.is_none());
    let mut c = client(s.addr).await;
    c.send(Instruction::new("select", ["echo", "extra"]))
        .await
        .unwrap();
    assert!(c.recv().await.is_none());
    let mut c = client(s.addr).await;
    c.send(Instruction::new("select", Vec::<String>::new()))
        .await
        .unwrap();
    assert!(c.recv().await.is_none());
    assert_eq!(s.started.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gd_dsp_06_garbage_before_select() {
    let s = start().await;
    let mut c = client(s.addr).await;
    c.send_raw(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    assert!(c.recv().await.is_none());
}

#[tokio::test]
async fn gd_dsp_07_concurrent_connections_are_independent() {
    let s = start().await;
    let mut cs = vec![];
    for i in 0..20 {
        let mut c = client(s.addr).await;
        c.handshake("echo", &[("hostname", "h")], 800, 600).await;
        c.send(Instruction::new(
            "mouse",
            [i.to_string(), "0".into(), "0".into()],
        ))
        .await
        .unwrap();
        cs.push((i, c));
    }
    for (i, c) in &mut cs {
        assert_eq!(c.expect("nop").await.args[0], i.to_string());
    }
}

#[tokio::test]
async fn gd_dsp_08_plugin_panic_does_not_affect_other_connections() {
    let s = start().await;
    let mut good = client(s.addr).await;
    good.handshake("echo", &[("hostname", "h")], 800, 600).await;

    let mut bad = client(s.addr).await;
    bad.handshake("echo", &[("hostname", "panic")], 800, 600)
        .await;
    let e = bad.expect("error").await;
    assert_eq!(e.args[1], "512");
    assert_eq!(e.args[0], "Aborted. See logs.");

    good.send(Instruction::new("mouse", ["9", "9", "0"]))
        .await
        .unwrap();
    assert_eq!(good.expect("nop").await.args, vec!["9", "9"]);
    // and new connections still work
    let mut again = client(s.addr).await;
    again
        .handshake("echo", &[("hostname", "h")], 800, 600)
        .await;
}

#[tokio::test]
async fn gd_dsp_09_error_detail_is_not_leaked_to_client() {
    let s = start().await;
    let mut c = client(s.addr).await;
    c.handshake("echo", &[("hostname", "fail")], 800, 600).await;
    let e = c.expect("error").await;
    assert_eq!(e.args, vec!["Aborted. See logs.", "515"]); // UPSTREAM_ERROR 0x0203
    assert!(!c
        .log
        .iter()
        .any(|i| i.args.iter().any(|a| a.contains("secret"))));
}

#[tokio::test]
async fn gd_dsp_10_connection_limit_rejects_extra_clients() {
    let s = start_with(ServerConfig {
        max_connections: 2,
        ..Default::default()
    })
    .await;
    let mut a = client(s.addr).await;
    a.handshake("echo", &[("hostname", "h")], 800, 600).await;
    let mut b = client(s.addr).await;
    b.handshake("echo", &[("hostname", "h")], 800, 600).await;
    // third is dropped without service
    let mut c = client(s.addr).await;
    c.send(Instruction::new("select", ["echo"])).await.ok();
    assert!(c.recv_within(Duration::from_millis(500)).await.is_none());
    // after one leaves, capacity is available again
    a.send(Instruction::new("disconnect", Vec::<String>::new()))
        .await
        .unwrap();
    drop(a);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut d = client(s.addr).await;
    d.handshake("echo", &[("hostname", "h")], 800, 600).await;
}

#[tokio::test(start_paused = true)]
async fn gd_dsp_11_silent_client_times_out() {
    let s = start().await;
    let mut c = client(s.addr).await;
    // never sends select; paused time jumps past the 15 s select timeout
    assert!(c.recv_within(Duration::from_secs(30)).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn gd_dsp_12_slowloris_handshake_is_bounded() {
    let s = start().await;
    let mut c = client(s.addr).await;
    c.send(Instruction::new("select", ["echo"])).await.unwrap();
    c.expect("args").await;
    // Trickle harmless instructions faster than the 15 s per-instruction timeout, for well over
    // the 60 s total handshake bound, without ever sending `connect`.
    for _ in 0..8 {
        let _ = c
            .send(Instruction::new("audio", Vec::<String>::new()))
            .await;
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    // 80 s of virtual time have passed. A server without a total bound would still be waiting for
    // `connect` and would answer a valid one with `ready`; a bounded server has dropped us.
    let _ = c
        .send(Instruction::new("connect", ["VERSION_1_5_0", "h", "22"]))
        .await;
    let mut got_ready = false;
    while let Some(i) = c.recv_within(Duration::from_secs(5)).await {
        if i.opcode == "ready" {
            got_ready = true;
        }
    }
    assert!(
        !got_ready,
        "handshake must be abandoned after the total timeout"
    );
    assert_eq!(s.started.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gd_dsp_13_shutdown_sends_disconnect_and_returns() {
    let s = start().await;
    let mut c = client(s.addr).await;
    c.handshake("echo", &[("hostname", "h")], 800, 600).await;
    s.shutdown.cancel();
    c.expect("disconnect").await;
    let r = tokio::time::timeout(Duration::from_secs(5), s.handle).await;
    assert!(r.expect("serve returns after shutdown").unwrap().is_ok());
}

#[tokio::test]
async fn gd_dsp_14_shutdown_stops_accepting() {
    let s = start().await;
    s.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), s.handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    // listener is gone: new connections fail or are closed immediately
    match TcpStream::connect(s.addr).await {
        Err(_) => {}
        Ok(st) => {
            let mut c = SimClient::new(st);
            c.send(Instruction::new("select", ["echo"])).await.ok();
            assert!(c.recv_within(Duration::from_millis(300)).await.is_none());
        }
    }
}

#[tokio::test]
async fn gd_dsp_15_registry_lists_builtins() {
    let r = Registry::builtin();
    #[cfg(feature = "telnet")]
    assert!(r.get("telnet").is_some());
    #[cfg(feature = "rdp")]
    assert!(r.get("rdp").is_some());
    #[cfg(feature = "vnc")]
    assert!(r.get("vnc").is_some());
    assert!(r.get("kubernetes").is_none());
    assert_eq!(Registry::new().names().len(), 0);
}

#[cfg(feature = "ssh")]
#[tokio::test]
async fn gd_dsp_16_ssh_args_are_exactly_the_c_list() {
    let s = start().await;
    let mut c = client(s.addr).await;
    c.send(Instruction::new("select", ["ssh"])).await.unwrap();
    let args = c.expect("args").await.args;
    assert_eq!(args[0], "VERSION_1_5_0");
    assert_eq!(&args[1..], proto_ssh::ARGS);
    assert_eq!(args.len(), 47);
}

#[cfg(feature = "ssh")]
#[tokio::test]
async fn gd_dsp_17_ssh_connect_failure_reaches_client_through_guacd() {
    let s = start().await;
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port().to_string();
    drop(l);
    let mut c = client(s.addr).await;
    c.handshake(
        "ssh",
        &[
            ("hostname", "127.0.0.1"),
            ("port", &port),
            ("username", "u"),
            ("password", "p"),
            ("timeout", "10"),
        ],
        640,
        384,
    )
    .await;
    let e = c.expect("error").await;
    assert_eq!(e.args, vec!["Aborted. See logs.", "520"]);
}

#[cfg(all(feature = "ssh", feature = "telnet", feature = "rdp", feature = "vnc"))]
#[tokio::test]
async fn gd_dsp_18_all_builtin_protocols_registered() {
    let r = Registry::builtin();
    assert_eq!(r.names(), vec!["rdp", "ssh", "telnet", "vnc"]);
}

#[cfg(all(feature = "ssh", feature = "telnet", feature = "rdp", feature = "vnc"))]
#[tokio::test]
async fn gd_dsp_19_protocols_publish_metadata_in_declared_order() {
    use guac_session::IdentityKind;
    let r = Registry::builtin();
    let rows: Vec<_> = r
        .plugins()
        .map(|p| {
            let i = p.info();
            (p.name(), i.label, i.default_port, i.identity)
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("ssh", "SSH", 22, IdentityKind::HostKey),
            ("telnet", "Telnet", 23, IdentityKind::None),
            ("rdp", "RDP", 3389, IdentityKind::Certificate),
            ("vnc", "VNC", 5900, IdentityKind::None),
        ]
    );
    // A plugin that publishes nothing still gets a usable label.
    let echo = Registry::new().with(EchoPlugin {
        started: Arc::new(AtomicUsize::new(0)),
    });
    let info = echo.get("echo").unwrap().info();
    assert_eq!((info.label, info.default_port), ("echo", 0));
    assert_eq!(info.identity, IdentityKind::None);
}
