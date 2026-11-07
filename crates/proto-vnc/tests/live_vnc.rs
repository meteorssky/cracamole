//! Interoperability checks against a real TigerVNC server (`tests/vnc/Dockerfile`).
//!
//! Ignored by default. Start the fixture, forward its ports, then run
//! `cargo test -p proto-vnc --test live_vnc -- --ignored --test-threads=1`.
//! Environment: `CRACAMOLE_VNC_HOST` (default 127.0.0.1), `CRACAMOLE_VNC_AUTH_PORT` (15901),
//! `CRACAMOLE_VNC_OPEN_PORT` (15902), `CRACAMOLE_VNC_PASSWORD` (fixture1).

use guac_protocol::Instruction;
use guac_session::*;
use guac_testkit::{DisplayReplay, SimClient};
use proto_vnc::VncPlugin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::duplex;

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

struct Rig {
    client: SimClient,
    replay: DisplayReplay,
}

async fn connect(port_var: &str, default_port: &str, extra: &[(&str, &str)]) -> Rig {
    let plugin: Arc<dyn ProtocolPlugin> = Arc::new(VncPlugin);
    let (c, s) = duplex(1 << 20);
    tokio::spawn(async move {
        let (r, mut w) = tokio::io::split(s);
        let mut reader = InstructionReader::new(r);
        reader.read().await?;
        let hs = handshake(
            &mut reader,
            &mut w,
            plugin.arg_names(),
            "$live",
            HANDSHAKE_TIMEOUT,
        )
        .await?;
        run_session(reader, w, hs, plugin, SessionConfig::default()).await
    });
    let host = env("CRACAMOLE_VNC_HOST", "127.0.0.1");
    let port = env(port_var, default_port);
    let mut values = vec![
        ("hostname", host.as_str()),
        ("port", port.as_str()),
        ("timeout", "10"),
    ];
    values.extend_from_slice(extra);
    let mut client = SimClient::new(c);
    client.handshake("vnc", &values, 1024, 768).await;
    Rig {
        client,
        replay: DisplayReplay::new(),
    }
}

impl Rig {
    async fn pump(&mut self, ms: u64) -> Vec<Instruction> {
        let mut seen = vec![];
        while let Some(i) = self.client.recv_within(Duration::from_millis(ms)).await {
            if i.opcode == "sync" {
                self.client
                    .send(Instruction::new("sync", [i.args[0].clone()]))
                    .await
                    .unwrap();
            }
            self.replay.apply(&i);
            seen.push(i);
        }
        seen
    }

    async fn until<F: Fn(&Rig) -> bool>(&mut self, what: &str, f: F) {
        for _ in 0..60 {
            self.pump(150).await;
            if f(self) {
                return;
            }
        }
        panic!("timed out waiting for: {what}");
    }

    fn rgb(&self, x: usize, y: usize) -> [u8; 3] {
        let p = self.replay.pixel(0, x, y).unwrap_or([0; 4]);
        [p[0], p[1], p[2]]
    }

    fn bright_pixels(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> usize {
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|(x, y)| self.rgb(*x, *y).iter().any(|c| *c > 128))
            .count()
    }

    async fn error_status(&mut self) -> u32 {
        loop {
            let i = self.client.recv().await.expect("closed without an error");
            if i.opcode == "error" {
                return i.args[1].parse().unwrap();
            }
        }
    }
}

const BACKGROUND: [u8; 3] = [0x33, 0x66, 0x99];

#[tokio::test]
#[ignore = "needs the TigerVNC fixture (tests/vnc)"]
async fn live_01_password_authentication_paints_the_desktop_and_accepts_typing() {
    let password = env("CRACAMOLE_VNC_PASSWORD", "fixture1");
    let mut rig = connect(
        "CRACAMOLE_VNC_AUTH_PORT",
        "15901",
        &[("password", &password)],
    )
    .await;
    rig.until("desktop background", |r| r.rgb(700, 500) == BACKGROUND)
        .await;
    assert_eq!(rig.replay.size(0), (800, 600));
    // The xterm window is black until something is typed.
    rig.until("xterm window", |r| r.rgb(300, 150) == [0, 0, 0])
        .await;
    let before = rig.bright_pixels(24, 24, 330, 160);

    // No window manager: keyboard focus follows the pointer.
    rig.client
        .send(Instruction::new("mouse", ["200", "100", "0"]))
        .await
        .unwrap();
    for ch in "echo cracamole-live".chars() {
        let sym = (ch as u32).to_string();
        for pressed in ["1", "0"] {
            rig.client
                .send(Instruction::new("key", [sym.as_str(), pressed, "0"]))
                .await
                .unwrap();
        }
    }
    for pressed in ["1", "0"] {
        rig.client
            .send(Instruction::new("key", ["65293", pressed, "0"]))
            .await
            .unwrap();
    }
    rig.until("typed text is rendered by the remote xterm", |r| {
        r.bright_pixels(24, 24, 330, 160) > before + 100
    })
    .await;
}

#[tokio::test]
#[ignore = "needs the TigerVNC fixture (tests/vnc)"]
async fn live_02_wrong_password_is_unauthorized() {
    let mut rig = connect(
        "CRACAMOLE_VNC_AUTH_PORT",
        "15901",
        &[("password", "not-the-password")],
    )
    .await;
    assert_eq!(rig.error_status().await, 0x0301);
}

