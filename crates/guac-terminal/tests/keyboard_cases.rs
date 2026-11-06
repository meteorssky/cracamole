use guac_terminal::{KeyModes, Keyboard};

fn send(kb: &mut Keyboard, modes: &KeyModes, keysym: u32) -> Vec<u8> {
    let down = kb.process(keysym, true, modes).send;
    kb.process(keysym, false, modes);
    down
}

fn with_mods(mods: &[u32], keysym: u32, modes: &KeyModes) -> Vec<u8> {
    let mut kb = Keyboard::default();
    for &m in mods {
        kb.process(m, true, modes);
    }
    kb.process(keysym, true, modes).send
}

const SHIFT: u32 = 0xFFE1;
const CTRL: u32 = 0xFFE3;
const ALT: u32 = 0xFFE9;
const SUPER: u32 = 0xFFEB;

#[test]
fn kb_01_printable_ascii() {
    let (mut kb, m) = (Keyboard::default(), KeyModes::default());
    assert_eq!(send(&mut kb, &m, 'a' as u32), b"a");
    assert_eq!(send(&mut kb, &m, ' ' as u32), b" ");
    assert_eq!(send(&mut kb, &m, '~' as u32), b"~");
}

#[test]
fn kb_02_unicode_keysyms_utf8() {
    let (mut kb, m) = (Keyboard::default(), KeyModes::default());
    assert_eq!(send(&mut kb, &m, 0x0100_4E2D), "中".as_bytes()); // BMP
    assert_eq!(send(&mut kb, &m, 0xE9), "é".as_bytes()); // Latin-1
                                                         // Deviation (DEVIATIONS): C ignored non-BMP; we encode the full code point.
    assert_eq!(send(&mut kb, &m, 0x0101_F600), "😀".as_bytes());
}

#[test]
fn kb_03_enter_lf_tab_escape() {
    let (mut kb, m) = (Keyboard::default(), KeyModes::default());
    assert_eq!(send(&mut kb, &m, 0xFF0D), b"\r");
    assert_eq!(send(&mut kb, &m, 0xFF8D), b"\r");
    assert_eq!(send(&mut kb, &m, 0xFF0A), b"\n");
    assert_eq!(send(&mut kb, &m, 0xFF09), b"\t");
    assert_eq!(send(&mut kb, &m, 0xFF1B), b"\x1b");
}

#[test]
fn kb_04_backspace_default_and_configured() {
    let mut kb = Keyboard::default();
    assert_eq!(send(&mut kb, &KeyModes::default(), 0xFF08), vec![127]);
    let m = KeyModes {
        backspace: 8,
        ..Default::default()
    };
    assert_eq!(send(&mut kb, &m, 0xFF08), vec![8]);
}

#[test]
fn kb_05_shift_tab_is_backtab() {
    let m = KeyModes::default();
    assert_eq!(with_mods(&[SHIFT], 0xFF09, &m), b"\x1b[Z");
}

#[test]
fn kb_06_arrows_normal_and_application() {
    let mut kb = Keyboard::default();
    let normal = KeyModes::default();
    let app = KeyModes {
        app_cursor: true,
        ..Default::default()
    };
    for (k, n, a) in [
        (0xFF52, &b"\x1b[A"[..], &b"\x1bOA"[..]),
        (0xFF54, b"\x1b[B", b"\x1bOB"),
        (0xFF53, b"\x1b[C", b"\x1bOC"),
        (0xFF51, b"\x1b[D", b"\x1bOD"),
    ] {
        assert_eq!(send(&mut kb, &normal, k), n);
        assert_eq!(send(&mut kb, &app, k), a);
    }
}

