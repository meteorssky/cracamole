//! Terminal emulation (alacritty_terminal grid) + Guacamole-oriented rendering and input.

#![forbid(unsafe_code)]

pub mod colors;
pub mod font;
pub mod keyboard;
mod named_colors;
mod session;
mod terminal;

pub use colors::{parse_color_scheme, parse_xcolor, ColorScheme};
pub use font::FontSet;
pub use keyboard::{KeyModes, KeyOutcome, Keyboard};
pub use session::{run_terminal, FilterOutput, RemoteCmd, RemoteFilter, RemoteIo, TerminalConfig};
pub use terminal::{Terminal, TerminalOptions};
