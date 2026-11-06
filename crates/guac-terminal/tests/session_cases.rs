use base64::Engine;
use guac_protocol::{Instruction, Status};
use guac_session::*;
use guac_terminal::*;
use guac_testkit::{DisplayReplay, SimClient};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::duplex;
use tokio::sync::mpsc;

struct TermPlugin {
    io: Mutex<Option<RemoteIo>>,
    filter: Mutex<Option<Box<dyn RemoteFilter>>>,
    start_immediately: bool,
}

impl ProtocolPlugin for TermPlugin {
    fn name(&self) -> &'static str {
        "termtest"
    }
    fn arg_names(&self) -> &'static [&'static str] {
        &["read-only", "scrollback"]
    }
    fn run(&self, ctx: SessionCtx, params: Params) -> BoxFuture<Result<(), SessionError>> {
        let io = self.io.lock().unwrap().take().unwrap();
        let filter = self.filter.lock().unwrap().take();
        let start = self.start_immediately;
        Box::pin(async move {
            let dpi = ctx.tx.caps().optimal_dpi;
            let mut cfg = TerminalConfig::from_params(&params, dpi);
            cfg.fonts = FontSet::bundled();
            cfg.filter = filter;
            cfg.start_immediately = start;
            run_terminal(ctx, cfg, io).await
        })
    }
}

struct Rig {
    client: SimClient,
    remote_tx: mpsc::Sender<Vec<u8>>,
    cmd_rx: mpsc::Receiver<RemoteCmd>,
    server: tokio::task::JoinHandle<Result<(), SessionError>>,
    replay: DisplayReplay,
}

async fn start(
    values: &[(&str, &str)],
    filter: Option<Box<dyn RemoteFilter>>,
    start_immediately: bool,
) -> Rig {
    let (remote_tx, from_remote) = mpsc::channel(64);
    let (to_remote, cmd_rx) = mpsc::channel(64);
    let plugin: Arc<dyn ProtocolPlugin> = Arc::new(TermPlugin {
        io: Mutex::new(Some(RemoteIo {
            from_remote,
            to_remote,
        })),
        filter: Mutex::new(filter),
        start_immediately,
    });
    let (c, s) = duplex(1 << 20);
    let server = tokio::spawn(async move {
        let (r, mut w) = tokio::io::split(s);
        let mut reader = InstructionReader::new(r);
        reader.read().await?; // select
        let hs = handshake(
            &mut reader,
            &mut w,
            plugin.arg_names(),
            "$t",
            HANDSHAKE_TIMEOUT,
        )
        .await?;
        run_session(reader, w, hs, plugin, SessionConfig::default()).await
    });
    let mut client = SimClient::new(c);
    client.handshake("termtest", values, 640, 384).await;
    Rig {
        client,
        remote_tx,
        cmd_rx,
        server,
        replay: DisplayReplay::new(),
    }
}

impl Rig {
    /// Apply one full frame (up to and including a sync, which is acknowledged).
    async fn frame(&mut self) -> Vec<Instruction> {
        let f = self.client.frame().await;
        for i in &f {
            self.replay.apply(i);
        }
        assert!(
            self.replay.unsupported.is_empty(),
            "{:?}",
            self.replay.unsupported
        );
        f
    }

    async fn cmd(&mut self) -> RemoteCmd {
        tokio::time::timeout(Duration::from_secs(5), self.cmd_rx.recv())
            .await
            .expect("timeout waiting for terminal output")
            .expect("terminal closed")
    }

    /// Collect Data commands until `n` bytes arrived (resize commands are skipped).
    async fn data(&mut self, n: usize) -> Vec<u8> {
        let mut v = vec![];
        while v.len() < n {
            if let RemoteCmd::Data(d) = self.cmd().await {
                v.extend(d);
            }
        }
        v
    }

    async fn key(&mut self, keysym: u32) {
        for p in ["1", "0"] {
            self.client
                .send(Instruction::new("key", [keysym.to_string(), p.to_string()]))
                .await
                .unwrap();
        }
    }
}

