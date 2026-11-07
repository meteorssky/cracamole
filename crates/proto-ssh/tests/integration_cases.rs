mod common;

use base64::Engine;
use common::{keygen, start_server, Behaviour, TestServer};
use guac_protocol::Instruction;
use guac_session::*;
use guac_testkit::{DisplayReplay, SimClient};
use proto_ssh::SshPlugin;
use russh::keys::PublicKeyBase64;
use russh::MethodKind;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::duplex;

struct Rig {
    client: SimClient,
    server: tokio::task::JoinHandle<Result<(), SessionError>>,
    replay: DisplayReplay,
}

async fn connect(srv: &TestServer, values: &[(&str, &str)]) -> Rig {
    let port = srv.addr.port().to_string();
    let key = srv.host_key.public_key().to_openssh().unwrap();
    let mut v: Vec<(&str, &str)> = vec![
        ("hostname", "127.0.0.1"),
        ("port", &port),
        ("timeout", "10"),
    ];
    if !values.iter().any(|(name, _)| *name == "host-key") {
        v.push(("host-key", &key));
    }
    v.extend_from_slice(values);
    connect_raw(&v).await
}

async fn connect_raw(values: &[(&str, &str)]) -> Rig {
    let plugin: Arc<dyn ProtocolPlugin> = Arc::new(SshPlugin);
    let (c, s) = duplex(1 << 20);
    let server = tokio::spawn(async move {
        let (r, mut w) = tokio::io::split(s);
        let mut reader = InstructionReader::new(r);
        reader.read().await?;
        let hs = handshake(
            &mut reader,
            &mut w,
            plugin.arg_names(),
            "$t",
            HANDSHAKE_TIMEOUT,
        )
        .await?;
        run_session(
            reader,
            w,
            hs,
            plugin,
            SessionConfig {
                shutdown_grace: Duration::from_millis(300),
                ..Default::default()
            },
        )
        .await
    });
    let mut client = SimClient::new(c);
    client.handshake("ssh", values, 800, 480).await;
    Rig {
        client,
        server,
        replay: DisplayReplay::new(),
    }
}

impl Rig {
    async fn pump(&mut self, ms: u64) {
        while let Some(i) = self.client.recv_within(Duration::from_millis(ms)).await {
            if i.opcode == "sync" {
                self.client
                    .send(Instruction::new("sync", [i.args[0].clone()]))
                    .await
                    .unwrap();
            }
            self.replay.apply(&i);
        }
    }

    async fn pump_until<F: Fn(&Rig) -> bool>(&mut self, what: &str, f: F) {
        for _ in 0..40 {
            self.pump(150).await;
            if f(self) {
                return;
            }
        }
        panic!("timed out waiting for: {what}");
    }

    fn ink(&self, rows_px: usize) -> bool {
        (0..rows_px).any(|y| {
            (0..400).any(|x| {
                self.replay
                    .pixel(0, x, y)
                    .is_some_and(|p| p != [0, 0, 0, 255] && p != [0, 0, 0, 0])
            })
        })
    }

    async fn type_text(&mut self, s: &str) {
        for ch in s.chars() {
            let k = if ch == '\r' {
                0xFF0D
            } else if ch as u32 > 0xff {
                0x0100_0000 | ch as u32
            } else {
                ch as u32
            };
            for p in ["1", "0"] {
                self.client
                    .send(Instruction::new("key", [k.to_string(), p.to_string()]))
                    .await
                    .unwrap();
            }
        }
    }

    async fn wait_disconnect(&mut self) {
        loop {
            let i = self.client.recv().await.expect("EOF before disconnect");
            if i.opcode == "sync" {
                let _ = self
                    .client
                    .send(Instruction::new("sync", [i.args[0].clone()]))
                    .await;
            }
            if i.opcode == "disconnect" {
                return;
            }
        }
    }
}

async fn wait_until<F: Fn() -> bool>(what: &str, f: F) {
    for _ in 0..100 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for: {what}");
}

