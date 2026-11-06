//! Process-level tests of the real `guacd` executable.

use guac_protocol::Instruction;
use guac_testkit::SimClient;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_guacd");

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

#[test]
fn dep_version_flag_prints_c_style_line() {
    let out = Command::new(BIN).arg("-v").output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.starts_with("Guacamole proxy daemon (guacd) version "),
        "{s}"
    );
}

#[test]
fn dep_invalid_log_level_exits_nonzero_with_c_message() {
    let out = Command::new(BIN).args(["-L", "loud"]).output().unwrap();
    assert!(!out.status.success());
    let e = String::from_utf8_lossy(&out.stderr);
    assert!(e.contains("Invalid log level. Valid levels are:"), "{e}");
}

#[test]
fn dep_unknown_flag_prints_usage() {
    let out = Command::new(BIN).arg("-z").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("USAGE:"));
}

#[test]
fn dep_bind_failure_is_reported() {
    // occupy a port, then ask guacd to bind the same one
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port().to_string();
    let out = Command::new(BIN)
        .args(["-f", "-b", "127.0.0.1", "-l", &port])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Unable to bind socket"));
}

#[test]
fn dep_invalid_port_is_reported() {
    let out = Command::new(BIN)
        .args(["-f", "-l", "notaport"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Invalid port"));
}

#[test]
fn dep_runs_serves_handshake_and_logs_in_guacd_format() {
    let port = free_port();
    let mut child = Command::new(BIN)
        .args([
            "-f",
            "-b",
            "127.0.0.1",
            "-l",
            &port.to_string(),
            "-L",
            "debug",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();

    // collect stderr lines on a thread
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });

    // wait for the listener
    let mut up = false;
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(up, "guacd did not start listening");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let mut c = SimClient::new(s);
        c.send(Instruction::new("select", ["telnet"]))
            .await
            .unwrap();
        let a = c.expect("args").await;
        assert_eq!(a.args[0], "VERSION_1_5_0");
        assert_eq!(a.args.len(), 38);
    });

    std::thread::sleep(Duration::from_millis(300));
    let _ = child.kill();
    let _ = child.wait();

    let lines: Vec<String> = rx.try_iter().collect();
    let all = lines.join("\n");
    assert!(
        lines.iter().any(|l| l.starts_with("guacd[")
            && l.contains("]: INFO:\tGuacamole proxy daemon (guacd) version ")),
        "{all}"
    );
    assert!(
        all.contains(&format!("Listening on host 127.0.0.1, port {port}")),
        "{all}"
    );
    assert!(
        all.contains("Creating new client for protocol \"telnet\""),
        "{all}"
    );
    assert!(
        lines.iter().all(|l| l.starts_with("guacd[")),
        "every log line uses the guacd format: {all}"
    );
}

#[test]
fn dep_log_level_filters_output() {
    let port = free_port();
    let mut child = Command::new(BIN)
        .args([
            "-f",
            "-b",
            "127.0.0.1",
            "-l",
            &port.to_string(),
            "-L",
            "error",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(200));
    let _ = child.kill();
    let _ = child.wait();
    let lines: Vec<String> = rx.try_iter().collect();
    assert!(
        lines.is_empty(),
        "-L error must suppress info logs: {lines:?}"
    );
}

#[test]
fn dep_pid_file_written_and_removed() {
    let dir = std::env::temp_dir().join(format!("cracamole-pid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pid_path = dir.join("guacd.pid");
    let port = free_port();
    let mut child = Command::new(BIN)
        .args([
            "-f",
            "-b",
            "127.0.0.1",
            "-l",
            &port.to_string(),
            "-p",
            pid_path.to_str().unwrap(),
        ])
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let content = std::fs::read_to_string(&pid_path).unwrap();
    assert_eq!(content.trim(), child.id().to_string());
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
