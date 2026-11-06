//! Test infrastructure: simulated Guacamole client, protocol display replay, helpers.
//! Never a dependency of production crates.

mod client;
mod display;

pub use client::{HandshakeOutcome, SimClient};
pub use display::{DisplayReplay, Pixel};