#[tokio::test]
#[ignore = "needs the TigerVNC fixture (tests/vnc)"]
async fn live_03_missing_password_is_unauthorized() {
    let mut rig = connect("CRACAMOLE_VNC_AUTH_PORT", "15901", &[]).await;
    assert_eq!(rig.error_status().await, 0x0301);
}

#[tokio::test]
#[ignore = "needs the TigerVNC fixture (tests/vnc)"]
async fn live_04_no_authentication_server_works_but_never_receives_credentials() {
    let mut rig = connect("CRACAMOLE_VNC_OPEN_PORT", "15902", &[]).await;
    rig.until("desktop background", |r| r.rgb(600, 450) == BACKGROUND)
        .await;
    assert_eq!(rig.replay.size(0), (640, 480));

    // Credentials configured for this host, but it offers none: refuse rather than silently proceed.
    let mut rig = connect(
        "CRACAMOLE_VNC_OPEN_PORT",
        "15902",
        &[("password", "anything")],
    )
    .await;
    assert_eq!(rig.error_status().await, 0x0203);
}

#[tokio::test]
#[ignore = "needs the TigerVNC fixture (tests/vnc)"]
async fn live_05_each_colour_depth_renders_the_same_background() {
    let password = env("CRACAMOLE_VNC_PASSWORD", "fixture1");
    for depth in ["16", "8"] {
        let mut rig = connect(
            "CRACAMOLE_VNC_AUTH_PORT",
            "15901",
            &[("password", &password), ("color-depth", depth)],
        )
        .await;
        rig.until("desktop background", |r| {
            let p = r.rgb(700, 500);
            // Reduced depths quantise the colour; it must still be close to #336699.
            p[0].abs_diff(0x33) < 40 && p[1].abs_diff(0x66) < 40 && p[2].abs_diff(0x99) < 60
        })
        .await;
    }
}

/// Needs `CRACAMOLE_VNC_RESIZE_CMD`: a shell command that changes the fixture's :1 screen to 1024x768,
/// for example `ssh host docker exec cracamole-vnc-dev sh -c 'DISPLAY=:1 xrandr --fb 1024x768'`.
#[tokio::test]
#[ignore = "needs the TigerVNC fixture (tests/vnc) and CRACAMOLE_VNC_RESIZE_CMD"]
async fn live_06_server_side_resize_follows_the_remote_desktop() {
    let Ok(command) = std::env::var("CRACAMOLE_VNC_RESIZE_CMD") else {
        eprintln!("CRACAMOLE_VNC_RESIZE_CMD not set; skipping");
        return;
    };
    let password = env("CRACAMOLE_VNC_PASSWORD", "fixture1");
    let mut rig = connect(
        "CRACAMOLE_VNC_AUTH_PORT",
        "15901",
        &[("password", &password)],
    )
    .await;
    rig.until("initial desktop", |r| r.replay.size(0) == (800, 600))
        .await;
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .status()
        .expect("run resize command");
    assert!(status.success());
    rig.until("resized desktop", |r| r.replay.size(0) == (1024, 768))
        .await;
    rig.until("background repainted after the resize", |r| {
        r.rgb(1000, 740) == BACKGROUND
    })
    .await;
}

/// Real macOS Screen Sharing (Apple Remote Desktop authentication, security type 30).
/// Needs `CRACAMOLE_VNC_HOST` (the Mac), `CRACAMOLE_VNC_USER` and `CRACAMOLE_VNC_PASSWORD`;
/// the port defaults to 5900 (`CRACAMOLE_VNC_MAC_PORT`). Skips when the user is not set.
#[tokio::test]
#[ignore = "needs a real Mac with Screen Sharing enabled"]
async fn live_mac_01_ard_login_paints_the_desktop_and_wrong_password_is_refused() {
    let Ok(user) = std::env::var("CRACAMOLE_VNC_USER") else {
        eprintln!("CRACAMOLE_VNC_USER not set; skipping");
        return;
    };
    let password = std::env::var("CRACAMOLE_VNC_PASSWORD").expect("CRACAMOLE_VNC_PASSWORD");
    let port = env("CRACAMOLE_VNC_MAC_PORT", "5900");
    std::env::set_var("CRACAMOLE_VNC_MAC_PORT", &port);

    let mut rig = connect(
        "CRACAMOLE_VNC_MAC_PORT",
        "5900",
        &[("username", &user), ("password", &password)],
    )
    .await;
    rig.until("a desktop is drawn", |r| {
        let (w, h) = r.replay.size(0);
        w > 0 && h > 0 && r.bright_pixels(0, 0, w.min(400), h.min(300)) > 1000
    })
    .await;
    eprintln!("Mac desktop size: {:?}", rig.replay.size(0));

    let mut bad = connect(
        "CRACAMOLE_VNC_MAC_PORT",
        "5900",
        &[
            ("username", &user),
            ("password", "definitely-wrong-password"),
        ],
    )
    .await;
    assert_eq!(bad.error_status().await, 0x0301);
}
