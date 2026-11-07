use guac_session::*;
use guac_testkit::SimClient;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::duplex;

const ARGS: &[&str] = &["hostname", "port"];

/// Plugin whose behaviour is chosen by a closure-free enum, to keep tests readable.
#[derive(Clone, Copy)]
enum Mode {
    /// Echo mouse events as `nop` instructions with x,y; end on Key 0xFFFF.
    Echo,
    /// Send `count` large instructions as fast as allowed, then finish.
    Flood(usize),
    /// Return an error immediately.
    Fail,
    /// Panic immediately.
    Panic,
    /// Ignore cancellation forever.
    Stubborn,
    /// Send a sync and wait for client ack.
    SyncWait,
    /// Report received params and clipboard as `log` instructions.
    Report,
}

struct TestPlugin {
    mode: Mode,
    started: Arc<AtomicUsize>,
    finished: Arc<AtomicUsize>,
}

impl ProtocolPlugin for TestPlugin {
    fn name(&self) -> &'static str {
        "test"
    }
    fn arg_names(&self) -> &'static [&'static str] {
        ARGS
    }
    fn run(&self, mut ctx: SessionCtx, params: Params) -> BoxFuture<Result<(), SessionError>> {
        let mode = self.mode;
        let started = self.started.clone();
        let finished = self.finished.clone();
        Box::pin(async move {
            started.fetch_add(1, Ordering::SeqCst);
            let r = match mode {
                Mode::Echo => {
                    while let Some(ev) = ctx.events.recv().await {
                        match ev {
                            InputEvent::Mouse { x, y, .. } => {
                                ctx.tx
                                    .send(Instruction::new("nop", [x.to_string(), y.to_string()]))
                                    .await?
                            }
                            InputEvent::Key { keysym: 0xFFFF, .. } => break,
                            InputEvent::Resize { width, height } => {
                                ctx.tx
                                    .send(Instruction::new(
                                        "size",
                                        ["0".to_string(), width.to_string(), height.to_string()],
                                    ))
                                    .await?
                            }
                            _ => {}
                        }
                    }
                    Ok(())
                }
                Mode::Flood(n) => {
                    let blob = "A".repeat(4000);
                    for _ in 0..n {
                        ctx.tx.send(Instruction::new("blob", ["1", &blob])).await?;
                    }
                    Ok(())
                }
                Mode::Fail => Err(SessionError::plugin(
                    Status::UpstreamUnavailable,
                    "backend down",
                )),
                Mode::Panic => panic!("boom"),
                Mode::Stubborn => {
                    std::future::pending::<()>().await;
                    Ok(())
                }
                Mode::SyncWait => {
                    ctx.tx.sync().await?;
                    ctx.tx.wait_lag(0).await?;
                    ctx.tx.send(Instruction::new("nop", ["acked"])).await?;
                    ctx.tx.cancelled().await;
                    Ok(())
                }
                Mode::Report => {
                    ctx.tx
                        .send(Instruction::new(
                            "log",
                            [format!(
                                "host={} port={} empty={:?} w={}",
                                params.get("hostname").unwrap_or("-"),
                                params.get("port").unwrap_or("-"),
                                params.get("nothing"),
                                ctx.tx.caps().optimal_width
                            )],
                        ))
                        .await?;
                    while let Some(ev) = ctx.events.recv().await {
                        if let InputEvent::Clipboard(s) = ev {
                            ctx.tx
                                .send(Instruction::new("log", [format!("clip={s}")]))
                                .await?;
                        }
                    }
                    Ok(())
                }
            };
            finished.fetch_add(1, Ordering::SeqCst);
            r
        })
    }
}

struct Harness {
    client: SimClient,
    started: Arc<AtomicUsize>,
    finished: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<Result<(), SessionError>>,
}

fn quick_cfg() -> SessionConfig {
    SessionConfig {
        shutdown_grace: Duration::from_millis(200),
        ..Default::default()
    }
}

