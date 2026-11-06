//! Palette, colour-scheme and X11 colour parsing.

use crate::named_colors::NAMED_COLORS;

pub type Rgb = [u8; 3];

/// xterm 256-colour palette with Guacamole's first 16 colours.
pub fn initial_palette() -> [Rgb; 256] {
    let mut p = [[0u8; 3]; 256];
    let base: [Rgb; 16] = [
        [0x00, 0x00, 0x00],
        [0x99, 0x3E, 0x3E],
        [0x3E, 0x99, 0x3E],
        [0x99, 0x99, 0x3E],
        [0x3E, 0x3E, 0x99],
        [0x99, 0x3E, 0x99],
        [0x3E, 0x99, 0x99],
        [0x99, 0x99, 0x99],
        [0x3E, 0x3E, 0x3E],
        [0xFF, 0x67, 0x67],
        [0x67, 0xFF, 0x67],
        [0xFF, 0xFF, 0x67],
        [0x67, 0x67, 0xFF],
        [0xFF, 0x67, 0xFF],
        [0x67, 0xFF, 0xFF],
        [0xFF, 0xFF, 0xFF],
    ];
    p[..16].copy_from_slice(&base);
    const LEVELS: [u8; 6] = [0x00, 0x5F, 0x87, 0xAF, 0xD7, 0xFF];
    for i in 0..216usize {
        p[16 + i] = [LEVELS[i / 36], LEVELS[(i / 6) % 6], LEVELS[i % 6]];
    }
    for i in 0..24usize {
        let v = (8 + 10 * i) as u8;
        p[232 + i] = [v, v, v];
    }
    p
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorScheme {
    pub foreground: Rgb,
    pub background: Rgb,
    pub palette: [Rgb; 256],
}

impl Default for ColorScheme {
    fn default() -> Self {
        let palette = initial_palette();
        Self {
            foreground: palette[7],
            background: palette[0],
            palette,
        }
    }
}

fn hex_component(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(s, 16).ok()
}

/// X11 colour specification: `rgb:h/h/h` .. `rgb:hhhh/hhhh/hhhh`, `#RRGGBB`/`#RGB`
/// (the latter two are a deliberate extension over the C implementation), or a colour name.
pub fn parse_xcolor(spec: &str) -> Option<Rgb> {
    let spec = spec.trim();
    if let Some(rest) = spec.strip_prefix("rgb:") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() != 3 || parts.iter().any(|p| p.len() != parts[0].len()) {
            return None;
        }
        let vals: Option<Vec<u32>> = parts.iter().map(|p| hex_component(p)).collect();
        let vals = vals?;
        let len = parts[0].len();
        let scale = |v: u32| -> u8 {
            match len {
                1 => (v << 4) as u8,
                2 => v as u8,
                3 => (v >> 4) as u8,
                _ => (v >> 8) as u8,
            }
        };
        return Some([scale(vals[0]), scale(vals[1]), scale(vals[2])]);
    }
    if let Some(hex) = spec.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        return match hex.len() {
            6 => {
                let v = u32::from_str_radix(hex, 16).ok()?;
                Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
            }
            3 => {
                let v = u32::from_str_radix(hex, 16).ok()?;
                Some([
                    ((v >> 8) & 0xF) as u8 * 17,
                    ((v >> 4) & 0xF) as u8 * 17,
                    (v & 0xF) as u8 * 17,
                ])
            }
            _ => None,
        };
    }
    find_named(spec)
}

/// Exact (case-insensitive) colour-name lookup. The C implementation matched loosely;
/// this deliberately does not (recorded in DEVIATIONS).
pub fn find_named(name: &str) -> Option<Rgb> {
    let n = name.trim().to_ascii_lowercase();
    NAMED_COLORS
        .binary_search_by(|(k, _)| (*k).cmp(n.as_str()))
        .ok()
        .map(|i| NAMED_COLORS[i].1)
}

fn parse_color_index(s: &str) -> Option<usize> {
    s.strip_prefix("color")
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|&i| i <= 255)
}

/// Parse a `color-scheme` connection parameter. Returns the scheme and warnings for entries ignored.
///
/// Accepts the named presets (`gray-black`, `black-white`, `green-black`, `white-black`) and
/// `name: value; name: value` lists where name is `foreground`, `background` or `color<N>`, and value
/// is `color<N>` or an X11 colour.
pub fn parse_color_scheme(s: &str) -> (ColorScheme, Vec<String>) {
    let s = match s.trim() {
        "" | "gray-black" => "foreground:color7;background:color0",
        "black-white" => "foreground:color0;background:color15",
        "green-black" => "foreground:color2;background:color0",
        "white-black" => "foreground:color15;background:color0",
        other => other,
    };
    let mut scheme = ColorScheme::default();
    let mut warnings = vec![];
    for pair in s.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let Some((name, value)) = pair.split_once(':') else {
            warnings.push(format!("Missing ':' in color-scheme entry \"{pair}\""));
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        let color = if let Some(i) = parse_color_index(value) {
            Some(scheme.palette[i])
        } else {
            parse_xcolor(value)
        };
        let Some(color) = color else {
            warnings.push(format!("Invalid color value: \"{value}\""));
            continue;
        };
        match name {
            "foreground" => scheme.foreground = color,
            "background" => scheme.background = color,
            n => match parse_color_index(n) {
                Some(i) => scheme.palette[i] = color,
                None => warnings.push(format!("Unknown color name: \"{n}\"")),
            },
        }
    }
    (scheme, warnings)
}
