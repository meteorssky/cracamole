//! RDP protocol plugin (IronRDP client engine, Guacamole display output).

#![forbid(unsafe_code)]

mod args;
mod clipboard;
mod connect;
pub mod keymap;
mod session;
pub mod settings;

pub use args::ARGS;
pub use connect::{fingerprint, probe_certificate, CertInfo};
pub use settings::{normalize_fingerprint, CertPin, RdpSettings, Security};

use guac_session::{BoxFuture, Params, ProtocolPlugin, SessionCtx, SessionError, Status};

pub struct RdpPlugin;

impl ProtocolPlugin for RdpPlugin {
    fn name(&self) -> &'static str {
        "rdp"
    }

    fn arg_names(&self) -> &'static [&'static str] {
        ARGS
    }

    fn run(&self, ctx: SessionCtx, params: Params) -> BoxFuture<Result<(), SessionError>> {
        Box::pin(async move {
            let settings = RdpSettings::from_params(&params).ok_or_else(|| {
                SessionError::plugin(Status::ClientBadRequest, "No hostname provided.")
            })?;
            for w in &settings.warnings {
                tracing::warn!("{w}");
            }
            session::run_rdp(ctx, settings).await
        })
    }
}