/// Server side performs: read `select`, handshake, run_session.
fn start(mode: Mode, buf: usize, cfg: SessionConfig) -> Harness {
    let (c, s) = duplex(buf);
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let plugin: Arc<dyn ProtocolPlugin> = Arc::new(TestPlugin {
        mode,
        started: started.clone(),
        finished: finished.clone(),
    });
    let server = tokio::spawn(async move {
        let (r, mut w) = tokio::io::split(s);
        let mut reader = InstructionReader::new(r);
        let sel = reader.read().await?.ok_or(SessionError::Closed)?;
        assert_eq!(sel.opcode, "select");
        let hs = handshake(
            &mut reader,
            &mut w,
            plugin.arg_names(),
            "$test-id",
            HANDSHAKE_TIMEOUT,
        )
        .await?;
        run_session(reader, w, hs, plugin, cfg).await
    });
    Harness {
        client: SimClient::new(c),
        started,
        finished,
        server,
    }
}

// ---- HS: handshake ----

#[tokio::test]
async fn hs_01_normal_handshake() {
    let mut h = start(Mode::Echo, 65536, quick_cfg());
    let out = h
        .client
        .handshake("test", &[("hostname", "h")], 800, 600)
        .await;
    assert_eq!(out.args, vec!["VERSION_1_5_0", "hostname", "port"]);
    assert_eq!(out.connection_id, "$test-id");
}

#[tokio::test]
async fn hs_03_optional_instructions_omitted() {
    let mut h = start(Mode::Report, 65536, quick_cfg());
    h.client
        .send(Instruction::new("select", ["test"]))
        .await
        .unwrap();
    let args = h.client.expect("args").await.args;
    assert_eq!(args.len(), 3);
    // straight to connect with no size/audio/...
    h.client
        .send(Instruction::new(
            "connect",
            ["VERSION_1_5_0", "host1", "22"],
        ))
        .await
        .unwrap();
    h.client.expect("ready").await;
    let log = h.client.expect("log").await;
    // defaults: 1024x768
    assert_eq!(log.args[0], "host=host1 port=22 empty=None w=1024");
}

#[tokio::test]
async fn hs_04_empty_value_is_unset() {
    let mut h = start(Mode::Report, 65536, quick_cfg());
    h.client
        .handshake("test", &[("hostname", "h"), ("port", "")], 640, 480)
        .await;
    let log = h.client.expect("log").await;
    assert_eq!(log.args[0], "host=h port=- empty=None w=640");
}

#[tokio::test]
async fn hs_05_wrong_connect_value_count_closes() {
    let mut h = start(Mode::Echo, 65536, quick_cfg());
    h.client
        .send(Instruction::new("select", ["test"]))
        .await
        .unwrap();
    h.client.expect("args").await;
    h.client
        .send(Instruction::new("connect", ["VERSION_1_5_0", "only-one"]))
        .await
        .unwrap();
    let r = h.server.await.unwrap();
    assert!(matches!(r, Err(SessionError::Handshake(_))));
    assert_eq!(h.started.load(Ordering::SeqCst), 0, "plugin must not start");
}

#[tokio::test(start_paused = true)]
async fn hs_08_handshake_timeout() {
    let mut h = start(Mode::Echo, 65536, quick_cfg());
    h.client
        .send(Instruction::new("select", ["test"]))
        .await
        .unwrap();
    h.client.expect("args").await;
    // never send connect; paused time auto-advances past 15 s
    let r = h.server.await.unwrap();
    assert!(matches!(r, Err(SessionError::Timeout)));
}

#[tokio::test]
async fn hs_09_garbage_during_handshake() {
    let mut h = start(Mode::Echo, 65536, quick_cfg());
    h.client
        .send(Instruction::new("select", ["test"]))
        .await
        .unwrap();
    h.client.expect("args").await;
    h.client.send_raw(b"x.garbage").await.unwrap();
    let r = h.server.await.unwrap();
    assert!(matches!(r, Err(SessionError::Protocol(_))));
}

