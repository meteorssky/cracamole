use guacd::config::*;
use guacd::logging::{format_line, level_name};

fn args(a: &[&str]) -> Result<Config, ConfigError> {
    let mut c = Config::default();
    c.apply_args(a.iter().map(|s| s.to_string()))?;
    Ok(c)
}

#[test]
fn gd_cli_01_defaults_match_c() {
    let c = Config::default();
    assert_eq!(
        (c.bind_host.as_str(), c.bind_port.as_str()),
        ("localhost", "4822")
    );
    assert_eq!(c.log_level, LogLevel::Info);
    assert!(!c.foreground && !c.print_version && c.pid_file.is_none());
}

#[test]
fn gd_cli_02_docker_entrypoint_invocation() {
    // The official entrypoint runs: guacd -f -b 0.0.0.0 -L <level>
    let c = args(&["-f", "-b", "0.0.0.0", "-L", "debug"]).unwrap();
    assert!(c.foreground);
    assert_eq!(c.bind_host, "0.0.0.0");
    assert_eq!(c.log_level, LogLevel::Debug);
}

#[test]
fn gd_cli_03_attached_values_and_combined_flags() {
    let c = args(&["-l4999", "-b127.0.0.1", "-fv"]).unwrap();
    assert_eq!(
        (c.bind_port.as_str(), c.bind_host.as_str()),
        ("4999", "127.0.0.1")
    );
    assert!(c.foreground && c.print_version);
}

#[test]
fn gd_cli_04_all_log_levels() {
    for (s, l) in [
        ("trace", LogLevel::Trace),
        ("debug", LogLevel::Debug),
        ("info", LogLevel::Info),
        ("warning", LogLevel::Warning),
        ("error", LogLevel::Error),
    ] {
        assert_eq!(args(&["-L", s]).unwrap().log_level, l);
    }
}

#[test]
fn gd_cli_05_invalid_level_message_matches_c() {
    let e = args(&["-L", "verbose"]).unwrap_err();
    assert_eq!(
        e.0,
        "Invalid log level. Valid levels are: \"trace\", \"debug\", \"info\", \"warning\", and \"error\"."
    );
}

#[test]
fn gd_cli_06_missing_value_unknown_flag_and_stray_args() {
    assert!(args(&["-l"]).is_err());
    assert!(args(&["-z"]).is_err());
    assert!(args(&["positional"]).is_err());
    assert!(
        args(&["-h"]).is_err(),
        "C guacd has no -h; it prints usage and fails"
    );
}

#[test]
fn gd_cli_07_tls_flags_rejected_clearly() {
    let e = args(&["-C", "cert.pem"]).unwrap_err();
    assert!(e.0.contains("SSL/TLS support"), "{e}");
    assert!(args(&["-K", "key.pem"]).is_err());
}

#[test]
fn gd_cli_08_pid_file_flag() {
    assert_eq!(
        args(&["-p", "/run/guacd.pid"]).unwrap().pid_file.as_deref(),
        Some("/run/guacd.pid")
    );
}

#[test]
fn gd_cli_09_version_line_format() {
    assert!(VERSION_LINE.starts_with("Guacamole proxy daemon (guacd) version "));
    assert!(usage("guacd").starts_with("USAGE: guacd"));
}

#[test]
fn gd_cfg_01_server_and_daemon_sections() {
    let mut c = Config::default();
    c.apply_conf(
        "[server]\nbind_host = 0.0.0.0\nbind_port = 4000\n\n# comment\n[daemon]\npid_file = /run/g.pid\nlog_level = debug\n",
    )
    .unwrap();
    assert_eq!(
        (c.bind_host.as_str(), c.bind_port.as_str()),
        ("0.0.0.0", "4000")
    );
    assert_eq!(c.pid_file.as_deref(), Some("/run/g.pid"));
    assert_eq!(c.log_level, LogLevel::Debug);
}

#[test]
fn gd_cfg_02_cli_overrides_file() {
    let mut c = Config::default();
    c.apply_conf("[server]\nbind_port = 4000\n").unwrap();
    c.apply_args(["-l".to_string(), "5000".to_string()])
        .unwrap();
    assert_eq!(c.bind_port, "5000");
}

#[test]
fn gd_cfg_03_errors() {
    let bad = |s: &str| Config::default().apply_conf(s).is_err();
    assert!(bad("[server]\nbogus = 1\n"), "unknown parameter");
    assert!(bad("[nosuch]\nx = 1\n"), "unknown section");
    assert!(bad("[server\nbind_port = 1\n"), "unterminated section");
    assert!(bad("[server]\nno equals sign\n"));
    assert!(bad("[daemon]\nlog_level = loud\n"));
    assert!(bad("[ssl]\nserver_certificate = x\n"), "TLS not supported");
}

#[test]
fn gd_cfg_04_quotes_comments_blank_lines() {
    let mut c = Config::default();
    c.apply_conf("\n\n[server]\nbind_host = \"10.0.0.1\"  # inline\n")
        .unwrap();
    assert_eq!(c.bind_host, "10.0.0.1");
}

#[test]
fn gd_log_01_format_matches_c() {
    assert_eq!(
        format_line(42, &tracing::Level::INFO, "hello"),
        "guacd[42]: INFO:\thello"
    );
    assert_eq!(
        format_line(1, &tracing::Level::WARN, "x"),
        "guacd[1]: WARNING:\tx"
    );
    assert_eq!(level_name(&tracing::Level::ERROR), "ERROR");
    assert_eq!(level_name(&tracing::Level::DEBUG), "DEBUG");
    assert_eq!(level_name(&tracing::Level::TRACE), "TRACE");
}
