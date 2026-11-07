use guac_session::Params;
use proto_telnet::codec::*;
use proto_telnet::login::LoginFilter;
use proto_telnet::negotiation::Negotiator;
use proto_telnet::{TelnetSettings, ARGS};

// ---------- TEL-C: codec ----------

fn dec(chunks: &[&[u8]]) -> Vec<Event> {
    let mut d = Decoder::new();
    let mut all = vec![];
    for c in chunks {
        all.extend(d.decode(c));
    }
    // merge adjacent Data events so chunk boundaries don't matter
    let mut out: Vec<Event> = vec![];
    for e in all {
        match (out.last_mut(), e) {
            (Some(Event::Data(a)), Event::Data(b)) => a.extend(b),
            (_, e) => out.push(e),
        }
    }
    out
}

#[test]
fn tel_c_01_plain_data() {
    assert_eq!(dec(&[b"hello"]), vec![Event::Data(b"hello".to_vec())]);
}

#[test]
fn tel_c_02_iac_iac_is_literal_255() {
    assert_eq!(
        dec(&[&[b'a', IAC, IAC, b'b']]),
        vec![Event::Data(vec![b'a', 255, b'b'])]
    );
}

#[test]
fn tel_c_03_negotiation_commands() {
    assert_eq!(
        dec(&[&[IAC, DO, OPT_TTYPE, IAC, WILL, OPT_ECHO, IAC, DONT, 9, IAC, WONT, 9]]),
        vec![
            Event::Do(24),
            Event::Will(1),
            Event::Dont(9),
            Event::Wont(9)
        ]
    );
}

#[test]
fn tel_c_04_split_at_every_byte() {
    let wire = [
        b'x', IAC, DO, OPT_NAWS, b'y', IAC, SB, OPT_TTYPE, 1, IAC, SE, IAC, IAC, b'z',
    ];
    let whole = dec(&[&wire]);
    let singles: Vec<&[u8]> = wire.chunks(1).collect();
    assert_eq!(dec(&singles), whole);
    assert_eq!(
        whole,
        vec![
            Event::Data(b"x".to_vec()),
            Event::Do(31),
            Event::Data(b"y".to_vec()),
            Event::Sb(24, vec![1]),
            Event::Data(vec![255, b'z']),
        ]
    );
}

#[test]
fn tel_c_05_sb_with_escaped_iac_in_payload() {
    assert_eq!(
        dec(&[&[IAC, SB, 99, 1, IAC, IAC, 2, IAC, SE]]),
        vec![Event::Sb(99, vec![1, 255, 2])]
    );
}

#[test]
fn tel_c_06_unknown_commands_do_not_leak_into_data() {
    // IAC NOP, IAC GA
    assert_eq!(
        dec(&[&[b'a', IAC, 241, b'b', IAC, 249, b'c']]),
        vec![
            Event::Data(b"a".to_vec()),
            Event::Command(241),
            Event::Data(b"b".to_vec()),
            Event::Command(249),
            Event::Data(b"c".to_vec()),
        ]
    );
}

#[test]
fn tel_c_07_oversized_subnegotiation_is_dropped() {
    let mut wire = vec![IAC, SB, 77];
    wire.extend(vec![b'a'; MAX_SB + 100]);
    wire.extend([IAC, SE, b'k']);
    assert_eq!(dec(&[&wire]), vec![Event::Data(b"k".to_vec())]);
}

#[test]
fn tel_c_08_malformed_iac_in_sb_recovers() {
    let r = dec(&[&[IAC, SB, 5, 1, IAC, 7, b'o', b'k']]);
    assert_eq!(r, vec![Event::Data(b"ok".to_vec())]);
}

#[test]
fn tel_c_09_escape_and_subnegotiation_builders() {
    assert_eq!(escape_data(&[1, 255, 2]), vec![1, 255, 255, 2]);
    assert_eq!(
        subnegotiation(31, &[0, 80, 0, 255]),
        vec![IAC, SB, 31, 0, 80, 0, 255, 255, IAC, SE]
    );
    assert_eq!(command(WILL, 24), vec![IAC, WILL, 24]);
}

#[test]
fn tel_c_10_arbitrary_bytes_never_panic() {
    let mut d = Decoder::new();
    let mut x = 1u32;
    for _ in 0..200 {
        let chunk: Vec<u8> = (0..257)
            .map(|_| {
                x = x.wrapping_mul(1103515245).wrapping_add(12345);
                (x >> 16) as u8
            })
            .collect();
        d.decode(&chunk);
    }
}

