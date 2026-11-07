mod common;

use base64::Engine;
use guac_session::Params;
use proto_ssh::hostkey::{self, Verdict};
use proto_ssh::keys::{load_private_key, KeyError};
use proto_ssh::{SshSettings, ARGS};
use russh::keys::PublicKeyBase64;

fn p(pairs: &[(&str, &str)]) -> Params {
    Params::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

// ---------- SSH-P: settings ----------

#[test]
fn ssh_p_01_defaults() {
    let s = SshSettings::from_params(&p(&[("hostname", "h")]), None).unwrap();
    assert_eq!(
        (
            s.port,
            s.timeout_secs,
            s.terminal_type.as_str(),
            s.backspace
        ),
        (22, 10, "linux", 127)
    );
    assert_eq!(s.server_alive_interval, 0);
    assert!(s.username.is_none() && s.password.is_none() && s.private_key.is_none());
}

#[test]
fn ssh_p_02_hostname_required() {
    assert!(SshSettings::from_params(&p(&[]), None).is_none());
    assert!(SshSettings::from_params(&p(&[("hostname", "")]), None).is_none());
}

#[test]
fn ssh_p_03_bad_numbers_fall_back() {
    let s = SshSettings::from_params(
        &p(&[
            ("hostname", "h"),
            ("port", "70000"),
            ("timeout", "x"),
            ("backspace", "9999"),
        ]),
        None,
    )
    .unwrap();
    assert_eq!((s.port, s.timeout_secs, s.backspace), (22, 10, 255));
}

#[test]
fn ssh_p_04_timezone_param_wins_over_client() {
    let s = SshSettings::from_params(
        &p(&[("hostname", "h"), ("timezone", "Asia/Taipei")]),
        Some("UTC"),
    )
    .unwrap();
    assert_eq!(s.timezone.as_deref(), Some("Asia/Taipei"));
    let s = SshSettings::from_params(&p(&[("hostname", "h")]), Some("UTC")).unwrap();
    assert_eq!(s.timezone.as_deref(), Some("UTC"));
}

#[test]
fn ssh_p_05_secrets_never_debug_printed() {
    let s = SshSettings::from_params(
        &p(&[
            ("hostname", "h"),
            ("password", "hunter2"),
            ("passphrase", "pp-secret"),
            ("private-key", "KEYDATA"),
        ]),
        None,
    )
    .unwrap();
    let dbg = format!("{s:?}");
    assert!(
        !dbg.contains("hunter2") && !dbg.contains("pp-secret") && !dbg.contains("KEYDATA"),
        "{dbg}"
    );
}

#[test]
fn ssh_p_06_unsupported_features_warn() {
    let s = SshSettings::from_params(
        &p(&[
            ("hostname", "h"),
            ("enable-sftp", "true"),
            ("recording-path", "/x"),
        ]),
        None,
    )
    .unwrap();
    assert_eq!(s.warnings.len(), 2);
}

#[test]
fn ssh_p_07_args_shape() {
    assert_eq!(ARGS.len(), 45);
    assert_eq!(
        &ARGS[..8],
        [
            "hostname",
            "host-key",
            "port",
            "timeout",
            "username",
            "password",
            "font-name",
            "font-size"
        ]
    );
    assert!(
        !ARGS.contains(&"enable-agent"),
        "agent code does not compile upstream and is off in the default build"
    );
    assert!(
        ARGS.contains(&"color-scheme")
            && ARGS.contains(&"server-alive-interval")
            && ARGS.contains(&"wol-wait-time")
    );
    let mut v = ARGS.to_vec();
    v.sort();
    v.dedup();
    assert_eq!(v.len(), ARGS.len());
}

#[test]
fn ssh_p_08_args_equal_c_source_if_available() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/guacamole-server/src/protocols/ssh/settings.c");
    let Ok(src) = std::fs::read_to_string(&path) else {
        eprintln!("SKIP: reference source not present");
        return;
    };
    let start = src.find("GUAC_SSH_CLIENT_ARGS[] = {").unwrap();
    let body = &src[start..];
    let body = &body[..body.find("NULL").unwrap()];
    let mut names = vec![];
    let mut in_agent = false;
    for line in body.lines().skip(1) {
        let l = line.trim();
        if l.starts_with("#ifdef ENABLE_SSH_AGENT") {
            in_agent = true;
            continue;
        }
        if l.starts_with("#endif") {
            in_agent = false;
            continue;
        }
        if in_agent {
            continue;
        }
        let l = l.trim_end_matches(',');
        if let Some(q) = l.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            names.push(q.to_string());
        } else if l == "GUAC_SSH_ARGV_FONT_NAME" {
            names.push("font-name".into());
        } else if l == "GUAC_SSH_ARGV_FONT_SIZE" {
            names.push("font-size".into());
        } else if l == "GUAC_SSH_ARGV_COLOR_SCHEME" {
            names.push("color-scheme".into());
        }
    }
    assert_eq!(
        names,
        ARGS.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    );
}

