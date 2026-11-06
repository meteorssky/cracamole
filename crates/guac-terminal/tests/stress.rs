use guac_display::Display;
use guac_terminal::{parse_color_scheme, parse_xcolor, FontSet, Terminal, TerminalOptions};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn iters(default: usize) -> usize {
    std::env::var("STRESS_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

const SNIPPETS: &[&str] = &[
    "\x1b[",
    "\x1b]0;title\x07",
    "\x1b[2J",
    "\x1b[H",
    "\x1b[?1049h",
    "\x1b[?1049l",
    "\x1b[38;2;1;2;3m",
    "\x1b[48;5;200m",
    "\x1b[999;999H",
    "\x1b[3J",
    "\x1b(0",
    "\x1b(B",
    "\x1bM",
    "\x1b[L",
    "\x1b[M",
    "\x1b[@",
    "\x1b[P",
    "\x1b[5S",
    "\x1b[5T",
    "\x1b[1;24r",
    "\x1b[?7l",
    "\x1b[?25l",
    "中文",
    "😀",
    "\u{0301}",
    "\r\n",
    "\t",
    "\x08",
    "text ",
    "\x1b[0m",
    "\x1b[6n",
    "\x1b[c",
    "\x1bc",
];

fn escape_soup(seed: u64, steps: usize) {
    let fonts = FontSet::bundled();
    let (cw, ch, _) = fonts.metrics(16.0);
    let mut t = Terminal::new(TerminalOptions::default(), fonts, 40 * cw, 12 * ch);
    let mut d = Display::new(40 * cw, 12 * ch);
    let mut rng = Rng(seed);
    for i in 0..steps {
        match rng.below(10) {
            0 => {
                let (w, h) = (
                    rng.below(60) as u32 * cw + rng.below(3) as u32,
                    rng.below(30) as u32 * ch,
                );
                t.resize_px(w, h);
            }
            1 => t.key(rng.below(0xFFFF) as u32, rng.below(2) == 0),
            2 => t.mouse(
                rng.below(2000) as i32 - 100,
                rng.below(1000) as i32 - 100,
                rng.below(32) as u32,
            ),
            3 => {
                let n = rng.below(200);
                let junk: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                t.feed(&junk);
            }
            _ => {
                for _ in 0..rng.below(6) + 1 {
                    t.feed(SNIPPETS[rng.below(SNIPPETS.len())].as_bytes());
                }
            }
        }
        if i % 7 == 0 {
            t.render(&mut d);
            let _ = d.flush();
        }
        let _ = t.take_output();
        let _ = t.take_clipboard();
        let (cols, rows) = t.size_cells();
        assert!(cols >= 1 && rows >= 1);
    }
}

#[test]
#[ignore]
fn stress_terminal_random_escape_soup_with_resizes_and_renders() {
    for seed in [0xC0FFEE, 1, 2, 3, 0xDEADBEEF] {
        escape_soup(seed, iters(20_000));
    }
}

/// Regression: this seed once drove the grid to a single column, where alacritty's reflow tried to
/// allocate ~20 GB (step 319). Kept as a fast, always-on test.
#[test]
fn regression_resize_to_one_column_after_random_activity() {
    escape_soup(0xC0FFEE, 400);
}

#[test]
#[ignore]
fn stress_color_parsers_on_noise() {
    let mut rng = Rng(99);
    let alphabet = b"rgb:/#;:abcdefABCDEF0123456789 colorforegroundbackground\n-_";
    for _ in 0..iters(200_000) {
        let n = rng.below(60);
        let s: String = (0..n)
            .map(|_| alphabet[rng.below(alphabet.len())] as char)
            .collect();
        let _ = parse_xcolor(&s);
        let _ = parse_color_scheme(&s);
    }
}
