//! SSH protocol plugin (pure Rust, `russh`).

#![forbid(unsafe_code)]

mod args;
pub mod hostkey;
pub mod keys;
mod probe;
pub mod prompt;
mod session;
pub mod settings;

pub use args::ARGS;
pub use probe::{probe_host_key, HostKeyInfo};
pub use session::run_ssh;
pub use settings::SshSettings;

use guac_session::{BoxFuture, Params, ProtocolPlugin, SessionCtx, SessionError, Status};

pub struct SshPlugin;

impl ProtocolPlugin for SshPlugin {
    fn name(&self) -> &'static str {
        "ssh"
    }

    fn arg_names(&self) -> &'static [&'static str] {
        ARGS
    }

    fn run(&self, ctx: SessionCtx, params: Params) -> BoxFuture<Result<(), SessionError>> {
        Box::pin(async move {
            let tz = ctx.tx.caps().timezone.clone();
            let settings = SshSettings::from_params(&params, tz.as_deref()).ok_or_else(|| {
                SessionError::plugin(Status::ClientBadRequest, "No hostname provided.")
            })?;
            for w in &settings.warnings {
                tracing::warn!("{w}");
            }
            run_ssh(ctx, settings, params).await
        })
    }
}
