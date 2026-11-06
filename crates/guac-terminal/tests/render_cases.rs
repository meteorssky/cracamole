use guac_display::Display;
use guac_terminal::{FontSet, Terminal, TerminalOptions};
use guac_testkit::DisplayReplay;

struct Rig {
    term: Terminal,
    display: Display,
    replay: DisplayReplay,
    cw: u32,
    ch: u32,
}

fn rig_with(cols: u32, rows: u32, fonts: FontSet, opts: TerminalOptions) -> Rig {
    let px = opts.font_size_pt * opts.dpi as f32 / 72.0;
    let (cw, ch, _) = fonts.metrics(px);
    let term = Terminal::new(opts, fonts, cols * cw, rows * ch);
    let display = Display::new(cols * cw, rows * ch);
    Rig {
        term,
        display,
        replay: DisplayReplay::new(),
        cw,
        ch,
    }
}

fn rig(cols: u32, rows: u32) -> Rig {
    rig_with(cols, rows, FontSet::bundled(), TerminalOptions::default())
}

impl Rig {
    /// Render and flush; returns the instructions sent.
    fn draw(&mut self) -> Vec<guac_protocol::Instruction> {
        self.term.render(&mut self.display);
        let out = self.display.flush();
        for i in &out {
            self.replay.apply(i);
        }
        assert!(
            self.replay.unsupported.is_empty(),
            "{:?}",
            self.replay.unsupported
        );
        out
    }

    /// Number of pixels in cell (col,row) that differ from `bg`.
    fn ink(&self, col: u32, row: u32, bg: [u8; 4]) -> usize {
        let mut n = 0;
        for y in 0..self.ch {
            for x in 0..self.cw {
                if self.replay.pixel(
                    0,
                    (col * self.cw + x) as usize,
                    (row * self.ch + y) as usize,
                ) != Some(bg)
                {
                    n += 1;
                }
            }
        }
        n
    }
}

const BG: [u8; 4] = [0, 0, 0, 255];

#[test]
fn rn_01_first_render_sets_size_and_background() {
    let mut r = rig(20, 5);
    r.term.feed(b"[?25l");
    let out = r.draw();
    assert_eq!(out[0].opcode, "size");
    assert_eq!(
        out[0].args,
        vec![
            "0".to_string(),
            (20 * r.cw).to_string(),
            (5 * r.ch).to_string()
        ]
    );
    assert_eq!(r.replay.pixel(0, 3, 3), Some(BG));
    assert_eq!(r.ink(10, 2, BG), 0);
}

#[test]
fn rn_02_text_draws_ink_only_where_written() {
    let mut r = rig(20, 5);
    r.term.feed(b"Hi");
    r.draw();
    assert!(r.ink(0, 0, BG) > 10, "H has ink");
    assert!(r.ink(1, 0, BG) > 5, "i has ink");
    assert_eq!(r.ink(5, 0, BG), 0, "unwritten cell is background");
    assert_eq!(r.ink(0, 1, BG), 0, "next row untouched");
}

#[test]
fn rn_03_distinct_glyphs_produce_distinct_pixels() {
    let mut r = rig(20, 5);
    r.term.feed(b"AB");
    r.draw();
    let cell = |c: u32| -> Vec<Option<[u8; 4]>> {
        (0..r.ch)
            .flat_map(|y| (0..r.cw).map(move |x| (x, y)))
            .map(|(x, y)| r.replay.pixel(0, (c * r.cw + x) as usize, y as usize))
            .collect()
    };
    assert_ne!(cell(0), cell(1));
}

#[test]
fn rn_04_colors_and_attributes() {
    let mut r = rig(20, 5);
    r.term.feed(b"\x1b[?25l\x1b[41m   \x1b[0m"); // red background (palette 1), cursor hidden
    r.draw();
    assert_eq!(r.replay.pixel(0, 2, 2), Some([0x99, 0x3E, 0x3E, 255]));
    assert_eq!(r.replay.pixel(0, (3 * r.cw) as usize + 2, 2), Some(BG));

    let mut r = rig(20, 5);
    r.term.feed(b"\x1b[7m \x1b[0m"); // inverse space = filled with foreground colour
    r.draw();
    assert_eq!(r.replay.pixel(0, 2, 2), Some([0x99, 0x99, 0x99, 255]));

    let mut r = rig(20, 5);
    r.term
        .feed(b"\x1b[38;2;10;20;30m\x1b[48;2;200;100;50m \x1b[0m"); // truecolor
    r.draw();
    assert_eq!(r.replay.pixel(0, 2, 2), Some([200, 100, 50, 255]));

    let mut r = rig(20, 5);
    r.term.feed(b"\x1b[48;5;196m "); // 256-colour index 196 = (255,0,0)
    r.draw();
    assert_eq!(r.replay.pixel(0, 2, 2), Some([255, 0, 0, 255]));
}