async fn copy_first_four_cells(rig: &mut Rig) -> String {
    let (cw, ch, _) = guac_terminal::FontSet::bundled().metrics(16.0);
    for (x, mask) in [
        (cw / 4, 1),
        (cw * 3 + cw * 3 / 4, 1),
        (cw * 3 + cw * 3 / 4, 0),
    ] {
        rig.client
            .send(Instruction::new(
                "mouse",
                [x.to_string(), (ch / 2).to_string(), mask.to_string()],
            ))
            .await
            .unwrap();
    }
    let mut id = None;
    let mut bytes = Vec::new();
    loop {
        let instruction = rig.client.recv().await.unwrap();
        match instruction.opcode.as_str() {
            "sync" => rig
                .client
                .send(Instruction::new("sync", [&instruction.args[0]]))
                .await
                .unwrap(),
            "clipboard" => id = Some(instruction.args[0].clone()),
            "blob" if id.as_ref() == instruction.args.first() => bytes.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(&instruction.args[1])
                    .unwrap(),
            ),
            "end" if id.as_ref() == instruction.args.first() => {
                return String::from_utf8(bytes).unwrap()
            }
            _ => {}
        }
    }
}

fn encoding_sentinel_drawn(rig: &Rig) -> bool {
    let (cw, ch, _) = guac_terminal::FontSet::bundled().metrics(16.0);
    (0..ch).any(|y| {
        (cw * 4..cw * 5).any(|x| {
            rig.replay
                .pixel(0, x as usize, y as usize)
                .is_some_and(|p| p[3] == 255 && p[..3] != [0, 0, 0])
        })
    })
}

#[tokio::test]
async fn ssh_legacy_text_decodes_chunks_and_encodes_keyboard_and_clipboard() {
    for (name, text, wire) in [
        ("Big5", "中文", vec![0xa4, 0xa4, 0xa4, 0xe5]),
        ("GBK", "中文", vec![0xd6, 0xd0, 0xce, 0xc4]),
        ("Shift_JIS", "日本", vec![0x93, 0xfa, 0x96, 0x7b]),
        ("EUC-JP", "日本", vec![0xc6, 0xfc, 0xcb, 0xdc]),
        ("EUC-KR", "가나", vec![0xb0, 0xa1, 0xb3, 0xaa]),
    ] {
        let mut raw_greeting = vec![(false, b"\x1b[?25l".to_vec())];
        raw_greeting.extend(wire.iter().map(|b| (false, vec![*b])));
        raw_greeting.push((false, b"z".to_vec()));
        let srv = start_server(Behaviour {
            raw_greeting,
            ..Default::default()
        })
        .await;
        let mut rig = connect(
            &srv,
            &[
                ("username", "alice"),
                ("password", "secret"),
                ("character-encoding", name),
            ],
        )
        .await;
        rig.pump_until("multibyte greeting", encoding_sentinel_drawn)
            .await;
        assert_eq!(copy_first_four_cells(&mut rig).await, text, "{name} output");
        rig.type_text(text).await;
        wait_until("encoded keyboard", || {
            srv.rec.lock().unwrap().received == wire
        })
        .await;
        rig.client
            .send(Instruction::new("clipboard", ["3", "text/plain"]))
            .await
            .unwrap();
        rig.client
            .send(Instruction::new(
                "blob",
                [
                    "3".into(),
                    base64::engine::general_purpose::STANDARD.encode(text),
                ],
            ))
            .await
            .unwrap();
        rig.client
            .send(Instruction::new("end", ["3"]))
            .await
            .unwrap();
        wait_until("encoded clipboard", || {
            srv.rec.lock().unwrap().received == [wire.clone(), wire.clone()].concat()
        })
        .await;
        rig.client
            .send(Instruction::new("disconnect", Vec::<String>::new()))
            .await
            .unwrap();
        assert!(rig.server.await.unwrap().is_ok());
    }
}

#[tokio::test]
async fn ssh_stdout_and_stderr_decode_independently() {
    let srv = start_server(Behaviour {
        raw_greeting: vec![
            (false, b"\x1b[?25l".to_vec()),
            (false, vec![0xa4]),
            (true, vec![0xba]),
            (false, vec![0xa4]),
            (true, vec![0x7e]),
            (false, b"z".to_vec()),
        ],
        ..Default::default()
    })
    .await;
    let mut rig = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("character-encoding", "Big5"),
        ],
    )
    .await;
    rig.pump_until("greeting", encoding_sentinel_drawn).await;
    assert_eq!(copy_first_four_cells(&mut rig).await, "中漢");
}

