//! Active VNC session: server framebuffer updates -> Guacamole display, client input -> RFB input.
//!
//! The update loop is pull-based: the next `FramebufferUpdateRequest` is sent only after the
//! previous update has been painted, flushed and acknowledged by the browser, which throttles a
//! fast server to what the client can display.

use crate::decode::{Decoder, Message};
use crate::handshake::{handshake, ServerInit};
use crate::messages;
use crate::screen::Screen;
use crate::settings::VncSettings;
use base64::Engine as _;
use guac_protocol::Instruction;
use guac_session::{connect_tcp, InputEvent, SessionCtx, SessionError, SessionTx, Status};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::Instant;

const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const HEARTBEAT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const LAG_WAIT: Duration = Duration::from_secs(10);

fn pack(size: (u32, u32)) -> u64 {
    (u64::from(size.0) << 32) | u64::from(size.1)
}
fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

pub async fn run_vnc(mut ctx: SessionCtx, settings: VncSettings) -> Result<(), SessionError> {
    let connecting = async {
        let mut stream =
            connect_tcp(&settings.hostname, settings.port, settings.timeout_secs).await?;
        tracing::info!("VNC TCP connection successful.");
        let init = handshake(&mut stream, &settings, crate::auth::random_bytes).await?;
        Ok::<_, SessionError>((stream, init))
    };
    let (stream, init) = tokio::select! {
        _ = ctx.tx.cancelled() => return Ok(()),
        result = tokio::time::timeout(Duration::from_secs(settings.timeout_secs), connecting) =>
            result.map_err(|_| SessionError::plugin(Status::UpstreamTimeout, "VNC handshake timed out."))??,
    };
    let (rd, wr) = stream.into_split();
    run_established(&mut ctx, &settings, init, rd, wr).await
}

/// Everything after the handshake; generic over the transport so tests can use in-memory pipes.
pub async fn run_established<R, W>(
    ctx: &mut SessionCtx,
    settings: &VncSettings,
    init: ServerInit,
    rd: R,
    mut wr: W,
) -> Result<(), SessionError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    tracing::info!("VNC session established ({}x{}).", init.width, init.height);
    let screen = Screen::new(init.width, init.height)?;
    let size = Arc::new(AtomicU64::new(pack(screen.size())));

    let mut hello = messages::set_pixel_format(&settings.format);
    hello.extend(messages::set_encodings());
    hello.extend(messages::update_request(false, init.width, init.height));
    write(&mut wr, &hello).await?;

    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(16);
    let server = server_loop(
        rd,
        Decoder::new(settings.format),
        screen,
        ctx.tx.clone(),
        out_tx,
        size.clone(),
        settings.clone(),
    );
    tokio::pin!(server);
    let mut heartbeat = tokio::time::interval_at(Instant::now() + HEARTBEAT, HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = ctx.tx.cancelled() => return Ok(()),
            result = &mut server => return result,
            Some(message) = out_rx.recv() => write(&mut wr, &message).await?,
            event = ctx.events.recv() => {
                let Some(event) = event else { return Ok(()) };
                let message = match event {
                    InputEvent::Key { keysym, pressed } if !settings.read_only => {
                        messages::key_event(keysym, pressed)
                    }
                    InputEvent::Mouse { x, y, mask } if !settings.read_only => {
                        messages::pointer_event(mask, x, y, unpack(size.load(Ordering::Relaxed)))
                    }
                    InputEvent::Clipboard(text)
                        if !settings.read_only && !settings.disable_paste =>
                    {
                        messages::cut_text(&text, settings.clipboard)
                    }
                    // The desktop keeps the server's size; the browser scales the display.
                    _ => continue,
                };
                write(&mut wr, &message).await?;
            }
            _ = heartbeat.tick() => ctx.tx.sync().await?,
        }
    }
}

async fn write<W: AsyncWrite + Unpin>(wr: &mut W, bytes: &[u8]) -> Result<(), SessionError> {
    let fail = |e: std::io::Error| {
        SessionError::plugin(
            Status::UpstreamUnavailable,
            format!("VNC write failed: {e}"),
        )
    };
    tokio::time::timeout(WRITE_TIMEOUT, async {
        wr.write_all(bytes).await?;
        wr.flush().await
    })
    .await
    .map_err(|_| SessionError::plugin(Status::UpstreamTimeout, "VNC write timed out."))?
    .map_err(fail)
}

/// Reads server messages until the connection ends; never returns `Ok`.
async fn server_loop<R: AsyncRead + Unpin>(
    mut rd: R,
    mut decoder: Decoder,
    mut screen: Screen,
    tx: SessionTx,
    out: mpsc::Sender<Vec<u8>>,
    size: Arc<AtomicU64>,
    settings: VncSettings,
) -> Result<(), SessionError> {
    let mut last_flush: Option<Instant> = None;
    loop {
        match decoder.read_message(&mut rd, &mut screen).await? {
            Message::Update { resized } => {
                if let Some(t) = last_flush {
                    tokio::time::sleep_until(t + FRAME_INTERVAL).await;
                }
                let frame = screen.display.flush();
                if !frame.is_empty() {
                    for ins in frame {
                        tx.send(ins).await?;
                    }
                    tx.sync().await?;
                    last_flush = Some(Instant::now());
                    tokio::select! {
                        r = tx.wait_lag(2) => r?,
                        _ = tokio::time::sleep(LAG_WAIT) => {
                            tracing::warn!("client did not acknowledge frames for {LAG_WAIT:?}; continuing");
                        }
                    }
                }
                let (w, h) = screen.size();
                size.store(pack((w, h)), Ordering::Relaxed);
                // After a resize the server has nothing queued for the new geometry: ask for all of it.
                out.send(messages::update_request(!resized, w, h))
                    .await
                    .map_err(|_| SessionError::Closed)?;
            }
            Message::CutText(Some(bytes)) if !settings.disable_copy => {
                send_clipboard(&tx, &messages::clipboard_text(&bytes, settings.clipboard)).await?;
            }
            Message::CutText(_) | Message::Bell | Message::ColourMap => {}
        }
    }
}

/// Send remote clipboard text to the Guacamole client (3-byte aligned chunks keep base64 unpadded).
async fn send_clipboard(tx: &SessionTx, text: &str) -> Result<(), SessionError> {
    static NEXT: AtomicU64 = AtomicU64::new(0x4000_0000);
    let id = (NEXT.fetch_add(1, Ordering::Relaxed) & 0x7fff_ffff).to_string();
    tx.send(Instruction::new(
        "clipboard",
        [id.clone(), "text/plain".into()],
    ))
    .await?;
    for chunk in text.as_bytes().chunks(3072) {
        let b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
        tx.send(Instruction::new("blob", [id.clone(), b64])).await?;
    }
    tx.send(Instruction::new("end", [id])).await
}