#[test]
fn rn_05_underline_and_bold_change_pixels() {
    let mut plain = rig(10, 3);
    plain.term.feed(b"x");
    plain.draw();
    let mut ul = rig(10, 3);
    ul.term.feed(b"\x1b[4mx");
    ul.draw();
    assert!(ul.ink(0, 0, BG) > plain.ink(0, 0, BG));
    let mut bold = rig(10, 3);
    bold.term.feed(b"\x1b[1mx");
    bold.draw();
    assert_ne!(bold.ink(0, 0, BG), plain.ink(0, 0, BG));
}

#[test]
fn rn_06_cursor_is_drawn_and_moves() {
    let mut r = rig(10, 3);
    r.draw();
    // cursor at (0,0): block of foreground colour
    assert_eq!(r.replay.pixel(0, 2, 2), Some([0x99, 0x99, 0x99, 255]));
    r.term.feed(b"\r\n");
    r.draw();
    assert_eq!(r.replay.pixel(0, 2, 2), Some(BG), "old cursor cell cleared");
    assert_eq!(
        r.replay.pixel(0, 2, r.ch as usize + 2),
        Some([0x99, 0x99, 0x99, 255])
    );
    r.term.feed(b"\x1b[?25l");
    r.draw();
    assert_eq!(
        r.replay.pixel(0, 2, r.ch as usize + 2),
        Some(BG),
        "hidden cursor"
    );
}

#[test]
fn rn_07_incremental_render_touches_only_changed_rows() {
    let mut r = rig(40, 10);
    r.term.feed(b"\x1b[?25l");
    r.draw();
    r.term.feed(b"\x1b[5;1Hchanged");
    let out = r.draw();
    let imgs: Vec<_> = out.iter().filter(|i| i.opcode == "img").collect();
    assert_eq!(imgs.len(), 1, "one band");
    let y: u32 = imgs[0].args[5].parse().unwrap();
    assert_eq!(y, 4 * r.ch, "band starts at row 5");
    assert!(out.iter().all(|i| i.opcode != "size"));
}

#[test]
fn rn_08_idle_render_is_empty() {
    let mut r = rig(20, 5);
    r.term.feed(b"\x1b[?25l");
    r.draw();
    let out = r.draw();
    assert!(
        out.is_empty(),
        "nothing changed -> no instructions: {out:?}"
    );
}

#[test]
fn rn_09_deterministic() {
    let render = || {
        let mut r = rig(30, 6);
        r.term.feed(b"Hello, \x1b[1;32mworld\x1b[0m!\r\nline two");
        r.draw();
        (0..6 * r.ch)
            .flat_map(|y| (0..30 * r.cw).map(move |x| (x, y)))
            .map(|(x, y)| r.replay.pixel(0, x as usize, y as usize))
            .collect::<Vec<_>>()
    };
    assert_eq!(render(), render());
}

#[test]
fn rn_10_resize_resizes_display_and_redraws() {
    let mut r = rig(20, 5);
    r.term.feed(b"abc");
    r.draw();
    let (nw, nh) = (30 * r.cw, 8 * r.ch);
    r.term.resize_px(nw, nh);
    let out = r.draw();
    assert_eq!(out[0].opcode, "size");
    assert_eq!(
        out[0].args,
        vec!["0".to_string(), nw.to_string(), nh.to_string()]
    );
    assert!(r.ink(0, 0, BG) > 0, "text redrawn after resize");
}

#[test]
fn rn_11_scrollback_view_renders_history() {
    let mut r = rig(10, 3);
    let mut s = String::new();
    for i in 0..10 {
        s.push_str(&format!("{i}\r\n"));
    }
    r.term.feed(s.as_bytes());
    r.draw();
    let frame = |r: &Rig| {
        (0..3 * r.ch)
            .flat_map(|y| (0..10 * r.cw).map(move |x| (x, y)))
            .map(|(x, y)| r.replay.pixel(0, x as usize, y as usize))
            .collect::<Vec<_>>()
    };
    let live = frame(&r);
    r.term.mouse(0, 0, 8); // wheel up
    r.term.mouse(0, 0, 0);
    r.draw();
    let after = frame(&r);
    assert_ne!(live, after, "scrolling into history changes what is drawn");
    assert_ne!(r.term.row_text(0), "");
}

// ---- SEL ----

fn drag(r: &mut Rig, from: (u32, u32), to: (u32, u32)) {
    // start in the left half of the first cell, end in the right half of the last (inclusive drag)
    let x0 = (from.0 * r.cw + r.cw / 4) as i32;
    let y0 = (from.1 * r.ch + r.ch / 2) as i32;
    let x1 = (to.0 * r.cw + r.cw * 3 / 4) as i32;
    let y1 = (to.1 * r.ch + r.ch / 2) as i32;
    r.term.mouse(x0, y0, 1);
    r.term.mouse(x1, y1, 1);
    r.term.mouse(x1, y1, 0);
}

#[test]
fn sel_01_single_line_drag_copies_text() {
    let mut r = rig(30, 5);
    r.term.feed(b"hello world");
    drag(&mut r, (0, 0), (4, 0));
    assert_eq!(r.term.take_clipboard().as_deref(), Some("hello"));
    assert_eq!(r.term.take_clipboard(), None, "consumed");
}

