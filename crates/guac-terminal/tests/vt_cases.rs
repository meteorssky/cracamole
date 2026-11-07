use guac_terminal::{FontSet, Terminal, TerminalOptions};

/// 80x24 terminal (cell size from the bundled font).
fn term() -> Terminal {
    term_sized(80, 24)
}

fn term_sized(cols: u32, rows: u32) -> Terminal {
    let fonts = FontSet::bundled();
    let px = 12.0 * 96.0 / 72.0;
    let (cw, ch, _) = fonts.metrics(px);
    Terminal::new(TerminalOptions::default(), fonts, cols * cw, rows * ch)
}

fn t(input: &str) -> Terminal {
    let mut t = term();
    t.feed(input.as_bytes());
    t
}

#[test]
fn vt_00_dimensions() {
    assert_eq!(term().size_cells(), (80, 24));
}

#[test]
fn vt_cur_text_and_newlines() {
    let t = t("hello\r\nworld");
    assert_eq!(t.row_text(0), "hello");
    assert_eq!(t.row_text(1), "world");
    assert_eq!(t.cursor(), (1, 5));
}

#[test]
fn vt_cur_cup_and_relative_moves() {
    let t = t("\x1b[5;10Hx\x1b[2Ay\x1b[3Bz\x1b[4Dw");
    assert_eq!(t.row_text(4), "         x");
    assert_eq!(t.row_text(2).trim(), "y");
    assert_eq!(t.row_text(5).trim(), "w  z");
}

#[test]
fn vt_cur_save_restore() {
    let t = t("\x1b[3;3H\x1b7\x1b[10;10H\x1b8X");
    assert_eq!(t.row_text(2), "  X");
}

#[test]
fn vt_cur_clamped_at_edges() {
    let t = t("\x1b[999;999Hx\x1b[999A\x1b[999D");
    assert_eq!(t.cursor(), (0, 0));
    assert!(t.row_text(23).ends_with('x'));
}

#[test]
fn vt_era_ed_el() {
    let t = t("aaaa\r\nbbbb\r\ncccc\x1b[2;3H\x1b[K");
    assert_eq!(t.row_text(1), "bb");
    let t2 = t_("aaaa\r\nbbbb\r\ncccc\x1b[2;3H\x1b[J");
    assert_eq!(t2.row_text(0), "aaaa");
    assert_eq!(t2.row_text(1), "bb");
    assert_eq!(t2.row_text(2), "");
    let t3 = t_("aaaa\r\nbbbb\x1b[2J");
    assert!(t3.screen_text().iter().all(|r| r.is_empty()));
    let t4 = t_("abcdef\x1b[1;4H\x1b[1K");
    assert_eq!(t4.row_text(0).trim_start(), "ef");
}

fn t_(s: &str) -> Terminal {
    t(s)
}

#[test]
fn vt_era_ich_dch_ech_il_dl() {
    assert_eq!(t("abcdef\x1b[1;3H\x1b[2P").row_text(0), "abef");
    assert_eq!(t("abcdef\x1b[1;3H\x1b[2@").row_text(0), "ab  cdef");
    assert_eq!(t("abcdef\x1b[1;3H\x1b[2X").row_text(0), "ab  ef");
    let a = t("one\r\ntwo\r\nthree\x1b[2;1H\x1b[L");
    assert_eq!(a.screen_text()[..4], ["one", "", "two", "three"]);
    let b = t("one\r\ntwo\r\nthree\x1b[2;1H\x1b[M");
    assert_eq!(b.screen_text()[..3], ["one", "three", ""]);
}

#[test]
fn vt_sgr_does_not_corrupt_text() {
    let t = t("\x1b[1;31mred\x1b[0m \x1b[38;5;200mx\x1b[38;2;1;2;3my\x1b[48;5;17mz\x1b[0m");
    assert_eq!(t.row_text(0), "red xyz");
}

#[test]
fn vt_scr_region_and_scroll() {
    // Region rows 2-4; fill, then newline at the bottom of the region scrolls only the region.
    let mut s = String::from("\x1b[1;1Htop\x1b[2;4r");
    s.push_str("\x1b[2;1Ha\r\nb\r\nc\r\nd");
    let t = t(&s);
    assert_eq!(t.row_text(0), "top");
    assert_eq!(t.row_text(1), "b");
    assert_eq!(t.row_text(2), "c");
    assert_eq!(t.row_text(3), "d");
}