#[tokio::test]
async fn ssh_legacy_unrepresentable_paste_never_sends_a_command_prefix() {
    let srv = start_server(Behaviour::default()).await;
    let mut rig = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("character-encoding", "Big5"),
        ],
    )
    .await;
    rig.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    rig.client
        .send(Instruction::new("clipboard", ["3", "text/plain"]))
        .await
        .unwrap();
    rig.client
        .send(Instruction::new(
            "blob",
            [
                "3".into(),
                base64::engine::general_purpose::STANDARD.encode("prefix😀suffix\n"),
            ],
        ))
        .await
        .unwrap();
    rig.client
        .send(Instruction::new("end", ["3"]))
        .await
        .unwrap();
    assert_eq!(
        rig.client.expect("error").await.args[1],
        Status::ClientBadType.code().to_string()
    );
    assert!(srv.rec.lock().unwrap().received.is_empty());
}

// ---------- SSH-I: integration ----------

#[tokio::test]
async fn ssh_i_01_password_login_shell_echo_and_exit() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    r.pump_until("greeting drawn", |r| r.ink(60)).await;

    {
        let rec = srv.rec.lock().unwrap();
        assert_eq!(rec.auth_user.as_deref(), Some("alice"));
        assert!(rec.shell);
        let (term, cols, rows) = rec.pty.clone().unwrap();
        assert_eq!(term, "linux");
        assert!(
            cols >= 80 && rows >= 24,
            "PTY sized from the client: {cols}x{rows}"
        );
        assert!(
            rec.modes
                .iter()
                .any(|(m, v)| *m == russh::Pty::VERASE && *v == 127),
            "{:?}",
            rec.modes
        );
    }

    r.type_text("ls\r").await;
    wait_until("server got keys", || {
        srv.rec.lock().unwrap().received.ends_with(b"ls\r")
    })
    .await;
    r.type_text("exit\r").await;
    r.wait_disconnect().await;
    assert!(r.server.await.unwrap().is_ok());
}

#[tokio::test]
async fn ssh_i_02_wrong_password_reports_unauthorized() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "WRONG")]).await;
    let e = r.client.expect("error").await;
    assert_eq!(e.args, vec!["Aborted. See logs.", "769"]); // CLIENT_UNAUTHORIZED
}

#[tokio::test]
async fn ssh_i_03_prompts_for_username_and_password_in_terminal() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[]).await;
    r.pump(300).await;
    assert!(r.ink(60), "'Login as: ' prompt is drawn");
    r.type_text("alice\r").await;
    r.pump(300).await;
    r.type_text("secret\r").await;
    r.pump_until("shell started", |_| srv.rec.lock().unwrap().shell)
        .await;
    assert_eq!(srv.rec.lock().unwrap().auth_user.as_deref(), Some("alice"));
}

#[tokio::test]
async fn ssh_i_04_password_prompt_does_not_echo() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[("username", "alice")]).await;
    r.pump(400).await;
    let before = r.ink(60); // "Password: " prompt visible
    assert!(before);
    // Capture row-0 pixels, type a secret, and make sure the row did not change (no echo).
    let snapshot = |r: &Rig| -> Vec<Option<[u8; 4]>> {
        (0..20)
            .flat_map(|y| (0..400).map(move |x| (x, y)))
            .map(|(x, y)| r.replay.pixel(0, x, y))
            .collect()
    };
    let a = snapshot(&r);
    r.type_text("sec").await;
    r.pump(300).await;
    // cursor is not moved by non-echoed input, so the prompt row is unchanged
    assert_eq!(a, snapshot(&r), "typed password characters must not appear");
    r.type_text("ret\r").await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
}

#[tokio::test]
async fn ssh_i_05_backspace_edits_prompt_input() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[]).await;
    r.pump(300).await;
    // "alicx" <BS> "e" Enter -> "alice"
    r.type_text("alicx").await;
    for p in ["1", "0"] {
        r.client
            .send(Instruction::new(
                "key",
                ["65288".to_string(), p.to_string()],
            ))
            .await
            .unwrap(); // BackSpace
    }
    r.type_text("e\r").await;
    r.pump(300).await;
    r.type_text("secret\r").await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    assert_eq!(srv.rec.lock().unwrap().auth_user.as_deref(), Some("alice"));
}

