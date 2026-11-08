//! 使用行程內雙向串流連接既有協定外掛，不建立對外監聽埠。
use cracamole_store::Connection;
use guac_protocol::Instruction;
use guac_session::{handshake, run_session, InstructionReader, SessionConfig};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
use tokio_util::sync::CancellationToken;

pub struct Session {
    reader: InstructionReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
    shutdown: CancellationToken,
}

impl Session {
    pub async fn connect(connection: &Connection, width: u32, height: u32) -> Result<Self, String> {
        let plugin = guacd::Registry::builtin()
            .get(&connection.protocol)
            .ok_or("unsupported protocol")?;
        let (client, server) = tokio::io::duplex(64 * 1024);
        let shutdown = CancellationToken::new();
        let shutdown_guard = shutdown.clone().drop_guard();
        let server_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(server);
            let mut reader = InstructionReader::new(read);
            let id = guac_session::new_connection_id();
            let result = tokio::select! {
                _ = server_shutdown.cancelled() => return,
                result = handshake(&mut reader, &mut write, plugin.arg_names(), &id, Duration::from_secs(10)) => result,
            };
            if let Ok(handshake) = result {
                let config = SessionConfig {
                    shutdown: Some(server_shutdown),
                    ..SessionConfig::default()
                };
                let _ = run_session(reader, write, handshake, plugin, config).await;
            }
        });
        let (read, mut writer) = tokio::io::split(client);
        let mut reader = InstructionReader::new(read);
        let result = async {
            let args = reader
                .read()
                .await
                .map_err(|e| e.to_string())?
                .ok_or("session closed during handshake")?;
            if args.opcode != "args" || args.args.is_empty() {
                return Err("invalid session handshake".to_string());
            }
            let messages = [
                Instruction::new(
                    "size",
                    [
                        width.clamp(1, 4096).to_string(),
                        height.clamp(1, 4096).to_string(),
                        "96".into(),
                    ],
                ),
                Instruction::new("audio", Vec::<String>::new()),
                Instruction::new("video", Vec::<String>::new()),
                Instruction::new("image", ["image/png"]),
            ];
            for message in messages {
                writer
                    .write_all(&message.encode())
                    .await
                    .map_err(|e| e.to_string())?;
            }
            let mut values = vec![args.args[0].clone()];
            values.extend(
                args.args[1..]
                    .iter()
                    .map(|name| connection.params.get(name).cloned().unwrap_or_default()),
            );
            writer
                .write_all(&Instruction::new("connect", values).encode())
                .await
                .map_err(|e| e.to_string())?;
            let ready = reader
                .read()
                .await
                .map_err(|e| e.to_string())?
                .ok_or("session closed before ready")?;
            if ready.opcode != "ready" || ready.args.len() != 1 {
                return Err("invalid session response".to_string());
            }
            Ok(())
        };
        match tokio::time::timeout(Duration::from_secs(10), result).await {
            Ok(Ok(())) => {
                shutdown_guard.disarm();
                Ok(Self {
                    reader,
                    writer,
                    shutdown,
                })
            }
            Ok(Err(error)) => {
                shutdown.cancel();
                Err(error)
            }
            Err(_) => {
                shutdown.cancel();
                Err("session handshake timed out".into())
            }
        }
    }

    pub async fn read(&mut self) -> Result<Option<Instruction>, String> {
        self.reader.read().await.map_err(|e| e.to_string())
    }

    pub async fn send(&mut self, instruction: Instruction) -> Result<(), String> {
        if !matches!(
            instruction.opcode.as_str(),
            "key"
                | "mouse"
                | "size"
                | "clipboard"
                | "blob"
                | "end"
                | "sync"
                | "ack"
                | "nop"
                | "disconnect"
        ) {
            return Err("unsupported input instruction".into());
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            self.writer.write_all(&instruction.encode()),
        )
        .await
        .map_err(|_| "session write timed out".to_string())?
        .map_err(|e| e.to_string())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}