// ---------- TEL-N: negotiation ----------

fn neg() -> Negotiator {
    Negotiator::new("linux", Some("alice"), 80, 24)
}

#[test]
fn tel_n_01_ttype() {
    let mut n = neg();
    assert_eq!(
        n.handle(Event::Do(OPT_TTYPE)).send,
        command(WILL, OPT_TTYPE)
    );
    let r = n.handle(Event::Sb(OPT_TTYPE, vec![1]));
    let mut body = vec![0];
    body.extend(b"linux");
    assert_eq!(r.send, subnegotiation(OPT_TTYPE, &body));
}

#[test]
fn tel_n_02_naws_sends_size_now_and_on_resize() {
    let mut n = neg();
    let mut expect = command(WILL, OPT_NAWS);
    expect.extend(subnegotiation(OPT_NAWS, &[0, 80, 0, 24]));
    assert_eq!(n.handle(Event::Do(OPT_NAWS)).send, expect);
    assert_eq!(
        n.resize(132, 50),
        subnegotiation(OPT_NAWS, &[0, 132, 0, 50])
    );
}

#[test]
fn tel_n_02b_resize_before_naws_sends_nothing() {
    assert!(neg().resize(100, 40).is_empty());
}

#[test]
fn tel_n_02c_naws_value_with_255_is_escaped() {
    let mut n = Negotiator::new("linux", None, 255, 24);
    let r = n.handle(Event::Do(OPT_NAWS)).send;
    assert!(r.windows(2).any(|w| w == [255, 255]), "{r:?}");
}

#[test]
fn tel_n_03_server_echo_disables_local_echo() {
    let mut n = neg();
    assert!(n.local_echo);
    assert_eq!(n.handle(Event::Will(OPT_ECHO)).send, command(DO, OPT_ECHO));
    assert!(!n.local_echo);
    assert_eq!(
        n.handle(Event::Wont(OPT_ECHO)).send,
        command(DONT, OPT_ECHO)
    );
    assert!(n.local_echo);
}

#[test]
fn tel_n_04_unknown_options_refused_once() {
    let mut n = neg();
    assert_eq!(n.handle(Event::Do(200)).send, command(WONT, 200));
    assert!(
        n.handle(Event::Do(200)).send.is_empty(),
        "no negotiation loop"
    );
    assert_eq!(n.handle(Event::Will(201)).send, command(DONT, 201));
    assert!(n.handle(Event::Will(201)).send.is_empty());
}

#[test]
fn tel_n_05_repeated_request_not_answered_twice() {
    let mut n = neg();
    assert!(!n.handle(Event::Do(OPT_TTYPE)).send.is_empty());
    assert!(n.handle(Event::Do(OPT_TTYPE)).send.is_empty());
    assert!(!n.handle(Event::Will(OPT_ECHO)).send.is_empty());
    assert!(n.handle(Event::Will(OPT_ECHO)).send.is_empty());
}

#[test]
fn tel_n_06_mccp2_refused() {
    let mut n = neg();
    assert_eq!(
        n.handle(Event::Will(OPT_COMPRESS2)).send,
        command(DONT, OPT_COMPRESS2)
    );
}

#[test]
fn tel_n_07_new_environ_sends_user() {
    let mut n = neg();
    assert_eq!(
        n.handle(Event::Do(OPT_NEW_ENVIRON)).send,
        command(WILL, OPT_NEW_ENVIRON)
    );
    let r = n.handle(Event::Sb(OPT_NEW_ENVIRON, vec![1]));
    let mut body = vec![0, 0];
    body.extend(b"USER");
    body.push(1);
    body.extend(b"alice");
    assert_eq!(r.send, subnegotiation(OPT_NEW_ENVIRON, &body));
    // a request naming specific variables is not answered
    assert!(n
        .handle(Event::Sb(OPT_NEW_ENVIRON, vec![1, 0, b'X']))
        .send
        .is_empty());
}

#[test]
fn tel_n_07b_new_environ_without_username_sends_empty_is() {
    let mut n = Negotiator::new("linux", None, 80, 24);
    n.handle(Event::Do(OPT_NEW_ENVIRON));
    assert_eq!(
        n.handle(Event::Sb(OPT_NEW_ENVIRON, vec![1])).send,
        subnegotiation(OPT_NEW_ENVIRON, &[0])
    );
}