#[tokio::test]
async fn ssh_i_06_public_key_auth_ed25519_and_rsa() {
    for (kind, bits) in [("ed25519", None), ("rsa", Some("2048"))] {
        let Some((pem, public)) = keygen(kind, bits, "") else {
            eprintln!("SKIP: ssh-keygen unavailable");
            return;
        };
        let beh = Behaviour {
            password: None,
            authorized_keys: vec![public],
            ..Default::default()
        };
        let srv = start_server(beh).await;
        let mut r = connect(&srv, &[("username", "alice"), ("private-key", &pem)]).await;
        r.pump_until("shell via publickey", |_| srv.rec.lock().unwrap().shell)
            .await;
        assert!(
            srv.rec
                .lock()
                .unwrap()
                .auth_methods_tried
                .contains(&"publickey".to_string()),
            "{kind}"
        );
        r.client
            .send(Instruction::new("disconnect", Vec::<String>::new()))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn ssh_i_07_passphrase_param_and_prompt() {
    let Some((pem, public)) = keygen("ed25519", None, "pp123") else {
        eprintln!("SKIP: ssh-keygen unavailable");
        return;
    };
    // (a) passphrase supplied as a parameter
    let beh = Behaviour {
        password: None,
        authorized_keys: vec![public.clone()],
        ..Default::default()
    };
    let srv = start_server(beh.clone()).await;
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("private-key", &pem),
            ("passphrase", "pp123"),
        ],
    )
    .await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;

    // (b) no passphrase: the terminal prompts "Key passphrase: "
    let srv = start_server(beh).await;
    let mut r = connect(&srv, &[("username", "alice"), ("private-key", &pem)]).await;
    r.pump(400).await;
    assert!(r.ink(60), "passphrase prompt shown");
    r.type_text("pp123\r").await;
    r.pump_until("shell after prompt", |_| srv.rec.lock().unwrap().shell)
        .await;
}

#[tokio::test]
async fn ssh_i_08_wrong_passphrase_aborts_unauthorized() {
    let Some((pem, public)) = keygen("ed25519", None, "right") else {
        eprintln!("SKIP: ssh-keygen unavailable");
        return;
    };
    let beh = Behaviour {
        password: None,
        authorized_keys: vec![public],
        ..Default::default()
    };
    let srv = start_server(beh).await;
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("private-key", &pem),
            ("passphrase", "wrong"),
        ],
    )
    .await;
    let e = r.client.expect("error").await;
    assert_eq!(e.args[1], "769");
}

#[tokio::test]
async fn ssh_i_09_unauthorized_key_is_rejected() {
    let Some((pem, _)) = keygen("ed25519", None, "") else {
        return;
    };
    let Some((_, other_pub)) = keygen("ed25519", None, "") else {
        return;
    };
    let beh = Behaviour {
        password: None,
        authorized_keys: vec![other_pub],
        ..Default::default()
    };
    let srv = start_server(beh).await;
    let mut r = connect(&srv, &[("username", "alice"), ("private-key", &pem)]).await;
    let e = r.client.expect("error").await;
    assert_eq!(e.args[1], "769");
}

#[tokio::test]
async fn ssh_i_10_keyboard_interactive_single_password_prompt() {
    let beh = Behaviour {
        password: None,
        kbd: Some(vec![("Password: ".into(), false, "secret".into())]),
        methods: vec![MethodKind::KeyboardInteractive],
        ..Default::default()
    };
    let srv = start_server(beh).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    r.pump_until("shell via kbd-interactive", |_| {
        srv.rec.lock().unwrap().shell
    })
    .await;
    assert!(srv
        .rec
        .lock()
        .unwrap()
        .auth_methods_tried
        .contains(&"keyboard-interactive".to_string()));
}