// ---------- SSH-H: host keys ----------

fn blob_and_line(host_part: Option<&str>) -> (Vec<u8>, String) {
    let k = common::host_key();
    let blob = k.public_key().public_key_bytes();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&blob);
    let line = match host_part {
        Some(h) => format!("{h} ssh-ed25519 {b64}"),
        None => format!("ssh-ed25519 {b64}"),
    };
    (blob, line)
}

#[test]
fn ssh_h_01_plain_host_match() {
    let (blob, line) = blob_and_line(Some("example.com"));
    let (e, errs) = hostkey::parse(&line);
    assert!(errs.is_empty());
    assert_eq!(
        hostkey::verify(&e, "example.com", 22, &blob),
        Verdict::Match
    );
    assert_eq!(
        hostkey::verify(&e, "EXAMPLE.com", 22, &blob),
        Verdict::Match,
        "case-insensitive"
    );
}

#[test]
fn ssh_h_02_mismatch_and_notfound() {
    let (blob, line) = blob_and_line(Some("example.com"));
    let (_, other_line) = blob_and_line(Some("example.com"));
    let (e, _) = hostkey::parse(&format!("{line}\n"));
    let (e_other, _) = hostkey::parse(&other_line);
    // entry for the host exists but with a different key
    assert_eq!(
        hostkey::verify(&e_other, "example.com", 22, &blob),
        Verdict::Mismatch
    );
    // no entry for the host at all
    assert_eq!(
        hostkey::verify(&e, "other.org", 22, &blob),
        Verdict::NotFound
    );
}

#[test]
fn ssh_h_03_nondefault_port_uses_bracket_form() {
    let (blob, line) = blob_and_line(Some("[example.com]:2222"));
    let (e, _) = hostkey::parse(&line);
    assert_eq!(
        hostkey::verify(&e, "example.com", 2222, &blob),
        Verdict::Match
    );
    assert_eq!(
        hostkey::verify(&e, "example.com", 22, &blob),
        Verdict::NotFound
    );
}

#[test]
fn ssh_h_04_wildcards_and_negation() {
    let (blob, line) = blob_and_line(Some("*.example.com,!bad.example.com"));
    let (e, _) = hostkey::parse(&line);
    assert_eq!(
        hostkey::verify(&e, "web.example.com", 22, &blob),
        Verdict::Match
    );
    assert_eq!(
        hostkey::verify(&e, "bad.example.com", 22, &blob),
        Verdict::NotFound
    );
    assert_eq!(
        hostkey::verify(&e, "example.org", 22, &blob),
        Verdict::NotFound
    );
    let (blob2, line2) = blob_and_line(Some("host?.lan"));
    let (e2, _) = hostkey::parse(&line2);
    assert_eq!(
        hostkey::verify(&e2, "host1.lan", 22, &blob2),
        Verdict::Match
    );
    assert_eq!(
        hostkey::verify(&e2, "host12.lan", 22, &blob2),
        Verdict::NotFound
    );
}

#[test]
fn ssh_h_05_hashed_hostnames() {
    use hmac::{Hmac, KeyInit, Mac};
    let salt = [7u8; 20];
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&salt).unwrap();
    mac.update(b"secret.host");
    let hash = mac.finalize().into_bytes();
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let (blob, line) = blob_and_line(Some(&format!("|1|{}|{}", b64(&salt), b64(&hash))));
    let (e, errs) = hostkey::parse(&line);
    assert!(errs.is_empty());
    assert_eq!(
        hostkey::verify(&e, "secret.host", 22, &blob),
        Verdict::Match
    );
    assert_eq!(
        hostkey::verify(&e, "other.host", 22, &blob),
        Verdict::NotFound
    );
}

#[test]
fn ssh_h_06_bare_key_forms_match_any_host() {
    let (blob, line) = blob_and_line(None); // "ssh-ed25519 <b64>"
    let (e, _) = hostkey::parse(&line);
    assert_eq!(hostkey::verify(&e, "anything", 22, &blob), Verdict::Match);
    // lone base64 blob
    let b64 = base64::engine::general_purpose::STANDARD.encode(&blob);
    let (e, errs) = hostkey::parse(&b64);
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(hostkey::verify(&e, "x", 22, &blob), Verdict::Match);
}