#[test]
fn vt_scr_scrollback_accumulates() {
    let mut t = term_sized(20, 5);
    let mut s = String::new();
    for i in 0..30 {
        s.push_str(&format!("line{i}\r\n"));
    }
    t.feed(s.as_bytes());
    assert_eq!(t.row_text(0), "line26");
    // scroll back into history with Shift+PageUp
    t.key(0xFFE1, true);
    t.key(0xFF55, true);
    t.key(0xFF55, false);
    t.key(0xFFE1, false);
    assert_eq!(t.row_text(0), "line21");
    // typing returns to live view
    t.key('x' as u32, true);
    assert_eq!(t.row_text(0), "line26");
}

#[test]
fn vt_scr_scrollback_limit() {
    let fonts = FontSet::bundled();
    let (cw, ch, _) = fonts.metrics(16.0);
    let opts = TerminalOptions {
        scrollback: 10,
        ..Default::default()
    };
    let mut t = Terminal::new(opts, fonts, 20 * cw, 5 * ch);
    let mut s = String::new();
    for i in 0..100 {
        s.push_str(&format!("L{i}\r\n"));
    }
    t.feed(s.as_bytes());
    t.key(0xFFE1, true);
    t.key(0xFF50, true); // Shift+Home: all the way back
    assert_eq!(
        t.row_text(0),
        "L86",
        "5 rows + 10 scrollback lines retained: oldest retained is L86"
    );
}

#[test]
fn vt_alt_screen_enter_exit() {
    let mut t = term();
    t.feed(b"main screen");
    t.feed(b"\x1b[?1049h\x1b[2J\x1b[Halt content");
    assert_eq!(t.row_text(0), "alt content");
    t.feed(b"\x1b[?1049l");
    assert_eq!(t.row_text(0), "main screen");
}

#[test]
fn vt_mode_autowrap() {
    let mut t = term_sized(10, 5);
    t.feed(b"0123456789AB");
    assert_eq!(t.row_text(0), "0123456789");
    assert_eq!(t.row_text(1), "AB");
    let mut n = term_sized(10, 5);
    n.feed(b"\x1b[?7l0123456789AB");
    assert_eq!(n.row_text(0), "012345678B");
}

#[test]
fn vt_mode_cursor_visibility_does_not_affect_text() {
    assert_eq!(t("\x1b[?25lhi\x1b[?25h").row_text(0), "hi");
}

#[test]
fn vt_tab_stops() {
    let t = t("a\tb\tc");
    assert_eq!(t.row_text(0), "a       b       c");
    let t2 = t_("\x1b[3g\x1b[1;5H\x1bH\x1b[1;1Ha\tb");
    assert_eq!(t2.row_text(0), "a   b");
}

#[test]
fn vt_wide_chars_occupy_two_cells() {
    let t = t("中文ab");
    assert_eq!(t.row_text(0), "中文ab");
    assert_eq!(t.cursor(), (0, 6), "two wide chars = 4 cells + 2");
}

#[test]
fn vt_wide_char_wraps_at_line_end() {
    let mut t = term_sized(5, 3);
    t.feed("abcd中".as_bytes());
    assert_eq!(t.row_text(0), "abcd");
    assert_eq!(t.row_text(1), "中");
}

#[test]
fn vt_combining_character_attaches() {
    let t = t("e\u{0301}x");
    assert_eq!(t.cursor(), (0, 2), "combining mark occupies no cell");
}

#[test]
fn vt_charset_dec_special_graphics() {
    // ESC ( 0 selects line-drawing; 'q' -> horizontal line, 'x' -> vertical line
    let t = t("\x1b(0qqx\x1b(Bq");
    assert_eq!(t.row_text(0), "──│q");
}

#[test]
fn vt_resp_device_status_report() {
    let mut t = term();
    t.feed(b"\x1b[5;7H\x1b[6n");
    assert_eq!(t.take_output(), b"\x1b[5;7R");
}

#[test]
fn vt_resp_device_attributes() {
    let mut t = term();
    t.feed(b"\x1b[c");
    let out = t.take_output();
    assert!(
        out.starts_with(b"\x1b[?"),
        "{:?}",
        String::from_utf8_lossy(&out)
    );
}