#[tokio::test]
async fn ssh_i_11_keyboard_interactive_multiple_prompts_go_to_the_user() {
    let beh = Behaviour {
        password: None,
        kbd: Some(vec![
            ("Password: ".into(), false, "secret".into()),
            ("OTP: ".into(), true, "424242".into()),
        ]),
        methods: vec![MethodKind::KeyboardInteractive],
        ..Default::default()
    };
    let srv = start_server(beh).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    r.pump(500).await;
    assert!(r.ink(60), "prompts are shown in the terminal");
    // with two prompts nothing is answered automatically: user answers both
    r.type_text("secret\r").await;
    r.pump(300).await;
    r.type_text("424242\r").await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    assert_eq!(
        srv.rec.lock().unwrap().kbd_answers,
        vec!["secret", "424242"]
    );
}

#[tokio::test]
async fn ssh_i_12_server_without_password_methods_is_reported() {
    let beh = Behaviour {
        methods: vec![MethodKind::PublicKey],
        ..Default::default()
    };
    let srv = start_server(beh).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    let e = r.client.expect("error").await;
    assert_eq!(e.args[1], "769");
}

#[tokio::test]
async fn ssh_i_13_none_auth_server_logs_in_without_credentials() {
    let beh = Behaviour {
        allow_none: true,
        ..Default::default()
    };
    let srv = start_server(beh).await;
    let mut r = connect(&srv, &[("username", "anyone")]).await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
}

#[tokio::test]
async fn ssh_i_14_command_param_runs_exec_instead_of_shell() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("command", "uptime -p"),
        ],
    )
    .await;
    r.wait_disconnect().await;
    let rec = srv.rec.lock().unwrap();
    assert_eq!(rec.exec.as_deref(), Some("uptime -p"));
    assert!(!rec.shell);
}

#[tokio::test]
async fn ssh_i_15_resize_sends_window_change() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    r.client
        .send(Instruction::new("size", ["1200", "800"]))
        .await
        .unwrap();
    wait_until("window change", || {
        !srv.rec.lock().unwrap().window_changes.is_empty()
    })
    .await;
    let (c, ro) = *srv.rec.lock().unwrap().window_changes.last().unwrap();
    assert!(c > 100 && ro > 35, "{c}x{ro}");
}

#[tokio::test]
async fn ssh_i_16_timezone_and_locale_env() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("timezone", "Asia/Taipei"),
            ("locale", "zh_TW.UTF-8"),
        ],
    )
    .await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    let env = srv.rec.lock().unwrap().env.clone();
    assert!(
        env.contains(&("TZ".into(), "Asia/Taipei".into())),
        "{env:?}"
    );
    assert!(
        env.contains(&("LANG".into(), "zh_TW.UTF-8".into())),
        "{env:?}"
    );
}

#[tokio::test]
async fn ssh_i_17_client_timezone_from_handshake_is_forwarded() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    // SimClient handshake announces timezone "UTC"
    assert!(srv
        .rec
        .lock()
        .unwrap()
        .env
        .contains(&("TZ".into(), "UTC".into())));
}

#[tokio::test]
async fn ssh_i_18_terminal_type_and_backspace_params() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("terminal-type", "xterm-256color"),
            ("backspace", "8"),
        ],
    )
    .await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    let rec = srv.rec.lock().unwrap();
    assert_eq!(rec.pty.as_ref().unwrap().0, "xterm-256color");
    assert!(rec
        .modes
        .iter()
        .any(|(m, v)| *m == russh::Pty::VERASE && *v == 8));
}

#[tokio::test]
async fn ssh_i_19_pty_refused_is_an_upstream_error() {
    let srv = start_server(Behaviour {
        refuse_pty: true,
        ..Default::default()
    })
    .await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    let e = r.client.expect("error").await;
    assert_eq!(e.args[1], "515"); // UPSTREAM_ERROR 0x0203
}

#[tokio::test]
async fn ssh_i_20_connection_refused_maps_to_upstream_unavailable() {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port().to_string();
    drop(l);
    let mut r = connect_raw(&[
        ("hostname", "127.0.0.1"),
        ("port", &port),
        ("username", "a"),
        ("password", "b"),
        ("timeout", "10"),
    ])
    .await;
    let e = r.client.expect("error").await;
    assert_eq!(e.args[1], "520");
}