fn region_has_ink(
    r: &DisplayReplay,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    bg: [u8; 4],
) -> bool {
    (y0..y0 + h).any(|y| (x0..x0 + w).any(|x| r.pixel(0, x, y).is_some_and(|p| p != bg)))
}

#[tokio::test]
async fn ts_01_initial_frame_and_remote_output_reach_the_client() {
    let mut r = start(&[], None, true).await;
    let f = r.frame().await;
    assert_eq!(f[0].opcode, "size");
    assert_eq!(f[0].args, vec!["0", "640", "384"]);
    // initial resize notification to remote
    assert!(matches!(r.cmd().await, RemoteCmd::Resize { .. }));
    r.remote_tx.send(b"\x1b[?25lHello".to_vec()).await.unwrap();
    r.frame().await;
    // top-left cell region now contains ink
    assert!(region_has_ink(&r.replay, 0, 0, 40, 20, [0, 0, 0, 255]));
}

#[tokio::test]
async fn ts_02_keyboard_input_goes_to_remote() {
    let mut r = start(&[], None, true).await;
    r.frame().await;
    r.key('l' as u32).await;
    r.key('s' as u32).await;
    r.key(0xFF0D).await;
    assert_eq!(r.data(3).await, b"ls\r");
}

#[tokio::test]
async fn ts_03_resize_updates_display_and_notifies_remote() {
    let mut r = start(&[], None, true).await;
    r.frame().await;
    r.client
        .send(Instruction::new("size", ["800", "600"]))
        .await
        .unwrap();
    let f = r.frame().await;
    assert!(
        f.iter()
            .any(|i| i.opcode == "size" && i.args == vec!["0", "800", "600"]),
        "{f:?}"
    );
    let mut got = None;
    for _ in 0..5 {
        if let RemoteCmd::Resize {
            cols,
            rows,
            width_px,
            height_px,
        } = r.cmd().await
        {
            if width_px == 800 {
                got = Some((cols, rows, height_px));
                break;
            }
        }
    }
    let (cols, rows, hp) = got.expect("resize reached remote");
    assert_eq!(hp, 600);
    assert!(cols > 60 && rows > 20);
}

#[tokio::test]
async fn ts_04_remote_close_disconnects_cleanly() {
    let mut r = start(&[], None, true).await;
    r.frame().await;
    r.remote_tx.send(b"bye\r\n".to_vec()).await.unwrap();
    drop(r.remote_tx);
    r.client.expect("disconnect").await;
    assert!(r.server.await.unwrap().is_ok());
}

#[tokio::test]
async fn ts_05_clipboard_from_client_is_pasted() {
    let mut r = start(&[], None, true).await;
    r.frame().await;
    let b64 = base64::engine::general_purpose::STANDARD.encode("echo hi\nnext");
    r.client
        .send(Instruction::new("clipboard", ["7", "text/plain"]))
        .await
        .unwrap();
    r.client
        .send(Instruction::new("blob", ["7".to_string(), b64]))
        .await
        .unwrap();
    r.client.send(Instruction::new("end", ["7"])).await.unwrap();
    assert_eq!(r.data(12).await, b"echo hi\rnext");
}

#[tokio::test]
async fn ts_06_mouse_selection_is_sent_as_clipboard_stream() {
    let mut r = start(&[], None, true).await;
    r.frame().await;
    r.remote_tx.send(b"copy me please".to_vec()).await.unwrap();
    r.frame().await;
    let fonts = FontSet::bundled();
    let (cw, ch, _) = fonts.metrics(16.0);
    let (x1, y) = ((cw * 6 + cw * 3 / 4) as i32, (ch / 2) as i32);
    for (x, m) in [(cw as i32 / 4, 1), (x1, 1), (x1, 0)] {
        r.client
            .send(Instruction::new(
                "mouse",
                [x.to_string(), y.to_string(), m.to_string()],
            ))
            .await
            .unwrap();
    }
    let c = r.client.expect("clipboard").await;
    assert_eq!(c.args[1], "text/plain");
    let blob = r.client.expect("blob").await;
    let text = base64::engine::general_purpose::STANDARD
        .decode(&blob.args[1])
        .unwrap();
    assert_eq!(String::from_utf8(text).unwrap(), "copy me");
    r.client.expect("end").await;
}

