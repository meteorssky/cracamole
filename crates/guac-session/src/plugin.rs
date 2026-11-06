use crate::{Params, SessionCtx, SessionError};
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A remote-desktop/terminal protocol. This is the only extension point:
/// plugins talk to the client exclusively through [`SessionCtx`].
pub trait ProtocolPlugin: Send + Sync + 'static {
    /// Protocol name as sent in `select` (e.g. "ssh").
    fn name(&self) -> &'static str;

    /// Ordered connection-parameter names advertised in the `args` instruction.
    fn arg_names(&self) -> &'static [&'static str];

    /// Run the session until it ends. Returning `Err` sends an `error` instruction
    /// to the client; `Ok` is a clean disconnect. Must stop when `ctx.cancelled()` fires.
    fn run(&self, ctx: SessionCtx, params: Params) -> BoxFuture<Result<(), SessionError>>;
}
