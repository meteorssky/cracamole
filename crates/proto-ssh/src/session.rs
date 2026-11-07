use crate::hostkey::{self, Entry, Verdict};
use crate::keys::{load_private_key, KeyError};
use crate::prompt::Prompter;
use crate::settings::{SshSettings, SYSTEM_KNOWN_HOSTS};
use guac_protocol::Status;
use guac_session::{connect_tcp, Params, SessionCtx, SessionError};
use guac_terminal::{run_terminal, RemoteCmd, RemoteIo, TerminalConfig, TextEncoding};
use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKeyBase64};
use russh::{ChannelMsg, Disconnect, MethodKind, Pty};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

const CLEAR_SCREEN: &[u8] = b"\x1b[H\x1b[J";

fn err(status: Status, msg: impl Into<String>) -> SessionError {
    SessionError::plugin(status, msg)
}

/// Host key verification handler (outcomes follow C `guac_common_ssh_verify_host_key`).
struct Client {
    host: String,
    port: u16,
    entries: Vec<Entry>,
    failure: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        if self.entries.is_empty() {
            *self.failure.lock().unwrap() = Some(
                "SSH host key is unknown. Verify and save the server host key before connecting."
                    .into(),
            );
            return Ok(false);
        }
        let blob = match key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => key.public_key_bytes(),
            _ => {
                *self.failure.lock().unwrap() =
                    Some("Server presented a certificate, which is not supported.".into());
                return Ok(false);
            }
        };
        match hostkey::verify(&self.entries, &self.host, self.port, &blob) {
            Verdict::Match => {
                tracing::debug!("Host key match found for {}", self.host);
                Ok(true)
            }
            v => {
                tracing::error!("Host key {:?} for {}", v, self.host);
                *self.failure.lock().unwrap() =
                    Some("Host key did not match any provided known host keys.".into());
                Ok(false)
            }
        }
    }
}

pub(crate) fn load_known_hosts(settings: &SshSettings) -> Result<Vec<Entry>, SessionError> {
    let text = match &settings.host_key {
        Some(k) => Some(k.clone()),
        None => match std::fs::read_to_string(SYSTEM_KNOWN_HOSTS) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(err(
                    Status::ServerError,
                    format!("Unable to read known hosts: {e}"),
                ))
            }
        },
    };
    match text {
        Some(t) => {
            let (entries, errors) = hostkey::parse(&t);
            if !errors.is_empty() || (entries.is_empty() && settings.host_key.is_some()) {
                return Err(err(
                    Status::ServerError,
                    "Configured host keys contain no usable keys.",
                ));
            }
            for e in errors {
                tracing::warn!("host key: {e}");
            }
            Ok(entries)
        }
        None => Ok(vec![]),
    }
}

/// Everything the SSH task needs from the terminal side.
struct Io {
    prompter: Prompter,
}

