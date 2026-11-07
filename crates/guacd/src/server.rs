//! Accept loop, `select` routing and per-connection session lifecycle.

use guac_protocol::Instruction;
use guac_session::{
    handshake, new_connection_id, run_session, InstructionReader, ProtocolPlugin, SessionConfig,
    SessionError, Status, HANDSHAKE_TIMEOUT,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Protocol plugins by name. Built-ins are enabled by cargo features.
#[derive(Default)]
pub struct Registry {
    plugins: HashMap<String, Arc<dyn ProtocolPlugin>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, plugin: impl ProtocolPlugin) -> Self {
        self.plugins
            .insert(plugin.name().to_string(), Arc::new(plugin));
        self
    }

    /// All protocols compiled into this binary.
    pub fn builtin() -> Self {
        let r = Self::new();
        #[cfg(feature = "telnet")]
        let r = r.with(proto_telnet::TelnetPlugin);
        #[cfg(feature = "ssh")]
        let r = r.with(proto_ssh::SshPlugin);
        #[cfg(feature = "rdp")]
        let r = r.with(proto_rdp::RdpPlugin);
        #[cfg(feature = "vnc")]
        let r = r.with(proto_vnc::VncPlugin);
        r
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ProtocolPlugin>> {
        self.plugins.get(name).cloned()
    }

    pub fn names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.plugins.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }
}

#[derive(Clone)]
pub struct ServerConfig {
    /// Concurrent connections accepted; beyond this new sockets are closed immediately.
    pub max_connections: usize,
    /// Time allowed for the client to send `select` (C: 15 s).
    pub select_timeout: Duration,
    /// Upper bound for select + whole handshake, against clients that dribble instructions.
    pub handshake_total: Duration,
    pub session: SessionConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_connections: 4096,
            select_timeout: HANDSHAKE_TIMEOUT,
            handshake_total: Duration::from_secs(60),
            session: SessionConfig::default(),
        }
    }
}

async fn send_error<W: AsyncWrite + Unpin>(w: &mut W, message: &str, status: Status) {
    let ins = Instruction::new("error", [message.to_string(), status.code().to_string()]);
    let _ = w.write_all(&ins.encode()).await;
    let _ = w.flush().await;
}

/// Serve TCP clients until `shutdown` fires, then stop accepting and let sessions end gracefully.
pub async fn serve(
    listener: TcpListener,
    registry: Arc<Registry>,
    cfg: ServerConfig,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let acceptor = tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            loop {
                let accepted = tokio::select! {
                    _ = shutdown.cancelled() => break,
                    accepted = listener.accept() => accepted,
                };
                match accepted {
                    Ok((stream, peer)) => {
                        let _ = stream.set_nodelay(true);
                        if tx.send((stream, peer.to_string())).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("accept failed: {e}");
                        // Avoid a hot loop on persistent errors such as fd exhaustion.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        }
    });
    let result = serve_streams(rx, registry, cfg, shutdown).await;
    acceptor.abort();
    result
}

/// Serve already-accepted streams (for example in-memory pipes, so no socket is opened at all)
/// until `shutdown` fires or the sender side is dropped. Each item carries a peer label for logs.
pub async fn serve_streams<S>(
    mut incoming: tokio::sync::mpsc::Receiver<(S, String)>,
    registry: Arc<Registry>,
    cfg: ServerConfig,
    shutdown: CancellationToken,
) -> std::io::Result<()>
where
    S: tokio::io::AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let limit = Arc::new(Semaphore::new(cfg.max_connections.max(1)));
    let mut tasks: JoinSet<()> = JoinSet::new();
    let mut session_cfg = cfg.session.clone();
    session_cfg.shutdown = Some(shutdown.clone());
    let cfg = ServerConfig {
        session: session_cfg,
        ..cfg
    };

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            // Reap finished tasks so the set does not grow without bound.
            Some(done) = tasks.join_next(), if !tasks.is_empty() => {
                if let Err(e) = done {
                    if e.is_panic() {
                        tracing::error!("connection task panicked");
                    }
                }
            }
            received = incoming.recv() => {
                let Some((stream, peer)) = received else { break };
                let Ok(permit) = limit.clone().try_acquire_owned() else {
                    tracing::warn!("connection limit reached; rejecting {peer}");
                    drop(stream);
                    continue;
                };
                let registry = registry.clone();
                let cfg = cfg.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    handle_connection(stream, registry, cfg).await;
                });
            }
        }
    }

    // Graceful drain: sessions observe the shutdown token themselves.
    let grace = cfg.session.shutdown_grace + Duration::from_secs(2);
    let drain = async { while tasks.join_next().await.is_some() {} };
    if tokio::time::timeout(grace, drain).await.is_err() {
        tracing::warn!("sessions did not finish within the grace period; aborting them");
        tasks.abort_all();
    }
    Ok(())
}

async fn handle_connection<S>(stream: S, registry: Arc<Registry>, cfg: ServerConfig)
where
    S: tokio::io::AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = InstructionReader::new(r);

    // ---- select + handshake, bounded in total ----
    let routed = tokio::time::timeout(cfg.handshake_total, async {
        let select = match tokio::time::timeout(cfg.select_timeout, reader.read()).await {
            Err(_) => return Err("timed out waiting for \"select\"".to_string()),
            Ok(Err(e)) => return Err(format!("error reading \"select\": {e}")),
            Ok(Ok(None)) => return Err("connection closed before \"select\"".to_string()),
            Ok(Ok(Some(i))) => i,
        };
        if select.opcode != "select" {
            return Err(format!("expected \"select\", got \"{}\"", select.opcode));
        }
        if select.args.len() != 1 {
            return Err(format!(
                "bad number of arguments to \"select\" ({})",
                select.args.len()
            ));
        }
        let identifier = &select.args[0];

        // Joining an existing connection (identifiers starting with '$') is not supported yet.
        if identifier.starts_with('$') {
            tracing::info!("Connection \"{identifier}\" does not exist");
            send_error(&mut w, "No such connection.", Status::ResourceNotFound).await;
            return Ok(None);
        }

        let Some(plugin) = registry.get(identifier) else {
            tracing::warn!("Unable to load client plugin for protocol \"{identifier}\"");
            send_error(&mut w, "Aborted. See logs.", Status::Unsupported).await;
            return Ok(None);
        };
        tracing::info!("Creating new client for protocol \"{identifier}\"");

        let id = new_connection_id();
        match handshake(
            &mut reader,
            &mut w,
            plugin.arg_names(),
            &id,
            cfg.select_timeout,
        )
        .await
        {
            Ok(hs) => Ok(Some((plugin, hs, id))),
            Err(e) => Err(format!("Guacamole handshake failed: {e}")),
        }
    })
    .await;

    let (plugin, hs, id) = match routed {
        Err(_) => {
            tracing::warn!("handshake did not complete in {:?}", cfg.handshake_total);
            return;
        }
        Ok(Err(msg)) => {
            tracing::warn!("{msg}");
            return;
        }
        Ok(Ok(None)) => return,
        Ok(Ok(Some(v))) => v,
    };

    tracing::info!("Connection ID is \"{id}\"");
    let result = run_session(reader, w, hs, plugin, cfg.session.clone()).await;
    match result {
        Ok(()) => {}
        Err(SessionError::Closed) => tracing::debug!("user disconnected"),
        Err(e) => tracing::info!("Connection \"{id}\" ended with error: {e}"),
    }
    tracing::info!("Connection \"{id}\" removed.");
}