#[tokio::test]
async fn ssh_i_21_non_ssh_server_is_an_upstream_error_not_a_hang() {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port().to_string();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = l.accept().await.unwrap();
            use tokio::io::AsyncWriteExt;
            let _ = s.write_all(b"220 hello this is SMTP\r\n").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let mut r = connect_raw(&[
        ("hostname", "127.0.0.1"),
        ("port", &port),
        ("username", "a"),
        ("password", "b"),
        ("timeout", "3"),
    ])
    .await;
    let e = r.client.expect("error").await;
    assert!(e.args[1] == "515" || e.args[1] == "514", "{e:?}");
}

#[tokio::test]
async fn ssh_i_22_missing_hostname_is_a_client_error() {
    let mut r = connect_raw(&[("username", "a")]).await;
    let e = r.client.expect("error").await;
    assert_eq!(e.args[1], "768");
}

#[tokio::test]
async fn ssh_i_23_remote_shell_exit_disconnects_client() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    r.type_text("exit\r").await;
    r.wait_disconnect().await;
}

#[tokio::test]
async fn ssh_i_24_client_disconnect_closes_ssh_connection() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    r.client
        .send(Instruction::new("disconnect", Vec::<String>::new()))
        .await
        .unwrap();
    let res = tokio::time::timeout(Duration::from_secs(5), r.server).await;
    assert!(res.expect("session ends promptly").unwrap().is_ok());
}

#[tokio::test]
async fn ssh_i_25_clipboard_paste_reaches_remote() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    let b64 = base64::engine::general_purpose::STANDARD.encode("echo pasted");
    r.client
        .send(Instruction::new("clipboard", ["3", "text/plain"]))
        .await
        .unwrap();
    r.client
        .send(Instruction::new("blob", ["3".to_string(), b64]))
        .await
        .unwrap();
    r.client.send(Instruction::new("end", ["3"])).await.unwrap();
    wait_until("paste at server", || {
        srv.rec.lock().unwrap().received.ends_with(b"echo pasted")
    })
    .await;
}

#[tokio::test]
async fn ssh_i_26_read_only_blocks_input() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("read-only", "true"),
        ],
    )
    .await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    r.type_text("rm -rf\r").await;
    r.pump(400).await;
    assert!(srv.rec.lock().unwrap().received.is_empty());
}

#[tokio::test]
async fn ssh_i_27_binary_and_unicode_output_does_not_break_the_session() {
    let beh = Behaviour {
        greeting: "中文 \u{1F600} \x1b[31mred\x1b[0m \u{7f}\x00\r\n".into(),
        ..Default::default()
    };
    let srv = start_server(beh).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    r.pump_until("drawn", |r| r.ink(60)).await;
}

// ---------- host key verification (SSH-H integration) ----------

fn known_hosts_line(srv: &TestServer, host_part: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD
        .encode(srv.host_key.public_key().public_key_bytes());
    format!("{host_part} ssh-ed25519 {b64}")
}

#[tokio::test]
async fn ssh_h_i_01_matching_host_key_is_accepted() {
    let srv = start_server(Behaviour::default()).await;
    let line = known_hosts_line(&srv, &format!("[127.0.0.1]:{}", srv.addr.port()));
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("host-key", &line),
        ],
    )
    .await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
}

#[tokio::test]
async fn ssh_h_i_02_wrong_host_key_is_refused_before_authentication() {
    let srv = start_server(Behaviour::default()).await;
    let other = start_server(Behaviour::default()).await;
    let line = known_hosts_line(&other, &format!("[127.0.0.1]:{}", srv.addr.port()));
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("host-key", &line),
        ],
    )
    .await;
    let e = r.client.expect("error").await;
    assert_eq!(e.args[1], "515"); // SERVER_ERROR as in C
    assert!(
        srv.rec.lock().unwrap().auth_methods_tried.is_empty(),
        "no credentials may be sent to an unverified host"
    );
}

#[tokio::test]
async fn ssh_h_i_03_entry_for_other_host_means_not_found_and_is_refused() {
    let srv = start_server(Behaviour::default()).await;
    let line = known_hosts_line(&srv, "some.other.host");
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("host-key", &line),
        ],
    )
    .await;
    let e = r.client.expect("error").await;
    assert_eq!(e.args[1], "515");
    assert!(srv.rec.lock().unwrap().auth_methods_tried.is_empty());
}

