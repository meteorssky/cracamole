//! Font loading and glyph rasterisation (replaces Pango/Cairo; see plan decision D6).

use fontdue::{Font, FontSettings};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

static DEJAVU_MONO: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");
static DEJAVU_MONO_BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono-Bold.ttf");

/// A glyph rasterised at a given pixel size.
#[derive(Clone)]
pub struct Glyph {
    pub xmin: i32,
    pub ymin: i32,
    pub width: usize,
    pub height: usize,
    pub coverage: Vec<u8>,
}

/// Ordered font stack: bundled monospace first, then fallback fonts (CJK, symbols).
pub struct FontSet {
    regular: Arc<Font>,
    bold: Arc<Font>,
    fallbacks: Vec<Arc<Font>>,
    cache: HashMap<(char, bool, u32), Option<Glyph>>,
}

/// Parsed bundled fonts, shared by every session in the process.
static BUNDLED: OnceLock<(Arc<Font>, Arc<Font>)> = OnceLock::new();
/// System fallback fonts found on first use. Scanning and parsing large CJK fonts takes seconds,
/// so it is done once per process and the parsed fonts are shared.
static DISCOVERED: OnceLock<Vec<Arc<Font>>> = OnceLock::new();

fn load(bytes: &[u8]) -> Option<Font> {
    Font::from_bytes(bytes, FontSettings::default()).ok()
}

impl FontSet {
    /// Load (once per process) the bundled and system fonts. Call at startup, off the async
    /// workers, so the first connection does not pay for scanning and parsing large CJK fonts.
    pub fn warm_up() {
        let _ = Self::discover();
    }

    /// Bundled DejaVu Sans Mono only.
    pub fn bundled() -> Self {
        let (regular, bold) = BUNDLED.get_or_init(|| {
            (
                Arc::new(load(DEJAVU_MONO).expect("bundled font")),
                Arc::new(load(DEJAVU_MONO_BOLD).expect("bundled bold font")),
            )
        });
        Self {
            regular: regular.clone(),
            bold: bold.clone(),
            fallbacks: vec![],
            cache: HashMap::new(),
        }
    }

    /// Bundled fonts plus any CJK/fallback fonts discovered on the system.
    ///
    /// Search directories come from `CRACAMOLE_FONT_DIRS` (path-list) or well-known system locations.
    pub fn discover() -> Self {
        let mut s = Self::bundled();
        let found = DISCOVERED.get_or_init(|| {
            let dirs: Vec<PathBuf> = match std::env::var_os("CRACAMOLE_FONT_DIRS") {
                Some(v) => std::env::split_paths(&v).collect(),
                None => [
                    "/usr/share/fonts",
                    "/usr/local/share/fonts",
                    r"C:\Windows\Fonts",
                ]
                .iter()
                .map(PathBuf::from)
                .collect(),
            };
            let mut fonts = vec![];
            for d in dirs {
                scan_dir(&d, 3, &mut fonts);
            }
            fonts
        });
        s.fallbacks = found.clone();
        s
    }

    pub fn add_fallback_bytes(&mut self, bytes: &[u8]) -> bool {
        match load(bytes) {
            Some(f) => {
                self.fallbacks.push(Arc::new(f));
                self.cache.clear();
                true
            }
            None => false,
        }
    }

    pub fn fallback_count(&self) -> usize {
        self.fallbacks.len()
    }

    /// Cell metrics for `px` pixel size: `(cell_width, cell_height, ascent)`.
    pub fn metrics(&self, px: f32) -> (u32, u32, u32) {
        let adv = self.regular.metrics('M', px).advance_width;
        let lm = self.regular.horizontal_line_metrics(px);
        let (asc, desc) = lm
            .map(|l| (l.ascent, l.descent))
            .unwrap_or((px, -px * 0.25));
        let w = adv.round().max(1.0) as u32;
        let h = (asc - desc).ceil().max(1.0) as u32;
        (w, h, asc.ceil() as u32)
    }

    pub fn has_glyph(&self, ch: char) -> bool {
        self.regular.lookup_glyph_index(ch) != 0
            || self.fallbacks.iter().any(|f| f.lookup_glyph_index(ch) != 0)
    }

    /// Rasterise `ch`; `None` means no font has it (caller draws nothing).
    pub fn glyph(&mut self, ch: char, bold: bool, px: f32) -> Option<&Glyph> {
        let key = (ch, bold, px.to_bits());
        if !self.cache.contains_key(&key) {
            let base = if bold { &self.bold } else { &self.regular };
            let font = if base.lookup_glyph_index(ch) != 0 {
                Some(base.as_ref())
            } else if self.regular.lookup_glyph_index(ch) != 0 {
                Some(self.regular.as_ref())
            } else {
                self.fallbacks
                    .iter()
                    .map(|f| f.as_ref())
                    .find(|f| f.lookup_glyph_index(ch) != 0)
            };
            let g = font.map(|f| {
                let (m, coverage) = f.rasterize(ch, px);
                Glyph {
                    xmin: m.xmin,
                    ymin: m.ymin,
                    width: m.width,
                    height: m.height,
                    coverage,
                }
            });
            if self.cache.len() > 8192 {
                self.cache.clear();
            }
            self.cache.insert(key, g);
        }
        self.cache.get(&key).and_then(|g| g.as_ref())
    }
}

fn scan_dir(dir: &Path, depth: u32, out: &mut Vec<Arc<Font>>) {
    const WANTED: &[&str] = &[
        "cjk",
        "wenquanyi",
        "wqy",
        "msjh",
        "notosanstc",
        "notosanssc",
        "notosansjp",
        "notosanskr",
        "unifont",
        "simsun",
        "mingliu",
        "droidsansfallback",
    ];
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if depth > 0 {
                scan_dir(&p, depth - 1, out);
            }
            continue;
        }
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let ext_ok = name.ends_with(".ttf") || name.ends_with(".otf") || name.ends_with(".ttc");
        if ext_ok && WANTED.iter().any(|w| name.contains(w)) && out.len() < 4 {
            if let Ok(bytes) = std::fs::read(&p) {
                if let Some(f) = load(&bytes) {
                    out.push(Arc::new(f));
                }
            }
        }
    }
}