#[tokio::test]
async fn hs_07_disconnect_after_select() {
    let mut h = start(Mode::Echo, 65536, quick_cfg());
    h.client
        .send(Instruction::new("select", ["test"]))
        .await
        .unwrap();
    h.client.expect("args").await;
    drop(h.client);
    let r = h.server.await.unwrap();
    assert!(matches!(r, Err(SessionError::Closed)));
}

#[tokio::test]
async fn hs_11_version_negotiated_to_min() {
    let (c, s) = duplex(4096);
    let server = tokio::spawn(async move {
        let (r, mut w) = tokio::io::split(s);
        let mut reader = InstructionReader::new(r);
        reader.read().await.unwrap();
        handshake(&mut reader, &mut w, ARGS, "$x", HANDSHAKE_TIMEOUT).await
    });
    let mut cl = SimClient::new(c);
    cl.send(Instruction::new("select", ["test"])).await.unwrap();
    cl.expect("args").await;
    cl.send(Instruction::new("connect", ["VERSION_1_1_0", "a", "b"]))
        .await
        .unwrap();
    cl.expect("ready").await;
    let hs = server.await.unwrap().unwrap();
    assert_eq!(hs.version, guac_protocol::ProtocolVersion::V1_1_0);
}

// ---- IN: input ----

