//! Glue between a [`Terminal`] and a Guacamole session: input events in, drawing instructions out.

use crate::colors::parse_color_scheme;
use crate::font::FontSet;
use crate::terminal::{Terminal, TerminalOptions};
use base64::Engine;
use guac_display::Display;
use guac_protocol::{Instruction, Status};
use guac_session::{InputEvent, Params, SessionCtx, SessionError};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

/// Commands from the terminal toward the remote connection task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteCmd {
    /// Bytes typed/pasted by the user, or emulator replies.
    Data(Vec<u8>),
    /// The visible grid changed size (also pixel size, for protocols that report it).
    Resize {
        cols: u16,
        rows: u16,
        width_px: u32,
        height_px: u32,
    },
}

/// Channels connecting the terminal to a protocol's network task.
pub struct RemoteIo {
    /// Bytes received from the remote host. Closing the channel ends the session cleanly.
    pub from_remote: mpsc::Receiver<Vec<u8>>,
    pub to_remote: mpsc::Sender<RemoteCmd>,
}

/// What a [`RemoteFilter`] wants done after seeing remote output.
#[derive(Debug, Default)]
pub struct FilterOutput {
    /// Bytes to send to the remote (e.g. an auto-login username).
    pub send: Vec<u8>,
    /// Allow the terminal to start rendering and accepting input.
    pub start: bool,
    /// End the session with an error.
    pub abort: Option<(Status, String)>,
}

/// Observes remote output before it reaches the terminal (telnet auto-login).
pub trait RemoteFilter: Send {
    fn on_remote_data(&mut self, data: &[u8]) -> FilterOutput;
}

pub struct TerminalConfig {
    pub options: TerminalOptions,
    pub fonts: FontSet,
    /// Start rendering immediately; false waits for a [`FilterOutput::start`].
    pub start_immediately: bool,
    /// Ignore keyboard input and paste (`read-only`).
    pub read_only: bool,
    pub filter: Option<Box<dyn RemoteFilter>>,
    /// Minimum time between frames.
    pub frame_interval: Duration,
}

impl TerminalConfig {
    /// Build from connection parameters shared by all terminal-based protocols
    /// (`font-size`, `scrollback`, `backspace`, `color-scheme`, `func-keys-and-keypad`,
    /// `disable-copy`, `disable-paste`, `read-only`).
    pub fn from_params(params: &Params, dpi: u32) -> Self {
        let mut options = TerminalOptions {
            dpi: dpi.max(1),
            ..Default::default()
        };
        if let Some(sz) = params
            .get("font-size")
            .and_then(|s| s.trim().parse::<f32>().ok())
        {
            if sz > 0.0 && sz < 512.0 {
                options.font_size_pt = sz;
            }
        }
        if let Some(n) = params.get_u32("scrollback") {
            options.scrollback = n.clamp(1, MAX_SCROLLBACK) as usize;
        }
        if let Some(b) = params.get_u32("backspace") {
            options.backspace = b.min(255) as u8;
        }
        if let Some(cs) = params.get("color-scheme") {
            let (scheme, warnings) = parse_color_scheme(cs);
            for w in warnings {
                tracing::warn!("{w}");
            }
            options.color_scheme = scheme;
        }
        options.vt100_func_keys = params.get("func-keys-and-keypad") == Some("vt100");
        options.linux_console_keys = params.get("terminal-type").unwrap_or("linux") == "linux";
        options.disable_copy = params.get_bool("disable-copy");
        options.disable_paste = params.get_bool("disable-paste");
        Self {
            options,
            fonts: FontSet::discover(),
            start_immediately: true,
            read_only: params.get_bool("read-only"),
            filter: None,
            frame_interval: Duration::from_millis(16),
        }
    }
}

/// Upper bound for the `scrollback` parameter (memory grows with lines x columns).
const MAX_SCROLLBACK: u32 = 100_000;

const CLIPBOARD_STREAM_BASE: u32 = 0x4000_0000;

async fn send_clipboard(ctx: &SessionCtx, stream: u32, text: &str) -> Result<(), SessionError> {
    let id = stream.to_string();
    ctx.tx
        .send(Instruction::new(
            "clipboard",
            [id.clone(), "text/plain".into()],
        ))
        .await?;
    for chunk in text.as_bytes().chunks(4096) {
        let b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
        ctx.tx
            .send(Instruction::new("blob", [id.clone(), b64]))
            .await?;
    }
    ctx.tx.send(Instruction::new("end", [id])).await
}