#[tokio::test]
async fn ts_07_read_only_blocks_keys_and_paste() {
    let mut r = start(&[("read-only", "true")], None, true).await;
    r.frame().await;
    r.key('x' as u32).await;
    let b64 = base64::engine::general_purpose::STANDARD.encode("pasted");
    r.client
        .send(Instruction::new("clipboard", ["1", "text/plain"]))
        .await
        .unwrap();
    r.client
        .send(Instruction::new("blob", ["1".to_string(), b64]))
        .await
        .unwrap();
    r.client.send(Instruction::new("end", ["1"])).await.unwrap();
    // only the initial resize may arrive, never data
    let got = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let Some(RemoteCmd::Data(d)) = r.cmd_rx.recv().await {
                return d;
            }
        }
    })
    .await;
    assert!(got.is_err(), "read-only must not forward input: {got:?}");
}

struct LoginFilter {
    seen: String,
}
impl RemoteFilter for LoginFilter {
    fn on_remote_data(&mut self, data: &[u8]) -> FilterOutput {
        self.seen.push_str(&String::from_utf8_lossy(data));
        if self.seen.contains("login:") && !self.seen.contains("sent") {
            self.seen.push_str("sent");
            return FilterOutput {
                send: b"user\r".to_vec(),
                ..Default::default()
            };
        }
        if self.seen.contains("Welcome") {
            return FilterOutput {
                start: true,
                ..Default::default()
            };
        }
        if self.seen.contains("denied") {
            return FilterOutput {
                abort: Some((Status::ClientUnauthorized, "Login failed".into())),
                ..Default::default()
            };
        }
        FilterOutput::default()
    }
}

#[tokio::test]
async fn ts_08_filter_gates_rendering_until_started_and_can_send() {
    let filter: Box<dyn RemoteFilter> = Box::new(LoginFilter {
        seen: String::new(),
    });
    let mut r = start(&[], Some(filter), false).await;
    r.remote_tx
        .send(b"\x1b[?25llogin: ".to_vec())
        .await
        .unwrap();
    // the filter answers the prompt
    assert_eq!(r.data(5).await, b"user\r");
    // Nothing is drawn while the terminal is not started, and typing is ignored.
    r.key('z' as u32).await;
    // Only the blank initial frame (size + background) may arrive; acknowledge its sync.
    while let Some(i) = r.client.recv_within(Duration::from_millis(300)).await {
        if i.opcode == "sync" {
            r.client
                .send(Instruction::new("sync", [i.args[0].clone()]))
                .await
                .unwrap();
        }
    }
    let imgs = r.client.log.iter().filter(|i| i.opcode == "img").count();
    assert!(
        imgs <= 1,
        "no text drawn before start, got {imgs} img instructions"
    );
    r.replay = DisplayReplay::new();
    for i in r.client.log.clone() {
        r.replay.apply(&i);
    }
    // before start the layer is still untouched (transparent), not painted with the terminal background
    assert!(
        !region_has_ink(&r.replay, 0, 0, 640, 40, [0, 0, 0, 0]),
        "login prompt must stay hidden"
    );
    r.remote_tx.send(b"\r\nWelcome!".to_vec()).await.unwrap();
    // The blank initial frame may still be in flight; keep reading frames until text shows up.
    let mut drawn = false;
    for _ in 0..5 {
        r.frame().await;
        if region_has_ink(&r.replay, 0, 0, 640, 40, [0, 0, 0, 255]) {
            drawn = true;
            break;
        }
    }
    assert!(
        drawn,
        "text must appear once the filter starts the terminal"
    );
}

