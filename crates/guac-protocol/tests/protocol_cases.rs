use guac_protocol::*;
use proptest::prelude::*;

fn parse_all(data: &[u8]) -> Result<Vec<Instruction>, ParseError> {
    let mut p = Parser::new();
    p.feed(data)?;
    let mut v = vec![];
    while let Some(i) = p.next_instruction() {
        v.push(i);
    }
    Ok(v)
}

// ---- PR: parsing ----
#[test]
fn pr_01_basic() {
    let v = parse_all(b"4.size,1.0,4.1024,3.768;").unwrap();
    assert_eq!(v, vec![Instruction::new("size", ["0", "1024", "768"])]);
}

#[test]
fn pr_02_byte_at_a_time() {
    let mut p = Parser::new();
    for b in b"4.size,1.0,4.1024,3.768;" {
        p.feed(&[*b]).unwrap();
    }
    assert_eq!(
        p.next_instruction().unwrap(),
        Instruction::new("size", ["0", "1024", "768"])
    );
}

#[test]
fn pr_04_two_instructions() {
    let v = parse_all(b"3.nop;4.sync,1.5;").unwrap();
    assert_eq!(v.len(), 2);
    assert_eq!(v[1], Instruction::new("sync", ["5"]));
}

#[test]
fn pr_05_incomplete_not_emitted() {
    let mut p = Parser::new();
    p.feed(b"4.size,1.0").unwrap();
    assert!(p.next_instruction().is_none());
    assert!(p.is_mid_instruction());
}

#[test]
fn pr_06_no_args() {
    assert_eq!(
        parse_all(b"3.nop;").unwrap(),
        vec![Instruction::new("nop", Vec::<String>::new())]
    );
}

#[test]
fn pr_07_multibyte_length_is_chars() {
    // "中文" = 2 chars, 6 bytes
    let v = parse_all("4.test,2.中文;".as_bytes()).unwrap();
    assert_eq!(v[0].args, vec!["中文".to_string()]);
}

#[test]
fn pr_08_four_byte_chars() {
    let v = parse_all("4.test,2.😀😀;".as_bytes()).unwrap();
    assert_eq!(v[0].args, vec!["😀😀".to_string()]);
}

#[test]
fn pr_09_delimiters_inside_content() {
    let v = parse_all(b"4.test,5.a,b;.;").unwrap();
    assert_eq!(v[0].args, vec!["a,b;.".to_string()]);
}

#[test]
fn pr_10_empty_element() {
    let v = parse_all(b"4.test,0.,1.x;").unwrap();
    assert_eq!(v[0].args, vec!["".to_string(), "x".to_string()]);
}

#[test]
fn pr_11_non_digit_length_is_error() {
    // C parser (parser.c) also rejects any non-digit, non-period character.
    assert_eq!(parse_all(b"a.x;").unwrap_err(), ParseError::BadLength);
}

#[test]
fn pr_11b_period_without_digits_is_error() {
    assert_eq!(parse_all(b".x;").unwrap_err(), ParseError::BadLength);
}

#[test]
fn pr_12_element_too_long() {
    assert_eq!(parse_all(b"8193.").unwrap_err(), ParseError::ElementTooLong);
    // exactly at the limit is accepted
    let mut p = Parser::new();
    assert!(p.feed(b"8192.").is_ok());
}

#[test]
fn pr_14_too_many_elements() {
    let mut s = String::from("1.a");
    for _ in 0..128 {
        s.push_str(",1.b");
    }
    s.push(';');
    assert_eq!(
        parse_all(s.as_bytes()).unwrap_err(),
        ParseError::TooManyElements
    );

    // 128 elements total (opcode + 127 args) is allowed
    let mut ok = String::from("1.a");
    for _ in 0..127 {
        ok.push_str(",1.b");
    }
    ok.push(';');
    assert_eq!(parse_all(ok.as_bytes()).unwrap()[0].args.len(), MAX_ARGS);
}

#[test]
fn pr_15_bad_terminator() {
    assert_eq!(parse_all(b"3.abcX").unwrap_err(), ParseError::BadTerminator);
    // length shorter than content
    assert_eq!(parse_all(b"2.abc;").unwrap_err(), ParseError::BadTerminator);
}

#[test]
fn pr_16_poisoned_after_error() {
    let mut p = Parser::new();
    assert!(p.feed(b"x").is_err());
    assert_eq!(p.feed(b"3.nop;").unwrap_err(), ParseError::Poisoned);
    assert!(p.next_instruction().is_none());
}

#[test]
fn pr_17_unknown_opcode_ok() {
    let ok = parse_all(b"6.madeup,1.x;").unwrap();
    assert_eq!(ok[0].opcode, "madeup");
}

#[test]
fn pr_18_invalid_utf8() {
    assert_eq!(parse_all(b"1.\xff;").unwrap_err(), ParseError::InvalidUtf8);
    // truncated multibyte then terminator
    assert_eq!(parse_all(b"1.\xe4;").unwrap_err(), ParseError::InvalidUtf8);
}

