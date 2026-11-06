//! guacd: the Guacamole proxy daemon.

#![forbid(unsafe_code)]

pub mod config;
pub mod logging;
mod server;

pub use server::{serve, Registry, ServerConfig};
