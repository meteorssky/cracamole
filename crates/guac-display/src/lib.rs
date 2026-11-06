//! Single-layer display: RGBA framebuffer, dirty tracking, Guacamole drawing instructions.

#![forbid(unsafe_code)]

mod display;
mod rect;

pub use display::{Display, MAX_BLOB_BYTES, MAX_DIMENSION};
pub use rect::Rect;
