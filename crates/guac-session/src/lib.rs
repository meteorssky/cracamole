//! Connection handshake, input dispatch, backpressure and shutdown semantics.

#![forbid(unsafe_code)]

mod error;
mod handshake;
mod net;
mod params;
mod plugin;
mod reader;
mod session;

pub use error::SessionError;
pub use handshake::{handshake, ClientCaps, Handshake, HANDSHAKE_TIMEOUT};
pub use net::connect_tcp;
pub use params::Params;
pub use plugin::{BoxFuture, IdentityKind, ProtocolInfo, ProtocolPlugin};
pub use reader::InstructionReader;
pub use session::{run_session, InputEvent, SessionConfig, SessionCtx, SessionTx};

pub use guac_protocol::{Instruction, Status};

/// Generate a connection id in the form used by guacd (`$` + UUID).
pub fn new_connection_id() -> String {
    format!("${}", uuid::Uuid::new_v4())
}
mod clipboard;
