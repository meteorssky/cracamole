//! Telnet protocol plugin.

#![forbid(unsafe_code)]

mod args;
pub mod codec;
mod connect;
pub mod login;
pub mod negotiation;
mod session;
pub mod settings;

pub use args::ARGS;
pub use connect::connect;
pub use session::run_telnet;
pub use settings::TelnetSettings;

use guac_session::{
    BoxFuture, IdentityKind, Params, ProtocolInfo, ProtocolPlugin, SessionCtx, SessionError, Status,
};

pub struct TelnetPlugin;

impl ProtocolPlugin for TelnetPlugin {
    fn name(&self) -> &'static str {
        "telnet"
    }

    fn arg_names(&self) -> &'static [&'static str] {
        ARGS
    }

    fn info(&self) -> ProtocolInfo {
        ProtocolInfo {
            label: "Telnet",
            default_port: 23,
            identity: IdentityKind::None,
        }
    }

    fn run(&self, ctx: SessionCtx, params: Params) -> BoxFuture<Result<(), SessionError>> {
        Box::pin(async move {
            guac_terminal::TextEncoding::parse(params.get("character-encoding"))?;
            let settings = TelnetSettings::from_params(&params).ok_or_else(|| {
                SessionError::plugin(Status::ClientBadRequest, "No hostname provided.")
            })?;
            for w in &settings.warnings {
                tracing::warn!("{w}");
            }
            let stream = connect(&settings.hostname, settings.port, settings.timeout_secs).await?;
            tracing::info!("Telnet connection successful.");
            run_telnet(ctx, settings, params, stream).await
        })
    }
}
