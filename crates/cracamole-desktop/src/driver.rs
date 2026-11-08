//! 原生介面與共用 Guacamole 工作階段之間的有界資料管線。
use crate::{display::Display, session::Session};
use base64::{engine::general_purpose::STANDARD, Engine};
use cracamole_store::Connection;
use guac_protocol::Instruction;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Arc<[u8]>,
}

pub enum Input {
    Key {
        keysym: u32,
        pressed: bool,
    },
    Mouse {
        x: u32,
        y: u32,
        mask: u32,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Paste {
        text: zeroize::Zeroizing<String>,
        permit: tokio::sync::OwnedSemaphorePermit,
    },
}

impl std::fmt::Debug for Input {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Paste { .. } => formatter.write_str("Paste(<redacted>)"),
            Self::Key { .. } => formatter.write_str("Key"),
            Self::Mouse { .. } => formatter.write_str("Mouse"),
            Self::Resize { .. } => formatter.write_str("Resize"),
        }
    }
}

impl Input {
    fn instruction(self) -> Option<Instruction> {
        match self {
            Self::Key { keysym, pressed } => Some(Instruction::new(
                "key",
                [keysym.to_string(), u8::from(pressed).to_string()],
            )),
            Self::Mouse { x, y, mask } => Some(Instruction::new(
                "mouse",
                [
                    x.min(4095).to_string(),
                    y.min(4095).to_string(),
                    (mask & 31).to_string(),
                ],
            )),
            Self::Resize { width, height } => Some(Instruction::new(
                "size",
                [
                    width.clamp(1, 4096).to_string(),
                    height.clamp(1, 4096).to_string(),
                ],
            )),
            Self::Paste { .. } => None,
        }
    }
}

async fn send_paste(
    session: &mut Session,
    text: zeroize::Zeroizing<String>,
    cancel: &CancellationToken,
) -> Result<(), String> {
    if text.len() > 256 * 1024 {
        return Err("clipboard limit exceeded".into());
    }
    let send = async {
        let id = "1";
        session
            .send(Instruction::new("clipboard", [id, "text/plain"]))
            .await?;
        for chunk in text.as_bytes().chunks(3072) {
            session
                .send(Instruction::new(
                    "blob",
                    [id.to_string(), STANDARD.encode(chunk)],
                ))
                .await?;
        }
        session.send(Instruction::new("end", [id])).await
    };
    tokio::select! {
        _ = cancel.cancelled() => Err("session cancelled".into()),
        result = tokio::time::timeout(std::time::Duration::from_secs(15), send) =>
            result.map_err(|_| "clipboard send timed out".to_string())?,
    }
}

/// 驅動單一連線；影格採最新值覆蓋，避免 UI 停頓時堆積完整畫面。
pub async fn run(
    connection: &Connection,
    width: u32,
    height: u32,
    mut input: mpsc::Receiver<Input>,
    frames: watch::Sender<Option<Arc<Frame>>>,
    clipboard: watch::Sender<Option<Arc<str>>>,
    cancel: CancellationToken,
) -> Result<(), String> {
    let mut session = tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        result = Session::connect(connection, width, height) => result?,
    };
    let mut display = Display::default();
    let mut dirty = false;
    let mut clipboards = guac_session::Clipboards::new(256 * 1024);
    loop {
        let clipboard_deadline = clipboards.deadline();
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = async {
                match clipboard_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            } => return Err("clipboard stream timed out".into()),
            event = input.recv() => match event {
                Some(Input::Paste { text, permit }) => { let _permit = permit; send_paste(&mut session, text, &cancel).await?; },
                Some(event) => session.send(event.instruction().expect("non-paste input")).await?,
                None => return Ok(()),
            },
            incoming = session.read() => {
                let instruction = incoming?.ok_or("session closed without disconnect")?;
                match instruction.opcode.as_str() {
                    "clipboard" => {
                        if instruction.args.len() != 2 { return Err("invalid clipboard".into()); }
                        if display.contains_stream(&instruction.args[0]) { return Err("duplicate stream".into()); }
                        clipboards.open(&instruction.args[0], Some(&instruction.args[1])).map_err(|_| "invalid clipboard stream")?;
                    }
                    "blob" if instruction.args.first().is_some_and(|id| clipboards.contains(id)) => {
                        if instruction.args.len() != 2 || instruction.args[1].len() > 350_000 { return Err("invalid clipboard data".into()); }
                        let bytes = STANDARD.decode(&instruction.args[1]).map_err(|_| "invalid clipboard data")?;
                        clipboards.append(&instruction.args[0], &bytes).map_err(|_| "clipboard limit exceeded")?;
                    }
                    "end" if instruction.args.first().is_some_and(|id| clipboards.contains(id)) => {
                        if instruction.args.len() != 1 { return Err("invalid clipboard end".into()); }
                        let text = clipboards.finish(&instruction.args[0]).map_err(|_| "invalid clipboard text")?;
                        clipboard.send_replace(Some(Arc::from(text)));
                    }
                    "size" | "img" | "blob" | "end" => {
                        if instruction.opcode == "img" && instruction.args.first().is_some_and(|id| clipboards.contains(id)) { return Err("duplicate stream".into()); }
                        dirty |= display.apply(&instruction).map_err(str::to_string)?;
                    }
                    "sync" => {
                        if instruction.args.len() != 1 { return Err("invalid sync".into()); }
                        if dirty {
                            let (width, height) = display.dimensions();
                            frames.send_replace(Some(Arc::new(Frame { width, height, rgba: Arc::from(display.pixels()) })));
                            dirty = false;
                        }
                        session.send(Instruction::new("sync", instruction.args)).await?;
                    }
                    "nop" | "ack" => {}
                    "disconnect" => return Ok(()),
                    "error" => return Err("remote session failed".into()),
                    _ => return Err(format!("unsupported session instruction: {}", instruction.opcode)),
                }
            }
        }
    }
}