#[test]
fn ssh_h_07_multiple_entries_comments_and_garbage() {
    let (b1, l1) = blob_and_line(Some("a.example"));
    let (b2, l2) = blob_and_line(Some("b.example"));
    let text = format!("# comment\n\n{l1}\nnot a valid entry !!!\n{l2}\n");
    let (e, errs) = hostkey::parse(&text);
    assert_eq!(e.len(), 2);
    assert_eq!(errs.len(), 1);
    assert_eq!(hostkey::verify(&e, "a.example", 22, &b1), Verdict::Match);
    assert_eq!(hostkey::verify(&e, "b.example", 22, &b2), Verdict::Match);
    assert_eq!(hostkey::verify(&e, "a.example", 22, &b2), Verdict::Mismatch);
}

#[test]
fn ssh_h_08_empty_and_hostile_input() {
    assert!(hostkey::parse("").0.is_empty());
    let (e, errs) = hostkey::parse("@cert-authority * ssh-ed25519 AAAA");
    assert!(e.is_empty() && errs.len() == 1);
    let huge = "x".repeat(100_000);
    hostkey::parse(&huge);
    hostkey::parse(&format!("{huge} {huge} {huge}"));
}

#[test]
fn ssh_revocation_overrides_allow_regardless_of_order_and_host_pattern() {
    let (blob, allow) = blob_and_line(Some("host.example"));
    let base64 = base64::engine::general_purpose::STANDARD.encode(&blob);
    let revoked = format!("@revoked *.example ssh-ed25519 {base64}");
    for text in [format!("{allow}\n{revoked}"), format!("{revoked}\n{allow}")] {
        let (entries, errors) = hostkey::parse(&text);
        assert!(errors.is_empty());
        assert_eq!(
            hostkey::verify(&entries, "host.example", 22, &blob),
            Verdict::Mismatch
        );
    }
}

// ---------- SSH-K: private keys (generated by the real ssh-keygen) ----------

#[test]
fn ssh_k_01_unencrypted_openssh_ed25519_and_rsa_and_ecdsa() {
    for (kind, bits) in [
        ("ed25519", None),
        ("rsa", Some("2048")),
        ("ecdsa", Some("256")),
    ] {
        let Some((pem, public)) = common::keygen(kind, bits, "") else {
            eprintln!("SKIP: ssh-keygen unavailable");
            return;
        };
        let k = load_private_key(&pem, None).unwrap_or_else(|e| panic!("{kind}: {e:?}"));
        assert_eq!(k.public_key().key_data(), public.key_data(), "{kind}");
    }
}

#[test]
fn ssh_k_02_encrypted_key_needs_and_accepts_passphrase() {
    let Some((pem, public)) = common::keygen("ed25519", None, "correct horse") else {
        eprintln!("SKIP: ssh-keygen unavailable");
        return;
    };
    assert_eq!(
        load_private_key(&pem, None).unwrap_err(),
        KeyError::NeedsPassphrase
    );
    let k = load_private_key(&pem, Some("correct horse")).unwrap();
    assert_eq!(k.public_key().key_data(), public.key_data());
}

#[test]
fn ssh_k_03_wrong_passphrase_is_rejected() {
    let Some((pem, _)) = common::keygen("ed25519", None, "right") else {
        eprintln!("SKIP: ssh-keygen unavailable");
        return;
    };
    assert!(matches!(
        load_private_key(&pem, Some("wrong")),
        Err(KeyError::Invalid(_))
    ));
}

#[test]
fn ssh_k_04_garbage_and_truncated_keys() {
    assert!(matches!(
        load_private_key("not a key", None),
        Err(KeyError::Invalid(_))
    ));
    assert!(load_private_key("", None).is_err());
    let Some((pem, _)) = common::keygen("ed25519", None, "") else {
        return;
    };
    let truncated = &pem[..pem.len() / 2];
    assert!(load_private_key(truncated, None).is_err());
}

#[test]
fn ssh_k_05_key_with_surrounding_whitespace() {
    let Some((pem, public)) = common::keygen("ed25519", None, "") else {
        return;
    };
    let padded = format!("\n\n  {}\n\n", pem.trim());
    if let Ok(k) = load_private_key(&padded, None) {
        assert_eq!(k.public_key().key_data(), public.key_data());
    }
}
