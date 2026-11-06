//! Guacamole wire protocol: incremental parser and encoder.
//!
//! No I/O, no async, no panics on untrusted input.

#![forbid(unsafe_code)]

mod error;
mod instruction;
mod limits;
mod mouse;
mod parser;
mod status;
mod version;

pub use error::ParseError;
pub use instruction::Instruction;
pub use limits::{MAX_ARGS, MAX_ELEMENTS, MAX_ELEMENT_LEN};
pub use mouse::MouseMask;
pub use parser::Parser;
pub use status::Status;
pub use version::ProtocolVersion;