#[tokio::test]
async fn ssh_h_i_04_unknown_host_is_refused_before_credentials_are_sent() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("host-key", ""),
        ],
    )
    .await;
    assert_eq!(r.client.expect("error").await.args[1], "515");
    assert!(srv.rec.lock().unwrap().auth_methods_tried.is_empty());
}

#[tokio::test]
async fn ssh_host_key_probe_returns_fingerprint_without_authenticating() {
    let srv = start_server(Behaviour::default()).await;
    let info = proto_ssh::probe_host_key("127.0.0.1", srv.addr.port(), None, 2)
        .await
        .unwrap();
    assert_eq!(
        info.public_key,
        srv.host_key.public_key().to_openssh().unwrap()
    );
    assert_eq!(
        info.fingerprint,
        srv.host_key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string()
    );
    assert!(!info.trusted);
    assert!(!info.matches);
    assert!(srv.rec.lock().unwrap().auth_methods_tried.is_empty());
    let trusted =
        proto_ssh::probe_host_key("127.0.0.1", srv.addr.port(), Some(&info.public_key), 2)
            .await
            .unwrap();
    assert!(trusted.trusted && trusted.matches);
    assert!(srv.rec.lock().unwrap().auth_methods_tried.is_empty());
}

#[tokio::test]
async fn ssh_host_key_probe_times_out_and_releases_a_silent_connection() {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 1024];
        loop {
            match stream.read(&mut bytes).await {
                Ok(0) | Err(_) => break,
                _ => {}
            }
        }
    });
    let error = proto_ssh::probe_host_key("127.0.0.1", port, None, 1)
        .await
        .unwrap_err();
    assert_eq!(error.status(), Status::UpstreamTimeout);
    tokio::time::timeout(Duration::from_secs(2), peer)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn ssh_revoked_allowed_key_is_refused_before_authentication() {
    let srv = start_server(Behaviour::default()).await;
    let key = srv.host_key.public_key().to_openssh().unwrap();
    let config = format!("{key}\n@revoked * {key}");
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("host-key", &config),
        ],
    )
    .await;
    assert_eq!(r.client.expect("error").await.args[1], "515");
    assert!(srv.rec.lock().unwrap().auth_methods_tried.is_empty());
}

#[tokio::test]
async fn ssh_h_i_05_bare_key_without_hostname_matches() {
    let srv = start_server(Behaviour::default()).await;
    let b64 = base64::engine::general_purpose::STANDARD
        .encode(srv.host_key.public_key().public_key_bytes());
    let mut r = connect(
        &srv,
        &[
            ("username", "alice"),
            ("password", "secret"),
            ("host-key", &b64),
        ],
    )
    .await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
}

#[tokio::test]
async fn ssh_h_i_06_malformed_configured_key_never_disables_verification() {
    for configured in ["not-a-key", "# comment only", "@revoked * ssh-ed25519 AAAA"] {
        let srv = start_server(Behaviour::default()).await;
        let mut r = connect(
            &srv,
            &[
                ("username", "alice"),
                ("password", "secret"),
                ("host-key", configured),
            ],
        )
        .await;
        let error = r.client.expect("error").await;
        assert_eq!(error.args[1], "512");
        assert!(srv.rec.lock().unwrap().auth_methods_tried.is_empty());
    }
}

#[tokio::test]
async fn ssh_i_28_password_not_logged_or_sent_to_client() {
    let srv = start_server(Behaviour::default()).await;
    let mut r = connect(&srv, &[("username", "alice"), ("password", "secret")]).await;
    r.pump_until("shell", |_| srv.rec.lock().unwrap().shell)
        .await;
    assert!(!r
        .client
        .log
        .iter()
        .any(|i| i.args.iter().any(|a| a.contains("secret"))));
}

#[tokio::test]
async fn ssh_i_29_concurrent_sessions() {
    let srv = start_server(Behaviour::default()).await;
    let mut rigs = vec![];
    for _ in 0..8 {
        rigs.push(connect(&srv, &[("username", "alice"), ("password", "secret")]).await);
    }
    for r in &mut rigs {
        r.pump_until("drawn", |r| r.ink(60)).await;
    }
    for r in &mut rigs {
        r.client
            .send(Instruction::new("disconnect", Vec::<String>::new()))
            .await
            .unwrap();
    }
    for r in rigs {
        assert!(r.server.await.unwrap().is_ok());
    }
}
