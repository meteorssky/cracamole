use crate::codec::{escape_data, Decoder};
use crate::negotiation::Negotiator;
use crate::settings::TelnetSettings;
use guac_session::{Params, SessionCtx, SessionError};
use guac_terminal::{run_terminal, RemoteCmd, RemoteIo, TerminalConfig};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

/// Run a telnet session over an established byte stream.
pub async fn run_telnet<S>(
    ctx: SessionCtx,
    settings: TelnetSettings,
    params: Params,
    stream: S,
) -> Result<(), SessionError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (to_term, from_remote) = mpsc::channel::<Vec<u8>>(64);
    let (to_remote, mut cmd_rx) = mpsc::channel::<RemoteCmd>(64);

    let mut cfg = TerminalConfig::from_params(&params, ctx.tx.caps().optimal_dpi);
    cfg.start_immediately = !settings.waits_for_login();
    cfg.filter = settings.login_filter().map(|f| Box::new(f) as _);

    let terminal_type = settings.terminal_type.clone();
    let username = settings.username.clone();
    let io = tokio::spawn(async move {
        let (mut rd, mut wr) = tokio::io::split(stream);
        let mut dec = Decoder::new();
        let mut neg = Negotiator::new(&terminal_type, username.as_deref(), 80, 24);
        let mut buf = vec![0u8; 8192];
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    let res = match cmd {
                        RemoteCmd::Data(d) => {
                            let r = wr.write_all(&escape_data(&d)).await;
                            if neg.local_echo && to_term.send(d).await.is_err() {
                                break;
                            }
                            r
                        }
                        RemoteCmd::Resize { cols, rows, .. } => {
                            let bytes = neg.resize(cols, rows);
                            if bytes.is_empty() { Ok(()) } else { wr.write_all(&bytes).await }
                        }
                    };
                    if res.is_err() {
                        break;
                    }
                }
                n = rd.read(&mut buf) => {
                    let n = match n { Ok(0) | Err(_) => break, Ok(n) => n };
                    let mut reply = vec![];
                    for ev in dec.decode(&buf[..n]) {
                        let r = neg.handle(ev);
                        reply.extend(r.send);
                        if !r.data.is_empty() && to_term.send(r.data).await.is_err() {
                            return;
                        }
                    }
                    if !reply.is_empty() && wr.write_all(&reply).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = wr.shutdown().await;
        // dropping `to_term` tells the terminal the remote side is gone
    });

    let result = run_terminal(
        ctx,
        cfg,
        RemoteIo {
            from_remote,
            to_remote,
        },
    )
    .await;
    io.abort();
    result
}