#[test]
fn kb_07_ctrl_letters() {
    let m = KeyModes::default();
    assert_eq!(with_mods(&[CTRL], 'a' as u32, &m), vec![1]);
    assert_eq!(with_mods(&[CTRL], 'c' as u32, &m), vec![3]);
    assert_eq!(with_mods(&[CTRL], 'z' as u32, &m), vec![26]);
    assert_eq!(with_mods(&[CTRL], '[' as u32, &m), vec![0x1b]);
    assert_eq!(with_mods(&[CTRL], '@' as u32, &m), vec![0]);
    assert_eq!(with_mods(&[CTRL], '2' as u32, &m), vec![0]);
    assert_eq!(with_mods(&[CTRL], '3' as u32, &m), vec![0x1b]);
    assert_eq!(with_mods(&[CTRL], '7' as u32, &m), vec![0x1f]);
    assert_eq!(with_mods(&[CTRL], '?' as u32, &m), vec![0x7f]);
    assert_eq!(with_mods(&[CTRL], 0xFF08, &m), vec![23]); // Ctrl+Backspace = Ctrl+W
}

#[test]
fn kb_07b_ctrl_shift_c_swallowed_and_paste_shortcut() {
    let m = KeyModes::default();
    assert!(with_mods(&[CTRL, SHIFT], 'C' as u32, &m).is_empty());
    let mut kb = Keyboard::default();
    kb.process(CTRL, true, &m);
    kb.process(SHIFT, true, &m);
    assert!(kb.process('V' as u32, true, &m).paste);
}

#[test]
fn kb_08_alt_prefixes_escape() {
    let m = KeyModes::default();
    assert_eq!(with_mods(&[ALT], 'x' as u32, &m), b"\x1bx");
    assert_eq!(with_mods(&[ALT], 0xFF0D, &m), b"\x1b\r");
}

#[test]
fn kb_09_modifier_alone_sends_nothing() {
    let m = KeyModes::default();
    let mut kb = Keyboard::default();
    for k in [
        SHIFT, CTRL, ALT, SUPER, 0xFFE5, /*caps*/
        0xFE03, /*altgr*/
    ] {
        assert!(kb.process(k, true, &m).send.is_empty(), "{k:#x}");
        kb.process(k, false, &m);
    }
}

#[test]
fn kb_10_editing_keys() {
    let (mut kb, m) = (Keyboard::default(), KeyModes::default());
    assert_eq!(send(&mut kb, &m, 0xFF50), b"\x1b[1~"); // Home: always, even in xterm mode (C quirk)
    assert_eq!(send(&mut kb, &m, 0xFF57), b"\x1b[4~"); // End, linux console keys (default)
    let xterm = KeyModes {
        linux_console_keys: false,
        ..Default::default()
    };
    assert_eq!(send(&mut kb, &xterm, 0xFF50), b"\x1b[1~");
    assert_eq!(send(&mut kb, &xterm, 0xFF57), b"\x1b[F");
    assert_eq!(send(&mut kb, &m, 0xFF63), b"\x1b[2~"); // Insert
    assert_eq!(send(&mut kb, &m, 0xFFFF), b"\x1b[3~"); // Delete
    assert_eq!(send(&mut kb, &m, 0xFF55), b"\x1b[5~");
    assert_eq!(send(&mut kb, &m, 0xFF56), b"\x1b[6~");
}

#[test]
fn kb_11_function_keys_default_and_vt100() {
    let (mut kb, m) = (Keyboard::default(), KeyModes::default());
    let vt = KeyModes {
        vt100_func_keys: true,
        ..Default::default()
    };
    let table: [(u32, &[u8], &[u8]); 5] = [
        (0xFFBE, b"\x1b[[A", b"\x1bOP"),
        (0xFFBF, b"\x1b[[B", b"\x1bOQ"),
        (0xFFC0, b"\x1b[[C", b"\x1bOR"),
        (0xFFC1, b"\x1b[[D", b"\x1bOS"),
        (0xFFC2, b"\x1b[[E", b"\x1b[15~"),
    ];
    for (k, d, v) in table {
        assert_eq!(send(&mut kb, &m, k), d, "default {k:#x}");
        assert_eq!(send(&mut kb, &vt, k), v, "vt100 {k:#x}");
    }
    let rest: [(u32, &[u8]); 7] = [
        (0xFFC3, b"\x1b[17~"),
        (0xFFC4, b"\x1b[18~"),
        (0xFFC5, b"\x1b[19~"),
        (0xFFC6, b"\x1b[20~"),
        (0xFFC7, b"\x1b[21~"),
        (0xFFC8, b"\x1b[22~"), // C quirk: F11
        (0xFFC9, b"\x1b[23~"), // C quirk: F12
    ];
    for (k, d) in rest {
        assert_eq!(send(&mut kb, &m, k), d, "{k:#x}");
    }
}

