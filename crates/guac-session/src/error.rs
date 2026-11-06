use guac_protocol::{ParseError, Status};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(#[from] ParseError),
    #[error("handshake violation: {0}")]
    Handshake(String),
    #[error("timed out")]
    Timeout,
    #[error("connection closed")]
    Closed,
    /// A failure reported by a protocol plugin, to be relayed to the client.
    #[error("{message}")]
    Plugin { status: Status, message: String },
}

impl SessionError {
    pub fn plugin(status: Status, message: impl Into<String>) -> Self {
        Self::Plugin {
            status,
            message: message.into(),
        }
    }

    /// Status code to report to the client in an `error` instruction.
    pub fn status(&self) -> Status {
        match self {
            Self::Io(_) | Self::Closed => Status::ServerError,
            Self::Protocol(_) | Self::Handshake(_) => Status::ClientBadRequest,
            Self::Timeout => Status::ClientTimeout,
            Self::Plugin { status, .. } => *status,
        }
    }
}
