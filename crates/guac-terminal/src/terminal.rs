use crate::colors::{ColorScheme, Rgb};
use crate::font::FontSet;
use crate::keyboard::{KeyModes, Keyboard};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};
use guac_display::{Display, Rect};
use std::sync::{Arc, Mutex};

const MOUSE_LEFT: u32 = 1;
const MOUSE_SCROLL_UP: u32 = 8;
const MOUSE_SCROLL_DOWN: u32 = 16;
const WHEEL_LINES: i32 = 3;

/// Grid bounds. A single column makes alacritty's reflow request ~20 GB (found by the stress test),
/// and wide characters need two cells anyway. The upper bounds stop a hostile `size` from
/// allocating a huge grid.
const MIN_COLS: usize = 2;
const MIN_ROWS: usize = 1;
const MAX_COLS: usize = 1000;
const MAX_ROWS: usize = 500;

#[derive(Default)]
struct Shared {
    /// Bytes the emulator wants sent back to the remote (DSR/DA answers).
    to_remote: Vec<u8>,
    title: Option<String>,
}

#[derive(Clone)]
struct Listener(Arc<Mutex<Shared>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let mut s = self.0.lock().unwrap();
        match event {
            Event::PtyWrite(text) => s.to_remote.extend_from_slice(text.as_bytes()),
            Event::Title(t) => s.title = Some(t),
            _ => {}
        }
    }
}

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Debug, Clone)]
pub struct TerminalOptions {
    /// Font size in points (`font-size`, default 12).
    pub font_size_pt: f32,
    pub dpi: u32,
    /// Scrollback lines (`scrollback`, default 1000).
    pub scrollback: usize,
    /// Byte sent for Backspace (`backspace`, default 127).
    pub backspace: u8,
    pub color_scheme: ColorScheme,
    /// `func-keys-and-keypad=vt100`.
    pub vt100_func_keys: bool,
    /// Linux-console style Home/End encodings; C enables this exactly when `terminal-type` is `linux`.
    pub linux_console_keys: bool,
    /// Disable selection copy (`disable-copy`).
    pub disable_copy: bool,
    /// Disable paste (`disable-paste`).
    pub disable_paste: bool,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            font_size_pt: 12.0,
            dpi: 96,
            scrollback: 1000,
            backspace: 127,
            color_scheme: ColorScheme::default(),
            vt100_func_keys: false,
            linux_console_keys: true,
            disable_copy: false,
            disable_paste: false,
        }
    }
}

pub struct Terminal {
    term: Term<Listener>,
    parser: Processor,
    shared: Arc<Mutex<Shared>>,
    opts: TerminalOptions,
    keyboard: Keyboard,
    fonts: FontSet,
    px: f32,
    cell_w: u32,
    cell_h: u32,
    ascent: u32,
    cols: usize,
    rows: usize,
    width_px: u32,
    height_px: u32,
    out: Vec<u8>,
    clip_out: Option<String>,
    local_clipboard: String,
    prev_mask: u32,
    selecting: bool,
    force_full: bool,
    /// Per-row content hashes of what is currently on the display.
    prev_hashes: Vec<u64>,
    started: bool,
}

fn rgba(c: Rgb) -> [u8; 4] {
    [c[0], c[1], c[2], 255]
}

impl Terminal {
    pub fn new(opts: TerminalOptions, fonts: FontSet, width_px: u32, height_px: u32) -> Self {
        // DPI is supplied by the Guacamole client. Bound rasterisation as well as the
        // framebuffer: a tiny grid can otherwise still request an enormous glyph bitmap.
        let requested_px = opts.font_size_pt * opts.dpi as f32 / 72.0;
        let px = if requested_px.is_finite() {
            requested_px.clamp(1.0, 128.0)
        } else {
            16.0
        };
        let (cell_w, cell_h, ascent) = fonts.metrics(px);
        let cols = ((width_px / cell_w) as usize).clamp(MIN_COLS, MAX_COLS);
        let rows = ((height_px / cell_h) as usize).clamp(MIN_ROWS, MAX_ROWS);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let config = Config {
            scrolling_history: opts.scrollback,
            ..Config::default()
        };
        let term = Term::new(config, &Size { cols, rows }, Listener(shared.clone()));
        Self {
            term,
            parser: Processor::new(),
            shared,
            opts,
            keyboard: Keyboard::default(),
            fonts,
            px,
            cell_w,
            cell_h,
            ascent,
            cols,
            rows,
            width_px,
            height_px,
            out: vec![],
            clip_out: None,
            local_clipboard: String::new(),
            prev_mask: 0,
            selecting: false,
            force_full: true,
            prev_hashes: vec![],
            started: true,
        }
    }