#[test]
fn kb_12_modified_arrows_and_ctrl_word_motion() {
    let m = KeyModes::default();
    assert_eq!(with_mods(&[CTRL], 0xFF51, &m), b"\x1b[1;5D");
    assert_eq!(with_mods(&[CTRL], 0xFF53, &m), b"\x1b[1;5C");
    assert_eq!(with_mods(&[CTRL], 0xFF52, &m), b"\x1b[1;5A");
    assert_eq!(with_mods(&[ALT], 0xFF52, &m), b"\x1b\x1b[1;3A"); // alt adds ESC prefix, then modified arrow
    assert_eq!(with_mods(&[CTRL], 0xFFFF, &m), b"\x1bD");
}

#[test]
fn kb_16_modified_function_and_editing() {
    let m = KeyModes::default();
    assert_eq!(with_mods(&[CTRL], 0xFFBE, &m), b"\x1b[1;5P"); // Ctrl+F1
    assert_eq!(with_mods(&[CTRL], 0xFFC3, &m), b"\x1b[17;5~"); // Ctrl+F6
    assert_eq!(with_mods(&[CTRL], 0xFFC8, &m), b"\x1b[23;5~"); // Ctrl+F11 (C quirk: differs from unmodified)
    assert_eq!(with_mods(&[CTRL], 0xFF55, &m), b"\x1b[5;5~");
    assert_eq!(with_mods(&[CTRL], 0xFF50, &m), b"\x1b[1;5~"); // linux console
}

#[test]
fn kb_17_altgr_not_a_modifier() {
    let m = KeyModes::default();
    let mut kb = Keyboard::default();
    kb.process(0xFE03, true, &m);
    // AltGr+e produces é via the client; terminal sees only the printable keysym
    assert_eq!(kb.process(0xE9, true, &m).send, "é".as_bytes());
}

#[test]
fn kb_18_keypad() {
    let (mut kb, m) = (Keyboard::default(), KeyModes::default());
    assert_eq!(send(&mut kb, &m, 0xFFB5), b"5"); // KP_5
    let vt = KeyModes {
        vt100_func_keys: true,
        app_keypad: true,
        ..Default::default()
    };
    assert_eq!(send(&mut kb, &vt, 0xFFB5), b"\x1bOu");
    assert_eq!(send(&mut kb, &vt, 0xFFAD), b"\x1bOm"); // KP_Subtract
}

#[test]
fn kb_scroll_shortcuts_with_shift() {
    let m = KeyModes {
        rows: 30,
        max_scrollback: 500,
        ..Default::default()
    };
    let mut kb = Keyboard::default();
    kb.process(SHIFT, true, &m);
    assert_eq!(kb.process(0xFF55, true, &m).scroll, 30);
    assert_eq!(kb.process(0xFF56, true, &m).scroll, -30);
    assert_eq!(kb.process(0xFF52, true, &m).scroll, 1);
    assert_eq!(kb.process(0xFF54, true, &m).scroll, -1);
    assert_eq!(kb.process(0xFF50, true, &m).scroll, 500);
    assert_eq!(kb.process(0xFF57, true, &m).scroll, -500);
    assert!(kb.process(0xFF55, true, &m).send.is_empty());
}

#[test]
fn kb_typing_resets_scroll_but_modifiers_do_not() {
    let m = KeyModes::default();
    let mut kb = Keyboard::default();
    assert!(kb.process('a' as u32, true, &m).reset_scroll);
    assert!(!kb.process(0xFFE5, true, &m).reset_scroll); // caps lock
    assert!(!kb.process(SHIFT, true, &m).reset_scroll);
}

#[test]
fn kb_15_unknown_keysym_ignored_and_release_silent() {
    let (mut kb, m) = (Keyboard::default(), KeyModes::default());
    assert!(send(&mut kb, &m, 0x00FF_0000).is_empty());
    assert!(kb.process('a' as u32, false, &m).send.is_empty());
}