#[test]
fn sel_02_multi_line_selection() {
    let mut r = rig(30, 5);
    r.term.feed(b"first line\r\nsecond line");
    drag(&mut r, (6, 0), (5, 1));
    let got = r.term.take_clipboard().unwrap();
    assert!(got.starts_with("line"), "{got:?}");
    assert!(got.contains("second"), "{got:?}");
}

#[test]
fn sel_03_wide_chars_not_split() {
    let mut r = rig(30, 5);
    r.term.feed("中文字".as_bytes());
    drag(&mut r, (0, 0), (3, 0));
    let got = r.term.take_clipboard().unwrap();
    assert!(got.starts_with("中文"), "{got:?}");
    assert!(!got.contains('\u{FFFD}'));
}

#[test]
fn sel_04_selection_is_highlighted() {
    let mut r = rig(30, 5);
    r.term.feed(b"abcdef\x1b[?25l");
    r.draw();
    assert_eq!(r.replay.pixel(0, 1, 1), Some(BG));
    let p = |c: u32| ((c * r.cw + 1) as i32, (r.ch / 2) as i32);
    r.term.mouse(p(0).0, p(0).1, 1);
    r.term.mouse(p(2).0, p(2).1, 1);
    r.draw();
    assert_ne!(
        r.replay.pixel(0, 1, 1),
        Some(BG),
        "selected cell is drawn inverted"
    );
}

#[test]
fn sel_05_disable_copy() {
    let opts = TerminalOptions {
        disable_copy: true,
        ..Default::default()
    };
    let mut r = rig_with(30, 5, FontSet::bundled(), opts);
    r.term.feed(b"hello");
    drag(&mut r, (0, 0), (4, 0));
    assert_eq!(r.term.take_clipboard(), None);
}

#[test]
fn sel_06_click_without_drag_copies_nothing_useful() {
    let mut r = rig(30, 5);
    r.term.feed(b"hello");
    let (x, y) = ((r.cw / 4) as i32, (r.ch / 2) as i32);
    r.term.mouse(x, y, 1);
    r.term.mouse(x, y, 0);
    let c = r.term.take_clipboard();
    assert!(c.is_none() || c.as_deref() == Some(""), "{c:?}");
}

#[test]
fn sel_07_mouse_out_of_bounds_is_clamped() {
    let mut r = rig(30, 5);
    r.term.feed(b"hello");
    r.term.mouse(-100, -100, 1);
    r.term.mouse(100_000, 100_000, 1);
    r.term.mouse(100_000, 100_000, 0);
    r.draw();
}

// ---- CJK / fallback fonts ----

fn cjk_fonts() -> Option<FontSet> {
    let f = FontSet::discover();
    if f.fallback_count() > 0 && f.has_glyph('中') {
        Some(f)
    } else {
        None
    }
}

#[test]
fn rn_cjk_01_wide_glyphs_render_when_a_cjk_font_is_available() {
    let Some(fonts) = cjk_fonts() else {
        eprintln!("SKIP: no CJK font on this machine (set CRACAMOLE_FONT_DIRS)");
        return;
    };
    assert_eq!(
        fonts.fallback_count(),
        1,
        "system discovery loads one CJK face"
    );
    let mut r = rig_with(20, 3, fonts, TerminalOptions::default());
    r.term.feed("中a".as_bytes());
    r.draw();
    assert!(
        r.ink(0, 0, BG) + r.ink(1, 0, BG) > 20,
        "wide glyph drawn over two cells"
    );
    assert!(r.ink(2, 0, BG) > 5, "following ASCII aligned at cell 2");
}

#[test]
fn rn_cjk_02_missing_glyph_does_not_panic() {
    let mut r = rig(20, 3); // bundled font only: no CJK
    r.term.feed("中文\u{1F600}".as_bytes());
    r.draw();
}

#[test]
fn rn_hostile_dpi_and_nonfinite_font_sizes_bound_glyph_allocations() {
    for size in [12.0, f32::INFINITY, f32::NAN, -1.0] {
        let options = TerminalOptions {
            dpi: u32::MAX,
            font_size_pt: size,
            ..Default::default()
        };
        let mut terminal = Terminal::new(options, FontSet::bundled(), 320, 160);
        let (cw, ch) = terminal.cell_size();
        assert!((1..=256).contains(&cw));
        assert!((1..=256).contains(&ch));
        terminal.feed(b"M");
        let mut display = guac_display::Display::new(320, 160);
        terminal.render(&mut display);
        assert!(!display.flush().is_empty());
    }
}

#[test]
fn rn_delete_lines_while_viewing_history_keeps_viewport_in_bounds() {
    let mut r = rig(20, 5);
    r.term.feed(b"1\r\n2\r\n3\r\n4\r\n5\r\n6");
    r.term.mouse(0, 0, 8);
    r.term.mouse(0, 0, 0);
    assert_eq!(r.term.row_text(0), "1");
    // Delete a line below the first row while viewing the oldest history row.
    r.term.feed(b"\x1b[2;1H\x1b[M");
    r.draw();
    assert_eq!(r.term.row_text(0), "1");
}