#[test]
fn tel_n_08_data_passes_through() {
    assert_eq!(neg().handle(Event::Data(b"hi".to_vec())).data, b"hi");
}

#[test]
fn tel_n_09_terminal_type_is_configurable() {
    let mut n = Negotiator::new("xterm-256color", None, 80, 24);
    n.handle(Event::Do(OPT_TTYPE));
    let r = n.handle(Event::Sb(OPT_TTYPE, vec![1])).send;
    assert!(String::from_utf8_lossy(&r).contains("xterm-256color"));
}

// ---------- TEL-L: login ----------

fn filter(
    user: Option<&str>,
    pass: Option<&str>,
    ok: Option<&str>,
    bad: Option<&str>,
) -> LoginFilter {
    use proto_telnet::login::compile;
    LoginFilter::new(
        user.map(str::to_string),
        user.map(|_| compile("[Ll]ogin:").unwrap()),
        pass.map(str::to_string),
        pass.map(|_| compile("[Pp]assword:").unwrap()),
        ok.map(|r| compile(r).unwrap()),
        bad.map(|r| compile(r).unwrap()),
    )
}

#[test]
fn tel_l_01_username_then_password() {
    use guac_terminal::RemoteFilter;
    let mut f = filter(Some("bob"), Some("s3cret"), None, None);
    assert_eq!(f.on_remote_data(b"Welcome\r\nhost login: ").send, b"bob\r");
    // prompt text keeps arriving in the same unfinished line: must not fire twice
    assert!(f.on_remote_data(b"").send.is_empty());
    assert_eq!(f.on_remote_data(b"\r\nPassword: ").send, b"s3cret\r");
    assert!(
        f.on_remote_data(b"\r\nlogin: Password:").send.is_empty(),
        "done after password"
    );
}

#[test]
fn tel_l_02_prompt_split_across_chunks() {
    use guac_terminal::RemoteFilter;
    let mut f = filter(Some("bob"), None, None, None);
    assert!(f.on_remote_data(b"host log").send.is_empty());
    assert_eq!(f.on_remote_data(b"in: ").send, b"bob\r");
}

#[test]
fn tel_l_03_case_insensitive() {
    use guac_terminal::RemoteFilter;
    let mut f = filter(Some("bob"), None, None, None);
    assert_eq!(f.on_remote_data(b"HOST LOGIN: ").send, b"bob\r");
}

#[test]
fn tel_l_04_success_starts_terminal_and_stops_searching() {
    use guac_terminal::RemoteFilter;
    let mut f = filter(
        Some("bob"),
        Some("pw"),
        Some("Last login"),
        Some("incorrect"),
    );
    f.on_remote_data(b"login: ");
    f.on_remote_data(b"\r\nPassword: ");
    let out = f.on_remote_data(b"\r\nLast login: Mon\r\n");
    assert!(out.start);
    assert!(out.abort.is_none());
    let later = f.on_remote_data(b"login incorrect\r\n");
    assert!(later.abort.is_none(), "searches stop after success");
}

#[test]
fn tel_l_05_failure_aborts_with_unauthorized() {
    use guac_terminal::RemoteFilter;
    let mut f = filter(
        Some("bob"),
        Some("pw"),
        Some("Last login"),
        Some("incorrect"),
    );
    let out = f.on_remote_data(b"Login incorrect\r\n");
    let (status, msg) = out.abort.unwrap();
    assert_eq!(status, guac_protocol::Status::ClientUnauthorized);
    assert_eq!(msg, "Login failed");
}

#[test]
fn tel_l_06_no_credentials_means_no_sending() {
    use guac_terminal::RemoteFilter;
    let mut f = filter(None, None, Some("ok"), Some("bad"));
    assert!(f.on_remote_data(b"login: Password: ").send.is_empty());
}

#[test]
fn tel_l_07_overlong_line_is_bounded() {
    use guac_terminal::RemoteFilter;
    let mut f = filter(Some("bob"), None, None, None);
    let junk = vec![b'x'; 100_000];
    f.on_remote_data(&junk); // must not grow without bound or panic
    f.on_remote_data(b"\nlogin: ");
}

#[test]
fn tel_l_08_non_utf8_output_does_not_panic() {
    use guac_terminal::RemoteFilter;
    let mut f = filter(Some("bob"), None, None, None);
    f.on_remote_data(&[0xFF, 0xFE, b'l', b'o', b'g', b'i', b'n', b':', 0x80]);
}