/// Run a terminal session until the client leaves (cancellation), the remote closes
/// (`Ok`), or a filter aborts (`Err`).
pub async fn run_terminal(
    mut ctx: SessionCtx,
    cfg: TerminalConfig,
    mut remote: RemoteIo,
) -> Result<(), SessionError> {
    let TerminalConfig {
        options,
        fonts,
        start_immediately,
        read_only,
        mut filter,
        frame_interval,
    } = cfg;
    // Sizes come from the client: bound them so nothing hostile can force a huge allocation.
    let clamp = |v: u32| v.clamp(1, guac_display::MAX_DIMENSION);
    let (w, h) = (
        clamp(ctx.tx.caps().optimal_width),
        clamp(ctx.tx.caps().optimal_height),
    );
    let mut term = Terminal::new(options, fonts, w, h);
    term.set_started(start_immediately);
    let mut display = Display::new(w, h);
    let mut clip_stream = CLIPBOARD_STREAM_BASE;

    // Tell the remote the initial grid size.
    let (cols, rows) = term.size_cells();
    let _ = remote
        .to_remote
        .send(RemoteCmd::Resize {
            cols: cols as u16,
            rows: rows as u16,
            width_px: w,
            height_px: h,
        })
        .await;

    let mut dirty = true;
    let mut last_frame: Option<Instant> = None;
    // The official Java tunnel times out after 15 seconds without guacd output.
    // Keep idle terminals and credential prompts alive without repainting them.
    let heartbeat_interval = Duration::from_secs(5);
    let mut heartbeat =
        tokio::time::interval_at(Instant::now() + heartbeat_interval, heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let next_frame = match (dirty, last_frame) {
            (true, Some(t)) => Some(t + frame_interval),
            (true, None) => Some(Instant::now()),
            _ => None,
        };
        let frame_due = async {
            match next_frame {
                Some(t) => tokio::time::sleep_until(t).await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            biased;
            _ = ctx.tx.cancelled() => return Ok(()),
            ev = ctx.events.recv() => {
                let Some(ev) = ev else { return Ok(()) };
                match ev {
                    InputEvent::Key { keysym, pressed } => {
                        if !read_only {
                            term.key(keysym, pressed);
                        }
                    }
                    InputEvent::Mouse { x, y, mask } => term.mouse(x, y, mask),
                    InputEvent::Clipboard(text) => {
                        if read_only {
                            // ignore paste
                        } else {
                            term.set_local_clipboard(text);
                        }
                    }
                    InputEvent::Resize { width, height } => {
                        let (width, height) = (clamp(width), clamp(height));
                        if let Some((cols, rows)) = term.resize_px(width, height) {
                            let _ = remote
                                .to_remote
                                .send(RemoteCmd::Resize {
                                    cols: cols as u16,
                                    rows: rows as u16,
                                    width_px: width,
                                    height_px: height,
                                })
                                .await;
                        }
                    }
                }
                dirty = true;
            }
            data = remote.from_remote.recv() => {
                match data {
                    Some(bytes) => {
                        if let Some(f) = filter.as_mut() {
                            let out = f.on_remote_data(&bytes);
                            if !out.send.is_empty() {
                                let _ = remote.to_remote.send(RemoteCmd::Data(out.send)).await;
                            }
                            if out.start {
                                term.set_started(true);
                                filter = None;
                            }
                            if let Some((status, msg)) = out.abort {
                                return Err(SessionError::plugin(status, msg));
                            }
                        }
                        term.feed(&bytes);
                        dirty = true;
                    }
                    None => {
                        // Show whatever the remote printed last, then end the session.
                        term.render(&mut display);
                        for i in display.flush() {
                            ctx.tx.send(i).await?;
                        }
                        ctx.tx.sync().await?;
                        return Ok(());
                    }
                }
            }
            _ = heartbeat.tick() => {
                ctx.tx.sync().await?;
            }
            _ = frame_due => {
                term.render(&mut display);
                let frame = display.flush();
                if !frame.is_empty() {
                    for i in frame {
                        ctx.tx.send(i).await?;
                    }
                    ctx.tx.sync().await?;
                    last_frame = Some(Instant::now());
                    // Don't run ahead of a slow client: this also stops us reading from the remote.
                    tokio::select! {
                        _ = ctx.tx.wait_lag(2) => {}
                        _ = tokio::time::sleep(Duration::from_secs(10)) => {
                            tracing::warn!("client did not acknowledge frames for 10s; continuing");
                        }
                    }
                }
                dirty = false;
            }
        }

        // Flush terminal output (key presses, replies) and clipboard after every event.
        let out = term.take_output();
        if !out.is_empty() {
            let _ = remote.to_remote.send(RemoteCmd::Data(out)).await;
        }
        if let Some(text) = term.take_clipboard() {
            send_clipboard(&ctx, clip_stream, &text).await?;
            clip_stream += 1;
        }
    }
}