    pub fn size_cells(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    pub fn size_px(&self) -> (u32, u32) {
        (self.width_px, self.height_px)
    }

    pub fn cell_size(&self) -> (u32, u32) {
        (self.cell_w, self.cell_h)
    }

    pub fn title(&self) -> Option<String> {
        self.shared.lock().unwrap().title.clone()
    }

    /// Feed bytes received from the remote host.
    pub fn feed(&mut self, data: &[u8]) {
        self.parser.advance(&mut self.term, data);
        self.clamp_viewport();
        let mut s = self.shared.lock().unwrap();
        self.out.append(&mut s.to_remote);
    }

    /// Bytes to send to the remote host (keyboard input, paste, emulator replies).
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    /// Text newly copied by selecting with the mouse, to be sent as a clipboard stream.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clip_out.take()
    }

    fn key_modes(&self) -> KeyModes {
        let mode = self.term.mode();
        KeyModes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            app_keypad: mode.contains(TermMode::APP_KEYPAD),
            vt100_func_keys: self.opts.vt100_func_keys,
            linux_console_keys: self.opts.linux_console_keys,
            backspace: self.opts.backspace,
            rows: self.rows,
            max_scrollback: self.opts.scrollback,
        }
    }

    /// While not started (e.g. waiting for a telnet login to succeed) nothing is drawn and
    /// user input is ignored, as in the C implementation.
    pub fn set_started(&mut self, started: bool) {
        if started && !self.started {
            self.force_full = true;
        }
        self.started = started;
    }

    pub fn is_started(&self) -> bool {
        self.started
    }

    pub fn key(&mut self, keysym: u32, pressed: bool) {
        if !self.started {
            return;
        }
        let modes = self.key_modes();
        let o = self.keyboard.process(keysym, pressed, &modes);
        if o.paste {
            let text = self.local_clipboard.clone();
            self.paste(&text);
            return;
        }
        if o.scroll != 0 {
            self.term.scroll_display(Scroll::Delta(
                o.scroll.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            ));
            self.force_full = true;
        } else if o.reset_scroll && self.term.grid().display_offset() != 0 {
            self.term.scroll_display(Scroll::Bottom);
            self.force_full = true;
        }
        self.out.extend_from_slice(&o.send);
    }

    /// Paste text from the client's clipboard into the remote.
    pub fn paste(&mut self, text: &str) {
        if self.opts.disable_paste {
            return;
        }
        let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
        let bracketed = self.term.mode().contains(TermMode::BRACKETED_PASTE);
        if bracketed {
            self.out.extend_from_slice(b"\x1b[200~");
        }
        self.out.extend_from_slice(normalized.as_bytes());
        if bracketed {
            self.out.extend_from_slice(b"\x1b[201~");
        }
    }

    /// Clipboard data received from the client; used for paste shortcuts and middle-click style paste.
    pub fn set_local_clipboard(&mut self, text: String) {
        self.local_clipboard = text.clone();
        self.paste(&text);
    }

    fn point_at(&self, x: i32, y: i32) -> (Point, Side) {
        let col = (x.max(0) as u32 / self.cell_w) as usize;
        let row = (y.max(0) as u32 / self.cell_h) as i32;
        let col = col.min(self.cols - 1);
        let row = row.min(self.rows as i32 - 1);
        let offset = self.term.grid().display_offset() as i32;
        let side = if (x.max(0) as u32 % self.cell_w) * 2 < self.cell_w {
            Side::Left
        } else {
            Side::Right
        };
        (Point::new(Line(row - offset), Column(col)), side)
    }

    pub fn mouse(&mut self, x: i32, y: i32, mask: u32) {
        if !self.started {
            return;
        }
        let pressed = mask & MOUSE_LEFT != 0;
        let was = self.prev_mask & MOUSE_LEFT != 0;
        if mask & MOUSE_SCROLL_UP != 0 && self.prev_mask & MOUSE_SCROLL_UP == 0 {
            self.term.scroll_display(Scroll::Delta(WHEEL_LINES));
            self.force_full = true;
        }
        if mask & MOUSE_SCROLL_DOWN != 0 && self.prev_mask & MOUSE_SCROLL_DOWN == 0 {
            self.term.scroll_display(Scroll::Delta(-WHEEL_LINES));
            self.force_full = true;
        }
        let (point, side) = self.point_at(x, y);
        if pressed && !was {
            self.term.selection = Some(Selection::new(SelectionType::Simple, point, side));
            self.selecting = true;
            self.force_full = true;
        } else if pressed && self.selecting {
            if let Some(sel) = self.term.selection.as_mut() {
                sel.update(point, side);
                self.force_full = true;
            }
        } else if !pressed && was && self.selecting {
            self.selecting = false;
            if let Some(text) = self.term.selection_to_string() {
                if !text.is_empty() && !self.opts.disable_copy {
                    self.clip_out = Some(text);
                }
            }
            self.force_full = true;
        }
        self.prev_mask = mask;
    }

    /// Current text of visible row `row` (0-based), trailing blanks trimmed. Test/diagnostic helper.
    pub fn row_text(&self, row: usize) -> String {
        let offset = self.term.grid().display_offset() as i32;
        let mut s = String::new();
        for c in 0..self.cols {
            let cell = &self.term.grid()[Point::new(Line(row as i32 - offset), Column(c))];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            s.push(if cell.c == '\t' || cell.c == '\0' {
                ' '
            } else {
                cell.c
            });
        }
        s.trim_end().to_string()
    }

    pub fn screen_text(&self) -> Vec<String> {
        (0..self.rows).map(|r| self.row_text(r)).collect()
    }

    /// Cursor position as (row, col) on screen.
    pub fn cursor(&self) -> (usize, usize) {
        let p = self.term.grid().cursor.point;
        (p.line.0.max(0) as usize, p.column.0)
    }

    /// Resize to a new pixel size. Returns the new cell grid when it changed.
    pub fn resize_px(&mut self, width_px: u32, height_px: u32) -> Option<(usize, usize)> {
        self.width_px = width_px;
        self.height_px = height_px;
        let cols = ((width_px / self.cell_w) as usize).clamp(MIN_COLS, MAX_COLS);
        let rows = ((height_px / self.cell_h) as usize).clamp(MIN_ROWS, MAX_ROWS);
        self.force_full = true;
        if (cols, rows) == (self.cols, self.rows) {
            return None;
        }
        self.cols = cols;
        self.rows = rows;
        self.term.resize(Size { cols, rows });
        self.clamp_viewport();
        Some((cols, rows))
    }

    fn clamp_viewport(&mut self) {
        // alacritty 0.26 increments display_offset on partial-region deletes even
        // though those deletes do not add history. Restore its public grid invariant
        // before selection or rendering indexes the visible rows.
        if self.term.grid().display_offset() > self.term.grid().history_size() {
            self.term.scroll_display(Scroll::Delta(0));
            self.force_full = true;
        }
    }

    fn resolve(&self, c: Color, is_fg: bool, bold: bool) -> Rgb {
        let sch = &self.opts.color_scheme;
        match c {
            Color::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
            Color::Indexed(i) => {
                let i = if bold && i < 8 { i + 8 } else { i };
                sch.palette[i as usize]
            }
            Color::Named(n) => {
                let idx = n as usize;
                match n {
                    NamedColor::Foreground | NamedColor::BrightForeground => sch.foreground,
                    NamedColor::Background => sch.background,
                    NamedColor::DimForeground => dim(sch.foreground),
                    NamedColor::Cursor => {
                        if is_fg {
                            sch.background
                        } else {
                            sch.foreground
                        }
                    }
                    _ if idx < 16 => {
                        let i = if bold && is_fg && idx < 8 {
                            idx + 8
                        } else {
                            idx
                        };
                        sch.palette[i]
                    }
                    // DimBlack..DimWhite
                    _ if (259..=266).contains(&idx) => dim(sch.palette[idx - 259]),
                    _ => {
                        if is_fg {
                            sch.foreground
                        } else {
                            sch.background
                        }
                    }
                }
            }
        }
    }

    /// Draw everything that changed into `display` (does not flush it).
    pub fn render(&mut self, display: &mut Display) {
        if !self.started {
            return;
        }
        if display.size() != (self.width_px, self.height_px) {
            display.resize(self.width_px, self.height_px);
            self.force_full = true;
        }

        // alacritty's own damage tracking always reports the cursor line, so it cannot tell us
        // "nothing changed". Compare a content hash per visible row instead (cells + cursor + selection).
        drop(self.term.damage());
        self.term.reset_damage();

        let hashes = self.row_hashes();
        if self.force_full {
            // Paint margins outside the cell grid (resize remainder).
            let bg = rgba(self.opts.color_scheme.background);
            display.fill_rect(Rect::new(0, 0, self.width_px, self.height_px), bg);
            self.prev_hashes.clear();
            self.force_full = false;
        }
        for (row, h) in hashes.iter().enumerate() {
            if self.prev_hashes.get(row) != Some(h) {
                self.draw_row(display, row);
            }
        }
        self.prev_hashes = hashes;
    }

    fn row_hashes(&self) -> Vec<u64> {
        use std::hash::{Hash, Hasher};
        let offset = self.term.grid().display_offset() as i32;
        let show_cursor = self.term.mode().contains(TermMode::SHOW_CURSOR) && offset == 0;
        let cursor_pt = self.term.grid().cursor.point;
        let selection = self
            .term
            .selection
            .as_ref()
            .and_then(|s| s.to_range(&self.term));
        (0..self.rows)
            .map(|row| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for col in 0..self.cols {
                    let pt = Point::new(Line(row as i32 - offset), Column(col));
                    let cell = &self.term.grid()[pt];
                    cell.c.hash(&mut h);
                    hash_color(cell.fg, &mut h);
                    hash_color(cell.bg, &mut h);
                    cell.flags.bits().hash(&mut h);
                    (show_cursor && pt == cursor_pt).hash(&mut h);
                    selection
                        .as_ref()
                        .is_some_and(|s| s.contains(pt))
                        .hash(&mut h);
                }
                h.finish()
            })
            .collect()
    }

    fn draw_row(&mut self, display: &mut Display, row: usize) {
        let offset = self.term.grid().display_offset() as i32;
        let show_cursor = self.term.mode().contains(TermMode::SHOW_CURSOR) && offset == 0;
        let cursor_pt = self.term.grid().cursor.point;
        let selection = self
            .term
            .selection
            .as_ref()
            .and_then(|s| s.to_range(&self.term));
        let (cw, ch) = (self.cell_w, self.cell_h);
        let y0 = row as u32 * ch;
        if y0 >= self.height_px {
            return;
        }
        let row_h = ch.min(self.height_px - y0);

        let mut col = 0usize;
        while col < self.cols {
            let pt = Point::new(Line(row as i32 - offset), Column(col));
            let cell = self.term.grid()[pt].clone();
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                col += 1;
                continue;
            }
            let wide = cell.flags.contains(Flags::WIDE_CHAR);
            let span = if wide { 2 } else { 1 };
            let bold = cell.flags.contains(Flags::BOLD);
            let mut fg = self.resolve(cell.fg, true, bold);
            let mut bg = self.resolve(cell.bg, false, false);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = dim(fg);
            }
            let selected = selection.as_ref().is_some_and(|s| s.contains(pt));
            let is_cursor = show_cursor && pt == cursor_pt;
            if selected != is_cursor {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::HIDDEN) {
                fg = bg;
            }

            let x0 = col as u32 * cw;
            let w = (cw * span as u32).min(self.width_px.saturating_sub(x0));
            if w > 0 {
                display.fill_rect(Rect::new(x0 as i32, y0 as i32, w, row_h), rgba(bg));
            }

            let c = cell.c;
            if c != ' ' && c != '\0' && !cell.flags.contains(Flags::HIDDEN) {
                let ascent = self.ascent as i32;
                if let Some(g) = self.fonts.glyph(c, bold, self.px) {
                    let gx = x0 as i32 + g.xmin;
                    let gy = y0 as i32 + ascent - (g.ymin + g.height as i32);
                    for yy in 0..g.height {
                        for xx in 0..g.width {
                            let a = g.coverage[yy * g.width + xx] as u32;
                            if a == 0 {
                                continue;
                            }
                            let (px_, py_) = (gx + xx as i32, gy + yy as i32);
                            if px_ < x0 as i32 || py_ < y0 as i32 {
                                continue;
                            }
                            if (px_ as u32) >= x0 + w || (py_ as u32) >= y0 + row_h {
                                continue;
                            }
                            let mix =
                                |f: u8, b: u8| ((f as u32 * a + b as u32 * (255 - a)) / 255) as u8;
                            let out =
                                [mix(fg[0], bg[0]), mix(fg[1], bg[1]), mix(fg[2], bg[2]), 255];
                            display.put_pixel_raw(px_ as u32, py_ as u32, out);
                        }
                    }
                }
            }
            if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                let uy = (y0 + self.ascent + 1).min(y0 + row_h - 1);
                display.fill_rect(Rect::new(x0 as i32, uy as i32, w, 1), rgba(fg));
            }
            if cell.flags.contains(Flags::STRIKEOUT) {
                display.fill_rect(
                    Rect::new(x0 as i32, (y0 + row_h / 2) as i32, w, 1),
                    rgba(fg),
                );
            }
            col += span;
        }
        display.mark_dirty(Rect::new(0, y0 as i32, self.width_px, row_h));
    }
}

fn dim(c: Rgb) -> Rgb {
    [
        (c[0] as u32 * 2 / 3) as u8,
        (c[1] as u32 * 2 / 3) as u8,
        (c[2] as u32 * 2 / 3) as u8,
    ]
}

fn hash_color<H: std::hash::Hasher>(c: Color, h: &mut H) {
    use std::hash::Hash;
    match c {
        Color::Named(n) => (0u8, n as u32).hash(h),
        Color::Spec(rgb) => (1u8, rgb.r, rgb.g, rgb.b).hash(h),
        Color::Indexed(i) => (2u8, i).hash(h),
    }
}