// ---------- settings ----------

fn p(pairs: &[(&str, &str)]) -> Params {
    Params::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

#[test]
fn st_01_defaults() {
    let s = TelnetSettings::from_params(&p(&[("hostname", "h")])).unwrap();
    assert_eq!(
        (s.port, s.timeout_secs, s.terminal_type.as_str()),
        (23, 10, "linux")
    );
    assert!(s.username_regex.is_none() && s.password_regex.is_none());
    assert!(!s.waits_for_login());
    assert!(s.login_filter().is_none());
}

#[test]
fn st_02_hostname_required() {
    assert!(TelnetSettings::from_params(&p(&[("port", "23")])).is_none());
    assert!(TelnetSettings::from_params(&p(&[("hostname", "")])).is_none());
}

#[test]
fn st_03_bad_values_fall_back() {
    let s = TelnetSettings::from_params(&p(&[
        ("hostname", "h"),
        ("port", "99999"),
        ("timeout", "abc"),
    ]))
    .unwrap();
    assert_eq!((s.port, s.timeout_secs), (23, 10));
    let s = TelnetSettings::from_params(&p(&[("hostname", "h"), ("port", "0")])).unwrap();
    assert_eq!(s.port, 23);
}

#[test]
fn st_04_regex_only_when_credential_given() {
    let s = TelnetSettings::from_params(&p(&[("hostname", "h"), ("username", "u")])).unwrap();
    assert!(s.username_regex.is_some() && s.password_regex.is_none());
    let s = TelnetSettings::from_params(&p(&[("hostname", "h"), ("password", "x")])).unwrap();
    assert!(s.username_regex.is_none() && s.password_regex.is_some());
}

#[test]
fn st_05_success_and_failure_regex_must_pair() {
    let s = TelnetSettings::from_params(&p(&[("hostname", "h"), ("login-success-regex", "ok")]))
        .unwrap();
    assert!(!s.waits_for_login());
    assert_eq!(s.warnings.len(), 1);
    let s = TelnetSettings::from_params(&p(&[
        ("hostname", "h"),
        ("login-success-regex", "ok"),
        ("login-failure-regex", "bad"),
    ]))
    .unwrap();
    assert!(s.waits_for_login());
    assert!(s.warnings.is_empty());
}

#[test]
fn st_06_invalid_regex_warns_not_fails() {
    let s = TelnetSettings::from_params(&p(&[
        ("hostname", "h"),
        ("username", "u"),
        ("username-regex", "(["),
    ]))
    .unwrap();
    assert!(s.username_regex.is_none());
    assert_eq!(s.warnings.len(), 1);
}

#[test]
fn st_07_args_match_c_implementation_order() {
    assert_eq!(ARGS.len(), 38);
    assert_eq!(ARGS.last(), Some(&"character-encoding"));
    assert_eq!(
        &ARGS[..8],
        [
            "hostname",
            "port",
            "timeout",
            "username",
            "username-regex",
            "password",
            "password-regex",
            "font-name"
        ]
    );
    assert!(ARGS.contains(&"color-scheme") && ARGS.contains(&"login-success-regex"));
    // No duplicates.
    let mut v = ARGS.to_vec();
    v.sort();
    v.dedup();
    assert_eq!(v.len(), ARGS.len());
}

#[test]
fn st_08_args_equal_c_source_if_available() {
    // Extract GUAC_TELNET_CLIENT_ARGS straight from the reference C source and compare, in order.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/guacamole-server/src/protocols/telnet/settings.c");
    let Ok(src) = std::fs::read_to_string(&path) else {
        eprintln!("SKIP: reference source not present");
        return;
    };
    let start = src.find("GUAC_TELNET_CLIENT_ARGS[] = {").expect("array");
    let body = &src[start..];
    let body = &body[..body.find("NULL").unwrap()];
    let mut names = vec![];
    for line in body.lines().skip(1) {
        let l = line.trim().trim_end_matches(',');
        if let Some(q) = l.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            names.push(q.to_string());
        } else if l == "GUAC_TELNET_ARGV_FONT_NAME" {
            names.push("font-name".into());
        } else if l == "GUAC_TELNET_ARGV_FONT_SIZE" {
            names.push("font-size".into());
        } else if l == "GUAC_TELNET_ARGV_COLOR_SCHEME" {
            names.push("color-scheme".into());
        }
    }
    assert_eq!(
        names,
        ARGS[..ARGS.len() - 1]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
}