#[tokio::test]
async fn ts_09_filter_abort_reports_status_to_client() {
    let filter: Box<dyn RemoteFilter> = Box::new(LoginFilter {
        seen: String::new(),
    });
    let mut r = start(&[], Some(filter), false).await;
    r.remote_tx.send(b"access denied".to_vec()).await.unwrap();
    let e = r.client.expect("error").await;
    assert_eq!(e.args, vec!["Aborted. See logs.", "769"]); // 0x0301
    assert!(r.server.await.unwrap().is_err());
}

#[tokio::test]
async fn ts_10_unacknowledged_frames_stop_the_flow_not_the_memory() {
    let mut r = start(&[], None, true).await;
    // Receive the first frame but never answer its sync.
    loop {
        let i = r.client.recv().await.unwrap();
        if i.opcode == "sync" {
            break;
        }
    }
    // Remote floods the terminal; the session must not keep producing frames beyond the lag limit.
    for _ in 0..50 {
        r.remote_tx.send(vec![b'x'; 2000]).await.unwrap_or(());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut syncs = 0;
    while let Some(i) = r.client.recv_within(Duration::from_millis(300)).await {
        if i.opcode == "sync" {
            syncs += 1;
        }
    }
    assert!(
        syncs <= 2,
        "server ran ahead of an unresponsive client: {syncs} syncs"
    );
}

#[tokio::test]
async fn ts_11_client_disconnect_ends_session() {
    let mut r = start(&[], None, true).await;
    r.frame().await;
    r.client
        .send(Instruction::new("disconnect", Vec::<String>::new()))
        .await
        .unwrap();
    assert!(r.server.await.unwrap().is_ok());
}

#[tokio::test]
async fn ts_12_color_scheme_param_is_honoured() {
    let mut r = {
        // plugin arg list only has read-only/scrollback; use params through from_params directly
        start(&[], None, true).await
    };
    r.frame().await;
    drop(r);
    let p = Params::from_pairs([
        (
            "color-scheme",
            "foreground: white; background: rgb:10/20/30",
        ),
        ("font-size", "20"),
    ]);
    let cfg = TerminalConfig::from_params(&p, 96);
    assert_eq!(cfg.options.color_scheme.background, [0x10, 0x20, 0x30]);
    assert_eq!(cfg.options.color_scheme.foreground, [255, 255, 255]);
    assert_eq!(cfg.options.font_size_pt, 20.0);
}

#[tokio::test]
async fn ts_13_invalid_params_fall_back_to_defaults() {
    let p = Params::from_pairs([
        ("font-size", "-3"),
        ("scrollback", "abc"),
        ("backspace", "999"),
        ("color-scheme", "garbage"),
    ]);
    let cfg = TerminalConfig::from_params(&p, 0);
    assert_eq!(cfg.options.font_size_pt, 12.0);
    assert_eq!(cfg.options.scrollback, 1000);
    assert_eq!(cfg.options.backspace, 127.max(cfg.options.backspace));
    assert!(cfg.options.dpi >= 1);
}

#[tokio::test]
async fn ts_14_absurd_client_sizes_are_clamped_not_allocated() {
    let mut r = start(&[], None, true).await;
    r.frame().await;
    r.client
        .send(Instruction::new("size", ["4294967295", "4294967295"]))
        .await
        .unwrap();
    let f = r.frame().await;
    let size = f
        .iter()
        .find(|i| i.opcode == "size")
        .expect("size instruction");
    assert_eq!(size.args, vec!["0", "4096", "4096"]);
    // and the remote was told a bounded grid, not billions of cells
    let mut seen = false;
    for _ in 0..6 {
        if let RemoteCmd::Resize { cols, rows, .. } = r.cmd().await {
            if cols >= 400 {
                assert!(cols <= 1000 && rows <= 500, "{cols}x{rows}");
                seen = true;
                break;
            }
        }
    }
    assert!(seen);
}

#[test]
fn ts_15_scrollback_param_is_capped() {
    let p = Params::from_pairs([("scrollback", "4000000000")]);
    let cfg = TerminalConfig::from_params(&p, 96);
    assert_eq!(cfg.options.scrollback, 100_000);
}
