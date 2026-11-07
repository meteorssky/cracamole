//! VNC (RFB) protocol plugin with Apple Remote Desktop authentication for macOS Screen Sharing.
//!
//! The client engine is implemented here: Raw, CopyRect, Hextile and ZRLE encodings, VNC password
//! and ARD (Diffie-Hellman + AES) authentication, text clipboard, and desktop size changes.
//! Traffic after authentication is not encrypted by the protocol; use a trusted network or tunnel.

#![forbid(unsafe_code)]

mod args;
pub mod auth;
pub mod decode;
pub mod format;
pub mod handshake;
pub mod messages;
pub mod screen;
pub mod session;
pub mod settings;

pub use args::ARGS;
pub use settings::{ClipboardEncoding, VncSettings};

use guac_session::{BoxFuture, Params, ProtocolPlugin, SessionCtx, SessionError, Status};

pub struct VncPlugin;

impl ProtocolPlugin for VncPlugin {
    fn name(&self) -> &'static str {
        "vnc"
    }

    fn arg_names(&self) -> &'static [&'static str] {
        ARGS
    }

    fn run(&self, ctx: SessionCtx, params: Params) -> BoxFuture<Result<(), SessionError>> {
        Box::pin(async move {
            let settings = VncSettings::from_params(&params).ok_or_else(|| {
                SessionError::plugin(Status::ClientBadRequest, "No hostname provided.")
            })?;
            for w in &settings.warnings {
                tracing::warn!("{w}");
            }
            session::run_vnc(ctx, settings).await
        })
    }
}
