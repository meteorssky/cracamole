use crate::clipboard::Clipboards;
use crate::{ClientCaps, Handshake, InstructionReader, ProtocolPlugin, SessionError};
use base64::Engine;
use guac_protocol::{Instruction, Status};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Bound on queued outgoing instructions. When full, `SessionTx::send` waits (backpressure).
    pub out_queue: usize,
    /// How long plugins and writers get to stop after cancellation before being aborted.
    pub shutdown_grace: Duration,
    /// Maximum clipboard payload accepted from the client, in bytes.
    pub max_clipboard: usize,
    /// When cancelled (server shutdown) the session ends gracefully with a `disconnect`.
    pub shutdown: Option<CancellationToken>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            out_queue: 1024,
            shutdown_grace: Duration::from_secs(5),
            max_clipboard: 256 * 1024,
            shutdown: None,
        }
    }
}

/// Input from the client, already decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Mouse { x: i32, y: i32, mask: u32 },
    Key { keysym: u32, pressed: bool },
    Resize { width: u32, height: u32 },
    Clipboard(String),
}

struct SyncState {
    start: Instant,
    sent: AtomicU64,
    acked: watch::Sender<u64>,
}

/// Cloneable handle for sending to the client.
#[derive(Clone)]
pub struct SessionTx {
    out: mpsc::Sender<Instruction>,
    sync: Arc<SyncState>,
    cancel: CancellationToken,
    caps: Arc<ClientCaps>,
}

impl SessionTx {
    pub fn caps(&self) -> &ClientCaps {
        &self.caps
    }

    /// Queue an instruction. Waits while the output queue is full.
    pub async fn send(&self, ins: Instruction) -> Result<(), SessionError> {
        self.out.send(ins).await.map_err(|_| SessionError::Closed)
    }

    /// Emit a `sync` frame boundary and remember it for lag accounting.
    pub async fn sync(&self) -> Result<(), SessionError> {
        let ts = self.sync.start.elapsed().as_millis().to_string();
        self.sync.sent.fetch_add(1, Ordering::SeqCst);
        self.send(Instruction::new("sync", [ts])).await
    }

    /// Wait until at most `max_unacked` of the sent `sync` frames are still unanswered by the client.
    pub async fn wait_lag(&self, max_unacked: u64) -> Result<(), SessionError> {
        let sent = self.sync.sent.load(Ordering::SeqCst);
        let mut rx = self.sync.acked.subscribe();
        loop {
            let acked = *rx.borrow_and_update();
            if sent.saturating_sub(acked) <= max_unacked {
                return Ok(());
            }
            tokio::select! {
                r = rx.changed() => if r.is_err() { return Err(SessionError::Closed) },
                _ = self.cancel.cancelled() => return Err(SessionError::Closed),
            }
        }
    }

