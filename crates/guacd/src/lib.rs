//! guacd: the Guacamole proxy daemon.

#![forbid(unsafe_code)]

pub mod config;
pub mod logging;
mod server;

pub use guac_terminal::TextEncoding;
pub use server::{serve, Registry, ServerConfig};

/// Inspect an RDP server's TLS certificate without authenticating (for the "trust this certificate" flow).
#[cfg(feature = "rdp")]
pub use proto_rdp::{normalize_fingerprint, probe_certificate, CertInfo};

#[cfg(feature = "ssh")]
pub use proto_ssh::{probe_host_key, HostKeyInfo};
