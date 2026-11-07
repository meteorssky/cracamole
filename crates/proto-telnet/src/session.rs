use crate::codec::{escape_data, Decoder};
use crate::negotiation::Negotiator;
use crate::settings::TelnetSettings;
use guac_session::{Params, SessionCtx, SessionError, Status};
use guac_terminal::{run_terminal, RemoteCmd, RemoteIo, TerminalConfig, TextEncoding};
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
    let encoding = TextEncoding::parse(params.get("character-encoding"))?;
    let (to_term, from_remote) = mpsc::channel::<Vec<u8>>(64);
    let (to_remote, mut cmd_rx) = mpsc::channel::<RemoteCmd>(64);

    let mut cfg = TerminalConfig::from_params(&params, ctx.tx.caps().optimal_dpi);
    cfg.start_immediately = !settings.waits_for_login();
    cfg.filter = settings.login_filter().map(|f| Box::new(f) as _);

    let terminal_type = settings.terminal_type.clone();
    let username = settings
        .username
        .as_deref()
        .map(|s| encoding.encode(s))
        .transpose()?;
    let backspace = cfg.options.backspace;
    let mut io = tokio::spawn(async move {
        let (mut rd, mut wr) = tokio::io::split(stream);
        let mut dec = Decoder::new();
        let mut neg = Negotiator::new(&terminal_type, None, 80, 24);
        neg.set_username_bytes(username);
        let mut text_decoder = encoding.decoder();
        let mut buf = vec![0u8; 8192];
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    let res = match cmd {
                        RemoteCmd::Data(d) => {
                            let encoded = encoding.encode_input(&d, backspace)?;
                            let r = wr.write_all(&escape_data(&encoded)).await;
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
                    let n = match n {
                        Ok(0) => break,
                        Err(error) => return Err(SessionError::plugin(Status::UpstreamUnavailable, format!("Telnet read failed: {error}"))),
                        Ok(n) => n,
                    };
                    let mut reply = vec![];
                    for ev in dec.decode(&buf[..n]) {
                        let r = neg.handle(ev);
                        reply.extend(r.send);
                        let text = text_decoder.decode(&r.data, false)?;
                        if !text.is_empty() && to_term.send(text).await.is_err() {
                            return Ok(());
                        }
                    }
                    if !reply.is_empty() && wr.write_all(&reply).await.is_err() {
                        break;
                    }
                }
            }
        }
        let tail = text_decoder.decode(&[], true)?;
        if !tail.is_empty() {
            let _ = to_term.send(tail).await;
        }
        let _ = wr.shutdown().await;
        Ok::<(), SessionError>(())
        // dropping `to_term` tells the terminal the remote side is gone
    });

    let tx = ctx.tx.clone();
    let mut terminal = Box::pin(run_terminal(
        ctx,
        cfg,
        RemoteIo {
            from_remote,
            to_remote,
        },
    ));
    let mut io_done = false;
    loop {
        tokio::select! {
            result = &mut terminal => {
                if result.is_ok() && !io_done && !tx.is_cancelled() {
                    if let Ok(Ok(Err(error))) = tokio::time::timeout(std::time::Duration::from_secs(2), &mut io).await {
                        return Err(error);
                    }
                }
                io.abort();
                return result;
            }
            result = &mut io, if !io_done => {
                io_done = true;
                match result {
                    Ok(Err(error)) => return Err(error),
                    Err(_) => return Err(SessionError::plugin(Status::ServerError, "Telnet task failed.")),
                    Ok(Ok(())) => {},
                }
            }
        }
    }
}