#[test]
fn pr_19_failure_discards_queued() {
    let mut p = Parser::new();
    let _ = p.feed(b"3.nop;x");
    assert!(p.next_instruction().is_none());
}

// ---- EN: encoding ----
#[test]
fn en_01_basic() {
    assert_eq!(
        Instruction::new("size", ["0", "1024", "768"]).encode(),
        b"4.size,1.0,4.1024,3.768;"
    );
}

#[test]
fn en_02_char_count() {
    assert_eq!(
        Instruction::new("t", ["中文"]).encode(),
        "1.t,2.中文;".as_bytes()
    );
    assert_eq!(
        Instruction::new("t", ["😀"]).encode(),
        "1.t,1.😀;".as_bytes()
    );
}

#[test]
fn en_03_empty_arg() {
    assert_eq!(Instruction::new("t", [""]).encode(), b"1.t,0.;");
}

#[test]
fn en_04_delimiters_not_escaped() {
    assert_eq!(Instruction::new("t", ["a,b;"]).encode(), b"1.t,4.a,b;;");
}

// ---- VER / ST / mouse ----
#[test]
fn ver_roundtrip_and_negotiate() {
    for v in [
        ProtocolVersion::V1_0_0,
        ProtocolVersion::V1_1_0,
        ProtocolVersion::V1_3_0,
        ProtocolVersion::V1_5_0,
    ] {
        assert_eq!(ProtocolVersion::parse(&v.to_string()), Some(v));
    }
    assert_eq!(ProtocolVersion::parse("VERSION_9_9_9"), None);
    assert_eq!(
        ProtocolVersion::V1_5_0.negotiate(ProtocolVersion::V1_3_0),
        ProtocolVersion::V1_3_0
    );
}

#[test]
fn st_01_codes_match_c() {
    // From reference/guacamole-server/src/libguac/guacamole/protocol-types.h
    let expected: [(Status, u16); 21] = [
        (Status::Success, 0x0000),
        (Status::Unsupported, 0x0100),
        (Status::ServerError, 0x0200),
        (Status::ServerBusy, 0x0201),
        (Status::UpstreamTimeout, 0x0202),
        (Status::UpstreamError, 0x0203),
        (Status::ResourceNotFound, 0x0204),
        (Status::ResourceConflict, 0x0205),
        (Status::ResourceClosed, 0x0206),
        (Status::UpstreamNotFound, 0x0207),
        (Status::UpstreamUnavailable, 0x0208),
        (Status::SessionConflict, 0x0209),
        (Status::SessionTimeout, 0x020A),
        (Status::SessionClosed, 0x020B),
        (Status::ClientBadRequest, 0x0300),
        (Status::ClientUnauthorized, 0x0301),
        (Status::ClientForbidden, 0x0303),
        (Status::ClientTimeout, 0x0308),
        (Status::ClientOverrun, 0x030D),
        (Status::ClientBadType, 0x030F),
        (Status::ClientTooMany, 0x031D),
    ];
    for (s, c) in expected {
        assert_eq!(s.code(), c);
        assert_eq!(Status::from_code(c), Some(s));
    }
    assert_eq!(Status::from_code(0x9999), None);
}

#[test]
fn in_mouse_mask_bits() {
    let m = MouseMask(MouseMask::LEFT | MouseMask::SCROLL_UP);
    assert!(m.has(MouseMask::LEFT));
    assert!(m.has(MouseMask::SCROLL_UP));
    assert!(!m.has(MouseMask::RIGHT));
}

// ---- PR-PROP ----
fn arb_instruction() -> impl Strategy<Value = Instruction> {
    let elem = "\\PC{0,40}"; // any printable unicode (no control chars)
    (elem, proptest::collection::vec(elem, 0..10)).prop_map(|(o, a)| Instruction::new(o, a))
}

proptest! {
    #[test]
    fn roundtrip(i in arb_instruction()) {
        let v = parse_all(&i.encode()).unwrap();
        prop_assert_eq!(v, vec![i]);
    }

    #[test]
    fn any_chunking_is_equivalent(
        insts in proptest::collection::vec(arb_instruction(), 1..5),
        cuts in proptest::collection::vec(1usize..7, 1..40),
    ) {
        let mut wire = vec![];
        for i in &insts { i.encode_into(&mut wire); }
        let mut p = Parser::new();
        let mut pos = 0; let mut k = 0;
        while pos < wire.len() {
            let n = cuts[k % cuts.len()].min(wire.len() - pos);
            p.feed(&wire[pos..pos + n]).unwrap();
            pos += n; k += 1;
        }
        let mut got = vec![];
        while let Some(i) = p.next_instruction() { got.push(i); }
        prop_assert_eq!(got, insts);
    }

    #[test]
    fn arbitrary_bytes_never_panic(data in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut p = Parser::new();
        let _ = p.feed(&data);
        while p.next_instruction().is_some() {}
    }
}
