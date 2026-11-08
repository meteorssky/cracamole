use crate::{Params, SessionCtx, SessionError};
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// What a user can verify about a server before trusting it (see the web trust flow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityKind {
    /// The protocol has no server identity to pin.
    None,
    /// SSH-style public host key.
    HostKey,
    /// TLS server certificate, pinned by SHA-256 fingerprint.
    Certificate,
}

impl IdentityKind {
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::HostKey => Some("host-key"),
            Self::Certificate => Some("certificate"),
        }
    }
}

/// Presentation and trust metadata a protocol publishes about itself, so front ends do not need
/// to hard-code protocol names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolInfo {
    /// Human-readable name, e.g. "SSH".
    pub label: &'static str,
    /// Port used when a connection does not set one. 0 = none.
    pub default_port: u16,
    pub identity: IdentityKind,
}

/// A remote-desktop/terminal protocol. This is the only extension point:
/// plugins talk to the client exclusively through [`SessionCtx`].
pub trait ProtocolPlugin: Send + Sync + 'static {
    /// Protocol name as sent in `select` (e.g. "ssh").
    fn name(&self) -> &'static str;

    /// Ordered connection-parameter names advertised in the `args` instruction.
    fn arg_names(&self) -> &'static [&'static str];

    /// Display and trust metadata. The default is a bare label with no port or identity.
    fn info(&self) -> ProtocolInfo {
        ProtocolInfo {
            label: self.name(),
            default_port: 0,
            identity: IdentityKind::None,
        }
    }

    /// Run the session until it ends. Returning `Err` sends an `error` instruction
    /// to the client; `Ok` is a clean disconnect. Must stop when `ctx.cancelled()` fires.
    fn run(&self, ctx: SessionCtx, params: Params) -> BoxFuture<Result<(), SessionError>>;
}
