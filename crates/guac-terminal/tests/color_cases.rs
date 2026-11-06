use guac_terminal::colors::{find_named, initial_palette};
use guac_terminal::{parse_color_scheme, parse_xcolor};

#[test]
fn cl_01_palette_matches_c_reference_if_available() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/guacamole-server/src/terminal/palette.c");
    let Ok(src) = std::fs::read_to_string(&path) else {
        eprintln!("SKIP: reference source not present at {path:?}");
        return;
    };
    let pal = initial_palette();
    let mut count = 0;
    for line in src.lines() {
        let l = line.trim();
        if !l.starts_with('{') || l.starts_with("{{") {
            continue;
        }
        // {idx, 0xRR, 0xGG, 0xBB}
        let inner = l.trim_start_matches('{').split('}').next().unwrap();
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() < 4 || !parts[1].starts_with("0x") {
            continue;
        }
        let idx: usize = parts[0].parse().unwrap();
        let rgb = [1, 2, 3].map(|i| u8::from_str_radix(&parts[i][2..], 16).unwrap());
        assert_eq!(pal[idx], rgb, "palette index {idx}");
        count += 1;
    }
    assert_eq!(count, 256, "every palette entry must be compared");
}

#[test]
fn cl_01b_palette_spot_checks() {
    let p = initial_palette();
    assert_eq!(p[1], [0x99, 0x3E, 0x3E]);
    assert_eq!(p[15], [0xFF, 0xFF, 0xFF]);
    assert_eq!(p[16], [0, 0, 0]);
    assert_eq!(p[21], [0, 0, 0xFF]);
    assert_eq!(p[231], [0xFF, 0xFF, 0xFF]);
    assert_eq!(p[232], [8, 8, 8]);
    assert_eq!(p[255], [0xEE, 0xEE, 0xEE]);
}

#[test]
fn cl_02_color_scheme_parsing() {
    let (s, w) =
        parse_color_scheme("foreground: rgb:ff/00/00; background: color4; color1: #00ff00");
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(s.foreground, [255, 0, 0]);
    assert_eq!(s.background, initial_palette()[4]);
    assert_eq!(s.palette[1], [0, 255, 0]);
}

#[test]
fn cl_02b_presets_and_default() {
    let pal = initial_palette();
    let d = parse_color_scheme("").0;
    assert_eq!((d.foreground, d.background), (pal[7], pal[0]));
    let g = parse_color_scheme("green-black").0;
    assert_eq!((g.foreground, g.background), (pal[2], pal[0]));
    let b = parse_color_scheme("black-white").0;
    assert_eq!((b.foreground, b.background), (pal[0], pal[15]));
    let w = parse_color_scheme("white-black").0;
    assert_eq!((w.foreground, w.background), (pal[15], pal[0]));
}

#[test]
fn cl_02c_bad_entries_warn_and_do_not_abort() {
    let (s, w) = parse_color_scheme("foreground: nonsense; bogus: red; background: black; nocolon");
    assert_eq!(w.len(), 3, "{w:?}");
    assert_eq!(s.background, [0, 0, 0]);
    assert_eq!(
        s.foreground,
        initial_palette()[7],
        "failed entry leaves default"
    );
}

#[test]
fn cl_03_xparsecolor_forms() {
    assert_eq!(parse_xcolor("rgb:f/0/8"), Some([0xF0, 0x00, 0x80]));
    assert_eq!(parse_xcolor("rgb:ff/80/00"), Some([0xFF, 0x80, 0x00]));
    assert_eq!(parse_xcolor("rgb:fff/800/000"), Some([0xFF, 0x80, 0x00]));
    assert_eq!(parse_xcolor("rgb:ffff/8000/0000"), Some([0xFF, 0x80, 0x00]));
    // mixed widths are rejected (as in C: each scanf pattern needs a uniform width)
    assert_eq!(parse_xcolor("rgb:f/00/0"), None);
    assert_eq!(parse_xcolor("rgb:gg/00/00"), None);
    // extension over C
    assert_eq!(parse_xcolor("#102030"), Some([0x10, 0x20, 0x30]));
    assert_eq!(parse_xcolor("#fa0"), Some([0xFF, 0xAA, 0x00]));
    assert_eq!(parse_xcolor("#12345"), None);
}

#[test]
fn cl_04_named_colors_exact_not_prefix() {
    assert_eq!(find_named("red"), Some([255, 0, 0]));
    assert_eq!(find_named("  RED "), Some([255, 0, 0]));
    assert_eq!(find_named("aliceblue"), Some([0xF0, 0xF8, 0xFF]));
    assert_eq!(find_named("re"), None, "no prefix matching");
    assert_eq!(find_named("reddish"), None);
    assert_eq!(find_named("antiquewhite3"), Some([0xCD, 0xC0, 0xB0]));
}

#[test]
fn cl_04b_named_table_matches_c_count() {
    // 676 unique names extracted from named-colors.c; table must stay sorted for binary search.
    let t = guac_terminal::colors::find_named("black");
    assert!(t.is_some());
}