#[test]
fn vt_osc_title_not_printed() {
    let mut t = term();
    t.feed(b"\x1b]0;my title\x07visible");
    assert_eq!(t.row_text(0), "visible");
    assert_eq!(t.title().as_deref(), Some("my title"));
    let mut t2 = term();
    t2.feed(b"\x1b]9999;unknown\x07ok");
    assert_eq!(t2.row_text(0), "ok");
}

#[test]
fn vt_robust_truncated_and_garbage_sequences() {
    let mut t = term();
    t.feed(b"\x1b[");
    t.feed(b"\x1b[1;2;3;4;5;6;7;8;9;10;11;12;13;14;15;16;17;18;19;20;21;22;23;24;25;26;27;28;29;30;31;32;33m");
    t.feed(b"\x1b[999999999999999999999999H");
    t.feed(&[0xFF, 0xFE, 0x1b, 0x1b, 0x1b, b'[', 0x18, b'o', b'k']);
    t.feed(&vec![b'x'; 100_000]);
    assert!(!t.row_text(23).is_empty() || !t.row_text(22).is_empty());
}

#[test]
fn vt_bell_and_backspace() {
    let t = t("abc\x07\x08\x08X");
    assert_eq!(t.row_text(0), "aXc");
}

#[test]
fn vt_reverse_index_and_nel() {
    let t = t("a\r\nb\x1b[1;1H\x1bM");
    assert_eq!(t.screen_text()[..3], ["", "a", "b"]);
    assert_eq!(t_("ab\x1bEcd").screen_text()[..2], ["ab", "cd"]);
}

#[test]
fn vt_app_keypad_and_cursor_modes_visible_to_keyboard() {
    let mut t = term();
    t.feed(b"\x1b[?1h"); // DECCKM
    t.key(0xFF52, true);
    assert_eq!(t.take_output(), b"\x1bOA");
    t.feed(b"\x1b[?1l");
    t.key(0xFF52, true);
    assert_eq!(t.take_output(), b"\x1b[A");
}

#[test]
fn vt_resize_updates_cells_and_reports() {
    let mut t = term();
    t.feed(b"content");
    let (cw, ch) = t.cell_size();
    assert_eq!(t.resize_px(100 * cw, 30 * ch), Some((100, 30)));
    assert_eq!(t.row_text(0), "content");
    assert_eq!(t.resize_px(100 * cw, 30 * ch), None, "same size: no change");
    assert_eq!(
        t.resize_px(1, 1),
        Some((2, 1)),
        "extreme values clamp to the 2x1 minimum grid"
    );
    t.feed(b"more output after tiny resize");
}

// RS-04: extreme sizes must never panic
#[test]
fn vt_resize_extremes_no_panic() {
    let mut t = term();
    for (w, h) in [(0, 0), (1, 1), (5000, 5000), (7, 3000), (3000, 7)] {
        t.resize_px(w, h);
        t.feed(b"x\r\ny\x1b[2J");
    }
}

#[test]
fn vt_paste_normalizes_newlines_and_honours_bracketed_mode() {
    let mut t = term();
    t.paste("a\nb\r\nc");
    assert_eq!(t.take_output(), b"a\rb\rc");
    t.feed(b"\x1b[?2004h");
    t.paste("x");
    assert_eq!(t.take_output(), b"\x1b[200~x\x1b[201~");
}

#[test]
fn vt_paste_cannot_inject_terminal_control_sequences() {
    let mut t = term();
    t.feed(b"\x1b[?2004h");
    t.paste("text\x1b[201~printf marker\n\0\x03\x7f\u{009b}中\t");
    assert_eq!(
        t.take_output(),
        "\x1b[200~text[201~printf marker\r中\t\x1b[201~".as_bytes()
    );
    t.feed(b"\x1b[?2004l");
    t.paste("\x1b[A\x03hello\n");
    assert_eq!(t.take_output(), b"[Ahello\r");
}

#[test]
fn vt_disable_paste() {
    let fonts = FontSet::bundled();
    let opts = TerminalOptions {
        disable_paste: true,
        ..Default::default()
    };
    let mut t = Terminal::new(opts, fonts, 800, 400);
    t.paste("secret");
    assert!(t.take_output().is_empty());
}

#[test]
fn vt_output_flood_is_bounded() {
    let mut t = term();
    let chunk = vec![b'a'; 1 << 20];
    for _ in 0..10 {
        t.feed(&chunk);
    }
    assert!(t.take_output().is_empty());
}
