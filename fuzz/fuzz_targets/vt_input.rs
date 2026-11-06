#![no_main]
use guac_display::Display;
use guac_terminal::{FontSet, Terminal, TerminalOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut terminal = Terminal::new(TerminalOptions::default(), FontSet::bundled(), 320, 160);
    // Control bytes change size between VT chunks, exercising grid reflow with wide glyphs.
    for chunk in data[..data.len().min(8192)].chunks(32) {
        if chunk.len() >= 2 {
            terminal.resize_px(u32::from(chunk[0]), u32::from(chunk[1]));
        }
        terminal.feed(chunk);
        let _ = terminal.take_output();
    }
    let mut display = Display::new(320, 160);
    terminal.render(&mut display);
    let _ = display.flush();
    let (cols, rows) = terminal.size_cells();
    assert!((2..=1000).contains(&cols));
    assert!((1..=500).contains(&rows));
});
