#![no_main]
use guac_terminal::{parse_color_scheme, parse_xcolor};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = parse_xcolor(text);
        let _ = parse_color_scheme(text);
    }
});
