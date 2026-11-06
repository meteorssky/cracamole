use crate::{InstructionReader, Params, SessionError};
use guac_protocol::{Instruction, ProtocolVersion};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

/// guacd waits at most 15 s for each handshake instruction (C `GUACD_TIMEOUT`).
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// What the client told us about itself during the handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCaps {
    pub optimal_width: u32,
    pub optimal_height: u32,
    pub optimal_dpi: u32,
    pub audio: Vec<String>,
    pub video: Vec<String>,
    pub image: Vec<String>,
    pub timezone: Option<String>,
}

impl Default for ClientCaps {
    fn default() -> Self {
        Self {
            optimal_width: 1024,
            optimal_height: 768,
            optimal_dpi: 96,
            audio: vec![],
            video: vec![],
            image: vec![],
            timezone: None,
        }
    }
}

#[derive(Debug)]
pub struct Handshake {
    pub caps: ClientCaps,
    pub params: Params,
    /// Protocol version both sides agreed to speak.
    pub version: ProtocolVersion,
}

/// Server side of the handshake, following C `guac_user_handle_connection`:
/// send `args`; read `size`/`audio`/`video`/`image`/`timezone`/`name`... until `connect`;
/// send `ready`. The `connect` instruction must carry the client version followed by
/// exactly one value per advertised argument.
pub async fn handshake<R, W>(
    reader: &mut InstructionReader<R>,
    writer: &mut W,
    arg_names: &[&str],
    connection_id: &str,
    timeout: Duration,
) -> Result<Handshake, SessionError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut args = vec![ProtocolVersion::LATEST.to_string()];
    args.extend(arg_names.iter().map(|s| s.to_string()));
    writer
        .write_all(&Instruction::new("args", args).encode())
        .await?;
    writer.flush().await?;

    let mut caps = ClientCaps::default();
    let connect = loop {
        let ins = match tokio::time::timeout(timeout, reader.read()).await {
            Err(_) => return Err(SessionError::Timeout),
            Ok(r) => r?.ok_or(SessionError::Closed)?,
        };
        let num = |i: usize| ins.args.get(i).and_then(|s| s.parse::<u32>().ok());
        match ins.opcode.as_str() {
            "connect" => break ins,
            "size" => {
                if let (Some(w), Some(h)) = (num(0), num(1)) {
                    caps.optimal_width = w;
                    caps.optimal_height = h;
                }
                if let Some(dpi) = num(2) {
                    caps.optimal_dpi = dpi;
                }
            }
            "audio" => caps.audio = ins.args,
            "video" => caps.video = ins.args,
            "image" => caps.image = ins.args,
            "timezone" => caps.timezone = ins.args.into_iter().next().filter(|s| !s.is_empty()),
            // Other handshake-time instructions (e.g. "name") are accepted and ignored.
            other => tracing::debug!(opcode = other, "ignoring handshake instruction"),
        }
    };

    // Same order as C: acknowledge first, then validate the value count.
    writer
        .write_all(&Instruction::new("ready", [connection_id]).encode())
        .await?;
    writer.flush().await?;

    if connect.args.len() != arg_names.len() + 1 {
        return Err(SessionError::Handshake(format!(
            "client returned {} connect values, expected {}",
            connect.args.len(),
            arg_names.len() + 1
        )));
    }

    let client_version =
        ProtocolVersion::parse(&connect.args[0]).unwrap_or(ProtocolVersion::V1_0_0);
    let params = Params::from_pairs(
        arg_names
            .iter()
            .map(|s| s.to_string())
            .zip(connect.args.into_iter().skip(1)),
    );
    Ok(Handshake {
        caps,
        params,
        version: ProtocolVersion::LATEST.negotiate(client_version),
    })
}
