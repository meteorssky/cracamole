//! DEP-03..05: the Docker entrypoint is compatible with the official guacamole/guacd image.
//! The script runs against a stub "guacd" that just prints its arguments, so no container is needed.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn sh_available() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("true")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn entrypoint() -> String {
    let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docker/entrypoint.sh");
    p.canonicalize()
        .unwrap()
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/")
}

/// Run the entrypoint with the given env and args; returns (stdout, stderr, success).
fn run(env: &[(&str, &str)], args: &[&str]) -> Option<(String, String, bool)> {
    if !sh_available() {
        eprintln!("SKIP: no `sh` available");
        return None;
    }
    let dir = std::env::temp_dir().join(format!(
        "cracamole-ep-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("stub.sh"), "#!/bin/sh\necho \"ARGS: $*\"\n").unwrap();
    let mut cmd = Command::new("sh");
    cmd.arg(entrypoint())
        .args(args)
        .current_dir(&dir)
        .env("GUACD_BIN", "sh ./stub.sh");
    // `sh ./stub.sh` is not a single path; use a wrapper instead so `exec "$GUACD_BIN"` stays valid.
    std::fs::write(
        dir.join("guacd-stub"),
        "#!/bin/sh\nexec sh ./stub.sh \"$@\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.join("guacd-stub"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    cmd.env("GUACD_BIN", "./guacd-stub");
    cmd.env_remove("LOG_LEVEL").env_remove("GUACD_LOG_LEVEL");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    Some((
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.success(),
    ))
}

#[test]
fn dep_default_invocation_matches_official_entrypoint() {
    let Some((out, err, ok)) = run(&[], &[]) else {
        return;
    };
    assert!(ok, "{err}");
    assert_eq!(out, "ARGS: -f -b 0.0.0.0 -L info");
    assert!(
        err.is_empty(),
        "no warning without deprecated variable: {err}"
    );
}

#[test]
fn dep_03_log_level_variable() {
    let Some((out, err, _)) = run(&[("LOG_LEVEL", "debug")], &[]) else {
        return;
    };
    assert_eq!(out, "ARGS: -f -b 0.0.0.0 -L debug");
    assert!(err.is_empty());
}

#[test]
fn dep_04_deprecated_variable_still_works_and_warns() {
    let Some((out, err, _)) = run(&[("GUACD_LOG_LEVEL", "trace")], &[]) else {
        return;
    };
    assert_eq!(out, "ARGS: -f -b 0.0.0.0 -L trace");
    assert!(err.contains("deprecated"), "{err}");
    assert!(err.contains("LOG_LEVEL"), "{err}");
}

#[test]
fn dep_05_log_level_wins_over_deprecated_name() {
    let Some((out, err, _)) = run(&[("LOG_LEVEL", "error"), ("GUACD_LOG_LEVEL", "trace")], &[])
    else {
        return;
    };
    assert_eq!(out, "ARGS: -f -b 0.0.0.0 -L error");
    assert!(
        err.is_empty(),
        "no deprecation warning when the new name is set: {err}"
    );
}

#[test]
fn dep_extra_arguments_are_passed_through() {
    let Some((out, _, _)) = run(&[], &["-l", "4000"]) else {
        return;
    };
    assert_eq!(out, "ARGS: -f -b 0.0.0.0 -L info -l 4000");
}

#[test]
fn dep_entrypoint_execs_the_daemon_so_it_is_pid_1() {
    let text = std::fs::read_to_string(entrypoint()).unwrap();
    assert!(
        text.contains("exec "),
        "must exec so signals reach guacd directly"
    );
    assert!(text.starts_with("#!/bin/sh"));
}

#[test]
fn dep_dockerfile_matches_deployment_contract() {
    let df = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docker/cracamole.Dockerfile"),
    )
    .unwrap();
    assert!(df.contains("EXPOSE 4822"));
    assert!(df.contains("USER guacd"), "must not run as root");
    assert!(df.contains("HEALTHCHECK"));
    assert!(df.contains("ENTRYPOINT [\"/usr/local/bin/entrypoint.sh\"]"));
    assert!(df.contains("--locked"), "reproducible builds");
}