async fn authenticate(
    handle: &mut client::Handle<Client>,
    settings: &SshSettings,
    username: &str,
    key: Option<Arc<PrivateKey>>,
    io: &mut Io,
) -> Result<bool, SessionError> {
    let unauthorized = |m: String| err(Status::ClientUnauthorized, m);

    let none = handle.authenticate_none(username).await.map_err(|e| {
        err(
            Status::UpstreamError,
            format!("SSH authentication failed: {e}"),
        )
    })?;
    let remaining = match none {
        client::AuthResult::Success => {
            tracing::debug!("SSH NONE authentication succeeded.");
            return Ok(false);
        }
        client::AuthResult::Failure {
            remaining_methods, ..
        } => remaining_methods,
    };
    tracing::debug!("Supported authentication methods: {:?}", &*remaining);

    if let Some(key) = key {
        if !remaining.contains(&MethodKind::PublicKey) {
            return Err(unauthorized(
                "Public key authentication is not supported by the SSH server".into(),
            ));
        }
        let hash = handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten();
        let r = handle
            .authenticate_publickey(username, PrivateKeyWithHashAlg::new(key, hash))
            .await
            .map_err(|e| unauthorized(format!("Public key authentication failed: {e}")))?;
        if !r.success() {
            return Err(unauthorized("Public key authentication failed".into()));
        }
        return Ok(false);
    }

    let mut prompted = false;
    let mut password = settings.password.clone();
    if password.is_none() {
        prompted = true;
        match io.prompter.prompt("Password: ", false).await {
            Some(p) => password = Some(crate::settings::Secret::new(p)),
            None => {
                return Err(unauthorized(
                    "SSH authentication requires either a private key or a password.".into(),
                ))
            }
        }
    }
    let password = password.expect("set above");

    if remaining.contains(&MethodKind::Password) {
        let r = handle
            .authenticate_password(username, password.expose())
            .await
            .map_err(|e| unauthorized(format!("Password authentication failed: {e}")))?;
        if !r.success() {
            return Err(unauthorized("Password authentication failed".into()));
        }
        return Ok(prompted);
    }

    if remaining.contains(&MethodKind::KeyboardInteractive) {
        let mut resp = handle
            .authenticate_keyboard_interactive_start(username, None)
            .await
            .map_err(|e| {
                unauthorized(format!("Keyboard-interactive authentication failed: {e}"))
            })?;
        for _ in 0..10 {
            match resp {
                KeyboardInteractiveAuthResponse::Success => return Ok(prompted),
                KeyboardInteractiveAuthResponse::Failure { .. } => {
                    return Err(unauthorized(
                        "Keyboard-interactive authentication failed".into(),
                    ))
                }
                KeyboardInteractiveAuthResponse::InfoRequest {
                    instructions,
                    prompts,
                    ..
                } => {
                    if !instructions.trim().is_empty() {
                        io.prompter
                            .show(&format!("{}\n", instructions.trim()))
                            .await;
                        prompted = true;
                    }
                    let mut answers = vec![];
                    for (i, p) in prompts.iter().enumerate() {
                        // A single prompt is the password prompt; further prompts go to the user.
                        if prompts.len() == 1 && i == 0 {
                            answers.push(password.expose().to_string());
                        } else {
                            prompted = true;
                            let a = io
                                .prompter
                                .prompt(&p.prompt, p.echo)
                                .await
                                .unwrap_or_default();
                            answers.push(a);
                        }
                    }
                    resp = handle
                        .authenticate_keyboard_interactive_respond(answers)
                        .await
                        .map_err(|e| {
                            unauthorized(format!("Keyboard-interactive authentication failed: {e}"))
                        })?;
                }
            }
        }
        return Err(unauthorized(
            "Keyboard-interactive authentication failed: too many rounds".into(),
        ));
    }

    Err(unauthorized(
        "Password and keyboard-interactive authentication are not supported by the SSH server"
            .into(),
    ))
}

/// Wait for the reply to a channel request, forwarding any early output.
async fn await_reply(
    ch: &mut russh::Channel<client::Msg>,
    early: &mut Vec<(bool, Vec<u8>)>,
) -> Result<bool, SessionError> {
    loop {
        match ch.wait().await {
            Some(ChannelMsg::Success) => return Ok(true),
            Some(ChannelMsg::Failure) => return Ok(false),
            Some(ChannelMsg::Data { data }) => early.push((false, data.to_vec())),
            Some(ChannelMsg::ExtendedData { data, .. }) => early.push((true, data.to_vec())),
            Some(_) => {}
            None => {
                return Err(err(
                    Status::UpstreamError,
                    "SSH channel closed unexpectedly.",
                ))
            }
        }
    }
}

