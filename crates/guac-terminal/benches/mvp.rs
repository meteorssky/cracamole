//! Measurements, not acceptance thresholds. Run: cargo bench -p guac-terminal --bench mvp
use guac_display::Display;
use guac_protocol::{Instruction, Parser};
use guac_terminal::{FontSet, Terminal, TerminalOptions};
use std::hint::black_box;
use std::time::Instant;

fn render(label: &str, fonts: FontSet, text: &str) {
    let (cw, ch, _) = fonts.metrics(16.0);
    let mut terminal = Terminal::new(TerminalOptions::default(), fonts, 80 * cw, 24 * ch);
    let mut display = Display::new(80 * cw, 24 * ch);
    let screen = format!("\x1b[H{}", (text.to_owned() + "\r\n").repeat(24));
    terminal.feed(screen.as_bytes());
    terminal.render(&mut display);
    black_box(display.flush());
    let mut micros = Vec::new();
    let mut bytes = 0;
    for i in 0..100 {
        // Alternate the entire screen's attributes so all rows actually need repainting.
        terminal.feed(if i % 2 == 0 { b"\x1b[7m" } else { b"\x1b[0m" });
        terminal.feed(screen.as_bytes());
        let start = Instant::now();
        terminal.render(&mut display);
        let frame = display.flush();
        micros.push(start.elapsed().as_micros());
        bytes += frame.iter().map(|i| i.encode().len()).sum::<usize>();
        black_box(frame);
    }
    micros.sort_unstable();
    println!(
        "{label}: render+PNG p50={} us p95={} us, mean wire={} bytes/frame",
        micros[50],
        micros[95],
        bytes / 100
    );
}

fn main() {
    let instruction = Instruction::new("blob", ["1", &"A".repeat(4096)]);
    let wire = instruction.encode();
    let start = Instant::now();
    for _ in 0..20_000 {
        let mut parser = Parser::new();
        parser.feed(black_box(&wire)).unwrap();
        black_box(parser.next_instruction().unwrap());
    }
    println!(
        "parser: {:.2} MiB/s (4096-byte payload)",
        (wire.len() * 20_000) as f64 / start.elapsed().as_secs_f64() / 1048576.0
    );
    render(
        "ASCII 80x24",
        FontSet::bundled(),
        "The quick brown fox 0123456789",
    );
    let fonts = FontSet::discover();
    if fonts.has_glyph('中') {
        render("CJK 80x24", fonts, "中文終端機測試 abc 0123456789");
    } else {
        println!("CJK: UNVERIFIED (no fallback font; set CRACAMOLE_FONT_DIRS)");
    }
}