    pub async fn cancelled(&self) {
        self.cancel.cancelled().await
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// What a plugin receives: a send handle plus the stream of client input.
pub struct SessionCtx {
    pub tx: SessionTx,
    pub events: mpsc::Receiver<InputEvent>,
}

fn ack(stream: &str, status: Status) -> Instruction {
    Instruction::new(
        "ack",
        [
            stream.to_string(),
            "OK".to_string(),
            status.code().to_string(),
        ],
    )
}

async fn read_loop<R: AsyncRead + Unpin>(
    mut reader: InstructionReader<R>,
    events: mpsc::Sender<InputEvent>,
    out: mpsc::Sender<Instruction>,
    sync: Arc<SyncState>,
    max_clipboard: usize,
) -> Result<(), SessionError> {
    let mut clipboards = Clipboards::new(max_clipboard);
    let b64 = base64::engine::general_purpose::STANDARD;
    loop {
        let next = match clipboards.deadline() {
            Some(deadline) => tokio::time::timeout_at(deadline, reader.read())
                .await
                .map_err(|_| {
                    SessionError::plugin(Status::ClientTimeout, "Clipboard stream expired.")
                })??,
            None => reader.read().await?,
        };
        let Some(ins) = next else {
            return Ok(());
        };
        let int = |i: usize| ins.args.get(i).and_then(|s| s.parse::<i64>().ok());
        let ev = match ins.opcode.as_str() {
            "disconnect" => return Ok(()),
            "sync" => {
                sync.acked.send_modify(|a| *a += 1);
                None
            }
            "mouse" => match (int(0), int(1), int(2)) {
                (Some(x), Some(y), Some(m)) => Some(InputEvent::Mouse {
                    x: x as i32,
                    y: y as i32,
                    mask: m as u32,
                }),
                _ => None, // malformed: ignored, not fatal
            },
            "key" => match (int(0), int(1)) {
                (Some(k), Some(p)) if k >= 0 => Some(InputEvent::Key {
                    keysym: k as u32,
                    pressed: p != 0,
                }),
                _ => None,
            },
            "size" => match (int(0), int(1)) {
                (Some(w), Some(h)) if w > 0 && h > 0 => Some(InputEvent::Resize {
                    width: w as u32,
                    height: h as u32,
                }),
                _ => None,
            },
            "clipboard" => {
                if let Some(id) = ins.args.first() {
                    clipboards.open(id, ins.args.get(1).map(String::as_str))?;
                    let _ = out.send(ack(id, Status::Success)).await;
                }
                None
            }
            "blob" => {
                if let (Some(id), Some(data)) = (ins.args.first(), ins.args.get(1)) {
                    let bytes = b64.decode(data).map_err(|_| {
                        SessionError::plugin(Status::ClientBadRequest, "Invalid clipboard base64.")
                    })?;
                    clipboards.append(id, &bytes)?;
                    let _ = out.send(ack(id, Status::Success)).await;
                }
                None
            }
            "end" => match ins.args.first() {
                Some(id) => {
                    let text = clipboards.finish(id)?;
                    let _ = out.send(ack(id, Status::Success)).await;
                    Some(InputEvent::Clipboard(text))
                }
                None => None,
            },
            "nop" | "ack" => None,
            other => {
                tracing::debug!(opcode = other, "ignoring unsupported input instruction");
                None
            }
        };
        if let Some(ev) = ev {
            if events.send(ev).await.is_err() {
                // Plugin no longer listens to events; keep draining input for sync/disconnect.
                continue;
            }
        }
    }
}

fn client_message(e: &SessionError) -> String {
    match e {
        // C `guac_client_abort` tells the client only "Aborted. See logs."; details stay in the server log.
        SessionError::Plugin { .. } => "Aborted. See logs.".into(),
        SessionError::Timeout => "User is not responding.".into(),
        SessionError::Protocol(_) | SessionError::Handshake(_) => "Protocol violation.".into(),
        SessionError::Io(_) | SessionError::Closed => "Internal error.".into(),
    }
}

/// Drive one connection after a successful handshake until either side ends it.
///
/// Guarantees:
/// * a plugin panic is contained and reported as an `error` to this client only;
/// * client disconnect cancels the plugin, which is aborted after `shutdown_grace`;
/// * clean end sends `disconnect`; failure sends `error` then `disconnect`.
pub async fn run_session<R, W>(
    reader: InstructionReader<R>,
    mut writer: W,
    hs: Handshake,
    plugin: Arc<dyn ProtocolPlugin>,
    cfg: SessionConfig,
) -> Result<(), SessionError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let cancel = CancellationToken::new();
    let (out_tx, mut out_rx) = mpsc::channel::<Instruction>(cfg.out_queue.max(1));
    let (ev_tx, ev_rx) = mpsc::channel::<InputEvent>(256);
    let (ack_tx, _) = watch::channel(0u64);
    let sync = Arc::new(SyncState {
        start: Instant::now(),
        sent: AtomicU64::new(0),
        acked: ack_tx,
    });

    let tx = SessionTx {
        out: out_tx.clone(),
        sync: sync.clone(),
        cancel: cancel.clone(),
        caps: Arc::new(hs.caps),
    };
    let ctx = SessionCtx { tx, events: ev_rx };

    let mut writer_h = tokio::spawn(async move {
        let mut buf = Vec::new();
        while let Some(ins) = out_rx.recv().await {
            buf.clear();
            ins.encode_into(&mut buf);
            while buf.len() < 64 * 1024 {
                match out_rx.try_recv() {
                    Ok(more) => more.encode_into(&mut buf),
                    Err(_) => break,
                }
            }
            writer.write_all(&buf).await?;
            writer.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    });

    let mut reader_h = tokio::spawn(read_loop(
        reader,
        ev_tx,
        out_tx.clone(),
        sync,
        cfg.max_clipboard,
    ));
    let mut plugin_h = tokio::spawn(plugin.run(ctx, hs.params));

    let report = |e: &SessionError| {
        tracing::warn!(status = e.status().code(), "session aborted: {e}");
        Instruction::new("error", [client_message(e), e.status().code().to_string()])
    };
    let disconnect = || Instruction::new("disconnect", Vec::<String>::new());

    let result: Result<(), SessionError>;
    tokio::select! {
        r = &mut plugin_h => {
            let first = match r {
                Ok(r) => r,
                Err(j) if j.is_panic() => {
                    tracing::error!("protocol plugin panicked");
                    Err(SessionError::plugin(Status::ServerError, "Internal error."))
                }
                Err(_) => Err(SessionError::Closed),
            };
            // The plugin may only have ended because the reader failed first (its event
            // channel closed); in that case the reader's error is the real cause.
            result = if first.is_ok() && reader_h.is_finished() {
                match (&mut reader_h).await {
                    Ok(Err(e)) => Err(e),
                    _ => first,
                }
            } else {
                first
            };
            if let Err(e) = &result {
                let _ = out_tx.send(report(e)).await;
            }
            let _ = out_tx.send(disconnect()).await;
        }
        _ = async {
            match &cfg.shutdown {
                Some(t) => t.cancelled().await,
                None => std::future::pending().await,
            }
        } => {
            cancel.cancel();
            if tokio::time::timeout(cfg.shutdown_grace, &mut plugin_h).await.is_err() {
                plugin_h.abort();
            }
            let _ = out_tx.send(disconnect()).await;
            result = Ok(());
        }
        r = &mut reader_h => {
            result = match r {
                Ok(r) => r,
                Err(_) => Err(SessionError::Closed),
            };
            cancel.cancel();
            if tokio::time::timeout(cfg.shutdown_grace, &mut plugin_h).await.is_err() {
                plugin_h.abort();
            }
            if let Err(e) = &result {
                if matches!(e, SessionError::Protocol(_) | SessionError::Timeout | SessionError::Plugin { .. }) {
                    let _ = out_tx.send(report(e)).await;
                }
            }
            let _ = out_tx.send(disconnect()).await;
        }
    }

    cancel.cancel();
    reader_h.abort();
    drop(out_tx);
    if tokio::time::timeout(cfg.shutdown_grace, &mut writer_h)
        .await
        .is_err()
    {
        writer_h.abort();
    }
    result
}