#[tokio::test]
async fn in_mouse_key_resize_dispatch() {
    let mut h = start(Mode::Echo, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    h.client
        .send(Instruction::new("mouse", ["10", "20", "1"]))
        .await
        .unwrap();
    assert_eq!(h.client.expect("nop").await.args, vec!["10", "20"]);
    h.client
        .send(Instruction::new("size", ["1280", "720"]))
        .await
        .unwrap();
    assert_eq!(h.client.expect("size").await.args, vec!["0", "1280", "720"]);
}

#[tokio::test]
async fn in_malformed_input_ignored_not_fatal() {
    let mut h = start(Mode::Echo, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    h.client
        .send(Instruction::new("mouse", ["only-one"]))
        .await
        .unwrap();
    h.client
        .send(Instruction::new("mouse", ["abc", "def", "ghi"]))
        .await
        .unwrap();
    h.client
        .send(Instruction::new("totally-unknown", ["x"]))
        .await
        .unwrap();
    h.client
        .send(Instruction::new("mouse", ["1", "2", "0"]))
        .await
        .unwrap();
    assert_eq!(h.client.expect("nop").await.args, vec!["1", "2"]);
}

#[tokio::test]
async fn in_clipboard_stream_assembled_and_acked() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut h = start(Mode::Report, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    h.client.expect("log").await;
    h.client
        .send(Instruction::new("clipboard", ["3", "text/plain"]))
        .await
        .unwrap();
    h.client
        .send(Instruction::new(
            "blob",
            ["3".to_string(), b64.encode("hello ")],
        ))
        .await
        .unwrap();
    h.client
        .send(Instruction::new(
            "blob",
            ["3".to_string(), b64.encode("世界")],
        ))
        .await
        .unwrap();
    h.client.send(Instruction::new("end", ["3"])).await.unwrap();
    assert_eq!(h.client.expect("log").await.args[0], "clip=hello 世界");
    let acks: Vec<_> = h.client.log.iter().filter(|i| i.opcode == "ack").collect();
    assert!(acks.len() >= 3);
    assert_eq!(acks[0].args, vec!["3", "OK", "0"]);
}

#[tokio::test]
async fn in_clipboard_over_limit_is_rejected_without_pasting_a_prefix() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let cfg = SessionConfig {
        max_clipboard: 8,
        ..quick_cfg()
    };
    let mut h = start(Mode::Report, 65536, cfg);
    h.client.handshake("test", &[], 800, 600).await;
    h.client.expect("log").await;
    h.client
        .send(Instruction::new("clipboard", ["1", "text/plain"]))
        .await
        .unwrap();
    h.client
        .send(Instruction::new(
            "blob",
            ["1".to_string(), b64.encode("12345")],
        ))
        .await
        .unwrap();
    h.client
        .send(Instruction::new(
            "blob",
            ["1".to_string(), b64.encode("67890")],
        ))
        .await
        .unwrap();
    h.client.send(Instruction::new("end", ["1"])).await.unwrap();
    assert_eq!(
        h.client.expect("error").await.args[1],
        Status::ClientOverrun.code().to_string()
    );
    h.client.expect("disconnect").await;
    assert!(!h
        .client
        .log
        .iter()
        .any(|i| i.opcode == "log" && i.args[0].starts_with("clip=")));
}

#[tokio::test]
async fn in_clipboard_aggregate_and_stream_count_limits_are_enforced() {
    use base64::Engine;
    for aggregate in [true, false] {
        let mut h = start(
            Mode::Report,
            65536,
            SessionConfig {
                max_clipboard: 8,
                ..quick_cfg()
            },
        );
        h.client.handshake("test", &[], 800, 600).await;
        h.client.expect("log").await;
        for n in 0..if aggregate { 2 } else { 5 } {
            let id = n.to_string();
            h.client
                .send(Instruction::new("clipboard", [&id, "text/plain"]))
                .await
                .unwrap();
            if aggregate {
                h.client
                    .send(Instruction::new(
                        "blob",
                        [
                            id,
                            base64::engine::general_purpose::STANDARD.encode("12345"),
                        ],
                    ))
                    .await
                    .unwrap();
            }
        }
        assert_eq!(
            h.client.expect("error").await.args[1],
            Status::ClientOverrun.code().to_string()
        );
        h.client.expect("disconnect").await;
    }
}

#[tokio::test(start_paused = true)]
async fn in_clipboard_idle_stream_expires_without_more_input() {
    let mut h = start(Mode::Report, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    h.client.expect("log").await;
    h.client
        .send(Instruction::new("clipboard", ["1", "text/plain"]))
        .await
        .unwrap();
    h.client.expect("ack").await;
    tokio::time::advance(Duration::from_secs(16)).await;
    assert_eq!(
        h.client.expect("error").await.args[1],
        Status::ClientTimeout.code().to_string()
    );
    h.client.expect("disconnect").await;
}

// ---- end-of-session semantics ----

#[tokio::test]
async fn disconnect_from_client_ends_cleanly_and_cancels_plugin() {
    let mut h = start(Mode::SyncWait, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    h.client
        .send(Instruction::new("disconnect", Vec::<String>::new()))
        .await
        .unwrap();
    let r = h.server.await.unwrap();
    assert!(r.is_ok());
    assert_eq!(
        h.finished.load(Ordering::SeqCst),
        0,
        "SyncWait never finishes normally; it was cancelled"
    );
}

#[tokio::test]
async fn plugin_clean_exit_sends_disconnect() {
    let mut h = start(Mode::Flood(0), 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    let d = h.client.expect("disconnect").await;
    assert!(d.args.is_empty());
    assert!(h.server.await.unwrap().is_ok());
}

// ---- FT: fault injection ----

#[tokio::test]
async fn ft_03_plugin_error_reaches_client_with_status() {
    let mut h = start(Mode::Fail, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    let e = h.client.expect("error").await;
    assert_eq!(e.args, vec!["Aborted. See logs.", "520"]); // 0x0208; details only in the server log
    h.client.expect("disconnect").await;
    assert!(matches!(
        h.server.await.unwrap(),
        Err(SessionError::Plugin { .. })
    ));
}

#[tokio::test]
async fn ft_05_plugin_panic_is_contained() {
    let mut h = start(Mode::Panic, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    let e = h.client.expect("error").await;
    assert_eq!(e.args[1], "512"); // SERVER_ERROR 0x0200
    assert!(
        !e.args[0].contains("boom"),
        "internal panic text must not leak to client"
    );

    // A second, independent session still works.
    let mut h2 = start(Mode::Echo, 65536, quick_cfg());
    h2.client.handshake("test", &[], 800, 600).await;
    h2.client
        .send(Instruction::new("mouse", ["5", "6", "0"]))
        .await
        .unwrap();
    assert_eq!(h2.client.expect("nop").await.args, vec!["5", "6"]);
}

#[tokio::test]
async fn ft_07_stubborn_plugin_is_aborted_after_grace() {
    let mut h = start(Mode::Stubborn, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    drop(h.client); // client vanishes
    let r = tokio::time::timeout(Duration::from_secs(3), h.server).await;
    assert!(
        r.is_ok(),
        "session must end despite a plugin ignoring cancellation"
    );
}

#[tokio::test]
async fn ft_08_client_disconnect_does_not_retry() {
    let mut h = start(Mode::Echo, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    h.client
        .send(Instruction::new("disconnect", Vec::<String>::new()))
        .await
        .unwrap();
    h.server.await.unwrap().unwrap();
    assert_eq!(
        h.started.load(Ordering::SeqCst),
        1,
        "plugin started exactly once"
    );
}

#[tokio::test]
async fn ft_protocol_violation_after_ready_reports_error() {
    let mut h = start(Mode::Echo, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    h.client.send_raw(b"zzz").await.unwrap();
    let e = h.client.expect("error").await;
    assert_eq!(e.args[1], "768"); // CLIENT_BAD_REQUEST 0x0300
    assert!(matches!(
        h.server.await.unwrap(),
        Err(SessionError::Protocol(_))
    ));
}

#[tokio::test]
async fn ft_09_many_concurrent_sessions_leave_no_tasks() {
    let mut hs = vec![];
    for _ in 0..50 {
        let mut h = start(Mode::Echo, 65536, quick_cfg());
        h.client.handshake("test", &[], 800, 600).await;
        hs.push(h);
    }
    for h in &mut hs {
        h.client
            .send(Instruction::new("mouse", ["1", "1", "0"]))
            .await
            .unwrap();
        h.client.expect("nop").await;
        h.client
            .send(Instruction::new("disconnect", Vec::<String>::new()))
            .await
            .unwrap();
    }
    for h in hs {
        assert!(h.server.await.unwrap().is_ok());
    }
}

// ---- BP: backpressure ----

#[tokio::test]
async fn bp_01_slow_client_bounds_server_queue() {
    // Tiny pipe + client that never reads: the plugin must block, not buffer unboundedly.
    let cfg = SessionConfig {
        out_queue: 8,
        ..quick_cfg()
    };
    let mut h = start(Mode::Flood(1_000_000), 1024, cfg);
    h.client.handshake("test", &[], 800, 600).await;
    // do not read; give the server time to run into the bounds
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        h.finished.load(Ordering::SeqCst),
        0,
        "plugin must still be blocked by backpressure"
    );
    // client disconnects: everything unwinds
    h.client
        .send(Instruction::new("disconnect", Vec::<String>::new()))
        .await
        .ok();
    drop(h.client);
    let r = tokio::time::timeout(Duration::from_secs(3), h.server).await;
    assert!(
        r.is_ok(),
        "blocked session must unwind on client disconnect"
    );
}

#[tokio::test]
async fn bp_02_client_resumes() {
    let cfg = SessionConfig {
        out_queue: 8,
        ..quick_cfg()
    };
    let mut h = start(Mode::Flood(200), 1024, cfg);
    h.client.handshake("test", &[], 800, 600).await;
    let mut n = 0;
    while let Some(i) = h.client.recv().await {
        if i.opcode == "blob" {
            n += 1;
        }
        if i.opcode == "disconnect" {
            break;
        }
    }
    assert_eq!(n, 200);
}

#[tokio::test]
async fn bp_04_sync_lag_waits_for_client_reply() {
    let mut h = start(Mode::SyncWait, 65536, quick_cfg());
    h.client.handshake("test", &[], 800, 600).await;
    let sync = h.client.expect("sync").await;
    // Not acknowledged yet: plugin must not have proceeded.
    assert!(h
        .client
        .recv_within(Duration::from_millis(200))
        .await
        .is_none());
    h.client
        .send(Instruction::new("sync", [sync.args[0].clone()]))
        .await
        .unwrap();
    assert_eq!(h.client.expect("nop").await.args, vec!["acked"]);
}