async fn ssh_task(
    settings: SshSettings,
    encoding: TextEncoding,
    to_term: mpsc::Sender<Vec<u8>>,
    cmd_rx: mpsc::Receiver<RemoteCmd>,
) -> Result<(), SessionError> {
    let mut io = Io {
        prompter: Prompter::new(to_term.clone(), cmd_rx),
    };

    // The terminal announces its size first; wait briefly so the PTY gets the right dimensions.
    // (Anything typed before the connection exists is ignored.)
    // `Prompter::prompt` also records later Resize messages.
    let mut size = (80u16, 24u16, 0u32, 0u32);
    {
        // Drain the initial Resize without blocking if it is already queued.
        let (to_term_, mut rx, s) = std::mem::replace(
            &mut io.prompter,
            Prompter::new(to_term.clone(), mpsc::channel(1).1),
        )
        .into_parts();
        if let Ok(Some(RemoteCmd::Resize {
            cols,
            rows,
            width_px,
            height_px,
        })) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await
        {
            size = (cols, rows, width_px, height_px);
        } else if let Some(s) = s {
            size = s;
        }
        io.prompter = Prompter::new(to_term_, rx);
    }

    // ---- credentials (before connecting, as in C) ----
    let mut prompted = false;
    let username = match &settings.username {
        Some(u) => u.clone(),
        None => {
            prompted = true;
            io.prompter
                .prompt("Login as: ", true)
                .await
                .ok_or_else(|| {
                    err(
                        Status::ClientUnauthorized,
                        "SSH authentication requires a username.",
                    )
                })?
        }
    };

    let mut key: Option<Arc<PrivateKey>> = None;
    if let Some(pem) = &settings.private_key {
        tracing::debug!("Attempting private key import (WITHOUT passphrase)");
        let first = match &settings.passphrase {
            // A supplied passphrase is used directly on the first attempt too; harmless for unencrypted keys.
            Some(p) => load_private_key(pem.expose(), Some(p.expose())),
            None => load_private_key(pem.expose(), None),
        };
        let loaded = match first {
            Ok(k) => k,
            Err(KeyError::NeedsPassphrase) | Err(KeyError::Invalid(_))
                if settings.passphrase.is_none() =>
            {
                tracing::debug!(
                    "Initial import failed; re-attempting private key import (WITH passphrase)"
                );
                prompted = true;
                let pass = io
                    .prompter
                    .prompt("Key passphrase: ", false)
                    .await
                    .ok_or_else(|| {
                        err(
                            Status::ClientUnauthorized,
                            "Auth key import failed: no passphrase",
                        )
                    })?;
                load_private_key(pem.expose(), Some(&pass)).map_err(|e| {
                    err(
                        Status::ClientUnauthorized,
                        format!("Auth key import failed: {e:?}"),
                    )
                })?
            }
            Err(e) => {
                return Err(err(
                    Status::ClientUnauthorized,
                    format!("Auth key import failed: {e:?}"),
                ))
            }
        };
        tracing::info!("Auth key successfully imported.");
        key = Some(Arc::new(loaded));
    }
    if prompted {
        let _ = to_term.send(CLEAR_SCREEN.to_vec()).await;
    }

    // ---- connect + host key check ----
    let tcp = connect_tcp(&settings.hostname, settings.port, settings.timeout_secs).await?;
    let failure = Arc::new(Mutex::new(None));
    let handler = Client {
        host: settings.hostname.clone(),
        port: settings.port,
        entries: load_known_hosts(&settings)?,
        failure: failure.clone(),
    };
    let mut config = client::Config::default();
    if settings.server_alive_interval > 0 {
        config.keepalive_interval = Some(Duration::from_secs(settings.server_alive_interval));
        config.keepalive_max = 3;
    }
    let handshake = tokio::time::timeout(
        Duration::from_secs(settings.timeout_secs.max(1)),
        client::connect_stream(Arc::new(config), tcp, handler),
    )
    .await;
    let mut handle = match handshake {
        Err(_) => return Err(err(Status::UpstreamTimeout, "SSH handshake timed out.")),
        Ok(Ok(h)) => h,
        Ok(Err(e)) => {
            if let Some(m) = failure.lock().unwrap().take() {
                return Err(err(Status::UpstreamError, m));
            }
            return Err(err(
                Status::UpstreamError,
                format!("SSH handshake failed: {e}"),
            ));
        }
    };

    // ---- authenticate ----
    if authenticate(&mut handle, &settings, &username, key, &mut io).await? {
        let _ = to_term.send(CLEAR_SCREEN.to_vec()).await;
    }
    tracing::info!("SSH authentication successful.");

    // ---- terminal channel ----
    let mut channel = handle.channel_open_session().await.map_err(|e| {
        err(
            Status::UpstreamError,
            format!("Unable to open terminal channel: {e}"),
        )
    })?;
    let mut early: Vec<(bool, Vec<u8>)> = vec![];

    if let Some(tz) = &settings.timezone {
        if channel.set_env(false, "TZ", tz.as_str()).await.is_err() {
            tracing::warn!(
                "Unable to set the timezone: SSH server refused to set \"TZ\" variable."
            );
        }
    }
    channel
        .request_pty(
            true,
            &settings.terminal_type,
            size.0 as u32,
            size.1 as u32,
            0,
            0,
            &[(Pty::VERASE, settings.backspace as u32)],
        )
        .await
        .map_err(|e| {
            err(
                Status::UpstreamError,
                format!("Unable to allocate PTY: {e}"),
            )
        })?;
    if !await_reply(&mut channel, &mut early).await? {
        return Err(err(Status::UpstreamError, "Unable to allocate PTY."));
    }
    if let Some(locale) = &settings.locale {
        if channel
            .set_env(false, "LANG", locale.as_str())
            .await
            .is_err()
        {
            tracing::warn!("Unable to forward locale: SSH server refused to set \"LANG\" environment variable.");
        }
    }
    match &settings.command {
        Some(cmd) => {
            channel.exec(true, cmd.as_str()).await.map_err(|e| {
                err(
                    Status::UpstreamError,
                    format!("Unable to execute command: {e}"),
                )
            })?;
            if !await_reply(&mut channel, &mut early).await? {
                return Err(err(Status::UpstreamError, "Unable to execute command."));
            }
        }
        None => {
            channel.request_shell(true).await.map_err(|e| {
                err(
                    Status::UpstreamError,
                    format!("Unable to associate shell with PTY: {e}"),
                )
            })?;
            if !await_reply(&mut channel, &mut early).await? {
                return Err(err(
                    Status::UpstreamError,
                    "Unable to associate shell with PTY.",
                ));
            }
        }
    }
    // stdout and extended-data must not complete one another's partial multibyte character.
    let mut stdout = encoding.decoder();
    let mut stderr = encoding.decoder();
    for (extended, bytes) in early {
        let decoder = if extended { &mut stderr } else { &mut stdout };
        let decoded = decoder.decode(&bytes, false)?;
        if !decoded.is_empty() && to_term.send(decoded).await.is_err() {
            return Ok(());
        }
    }

    // ---- pump ----
    let (_, mut cmd_rx, _) = io.prompter.into_parts();
    loop {
        tokio::select! {
            biased;
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                let r = match cmd {
                    RemoteCmd::Data(d) => channel.data_bytes(encoding.encode_input(&d, settings.backspace)?).await,
                    RemoteCmd::Resize { cols, rows, width_px, height_px } => {
                        channel.window_change(cols as u32, rows as u32, width_px, height_px).await
                    }
                };
                if r.is_err() { break; }
            }
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        let text = stdout.decode(&data, false)?;
                        if !text.is_empty() && to_term.send(text).await.is_err() { break; }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        let text = stderr.decode(&data, false)?;
                        if !text.is_empty() && to_term.send(text).await.is_err() { break; }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        tracing::debug!("remote exit status {exit_status}");
                    }
                    Some(ChannelMsg::Close) | None => break,
                    Some(_) => {}
                }
            }
        }
    }
    for decoder in [&mut stdout, &mut stderr] {
        let tail = decoder.decode(&[], true)?;
        if !tail.is_empty() {
            let _ = to_term.send(tail).await;
        }
    }
    let _ = handle
        .disconnect(Disconnect::ByApplication, "", "English")
        .await;
    Ok(())
}

/// Run an SSH terminal session.
pub async fn run_ssh(
    ctx: SessionCtx,
    settings: SshSettings,
    params: Params,
) -> Result<(), SessionError> {
    let encoding = TextEncoding::parse(params.get("character-encoding"))?;
    let (to_term, from_remote) = mpsc::channel::<Vec<u8>>(64);
    let (to_remote, cmd_rx) = mpsc::channel::<RemoteCmd>(64);

    let mut cfg = TerminalConfig::from_params(&params, ctx.tx.caps().optimal_dpi);
    cfg.options.backspace = settings.backspace;

    let tx = ctx.tx.clone();
    let mut io = tokio::spawn(ssh_task(settings, encoding, to_term, cmd_rx));
    let mut term = Box::pin(run_terminal(
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
            r = &mut term => {
                // The terminal also ends cleanly when the SSH task simply dropped its channel,
                // e.g. because authentication failed. In that case the SSH task's error is the
                // real outcome and must win (it is racing with this branch).
                if r.is_ok() && !io_done && !tx.is_cancelled() {
                    if let Ok(Ok(Err(e))) =
                        tokio::time::timeout(Duration::from_secs(2), &mut io).await
                    {
                        return Err(e);
                    }
                }
                io.abort();
                return r;
            }
            r = &mut io, if !io_done => {
                io_done = true;
                match r {
                    Ok(Err(e)) => return Err(e),
                    Ok(Ok(())) => {} // remote closed: let the terminal flush its final frame
                    Err(j) if j.is_panic() => return Err(err(Status::ServerError, "Internal error.")),
                    Err(_) => {}
                }
            }
        }
    }
}
