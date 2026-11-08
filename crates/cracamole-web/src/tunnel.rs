use crate::{auth, ApiError, App};
use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket},
        Path, Query, State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode},
    response::Response,
};
use futures_util::SinkExt;
use guac_protocol::{Instruction, Parser};
use guac_session::InstructionReader;
use serde::Deserialize;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf},
    net::TcpStream,
};

trait Pipe: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Pipe for T {}
type BackendStream = Box<dyn Pipe>;

/// Memory bound of one in-process tunnel pipe in each direction.
const PIPE_BYTES: usize = 64 * 1024;

async fn open_stream(link: &crate::BackendLink) -> Result<BackendStream, String> {
    match link {
        crate::BackendLink::Tcp(address) => {
            let stream = TcpStream::connect(address)
                .await
                .map_err(|e| e.to_string())?;
            stream.set_nodelay(true).map_err(|e| e.to_string())?;
            Ok(Box::new(stream))
        }
        crate::BackendLink::Memory(sender) => {
            let (ours, theirs) = tokio::io::duplex(PIPE_BYTES);
            sender
                .send((theirs, "web".to_string()))
                .await
                .map_err(|_| "daemon is not running".to_string())?;
            Ok(Box::new(ours))
        }
    }
}

#[derive(Deserialize, Clone, Copy)]
pub struct Size {
    #[serde(default = "width")]
    width: u32,
    #[serde(default = "height")]
    height: u32,
    #[serde(default = "dpi")]
    dpi: u32,
}
fn width() -> u32 {
    1024
}
fn height() -> u32 {
    768
}
fn dpi() -> u32 {
    96
}

pub async fn upgrade(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(size): Query<Size>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let (expires, cancel) = app.auth.authorize(&headers).await?;
    auth::same_origin(&headers)?;
    let connection = app
        .store
        .get(&id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "找不到連線"))?;
    let permit = app
        .tunnels
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "連線數已達上限"))?;
    let backend = tokio::time::timeout(
        Duration::from_secs(15),
        connect(&app.backend, &connection, size),
    )
    .await
    .map_err(|_| ApiError::new(StatusCode::GATEWAY_TIMEOUT, "終端機服務握手逾時"))?
    .map_err(|e| {
        tracing::warn!("web tunnel handshake failed: {e}");
        ApiError::new(StatusCode::BAD_GATEWAY, "無法建立終端機連線")
    })?;
    Ok(ws
        .protocols(["guacamole"])
        .read_buffer_size(8192)
        .write_buffer_size(0)
        .max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .max_write_buffer_size(128 * 1024)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            let started = std::time::Instant::now();
            tracing::info!(
                "web tunnel opened: {} ({}, {}x{})",
                connection.id,
                connection.protocol,
                size.width,
                size.height
            );
            let result = bridge(socket, backend, expires, cancel, app.shutdown.clone()).await;
            let seconds = started.elapsed().as_secs();
            match result {
                Ok(()) => tracing::info!("web tunnel closed: {} after {seconds}s", connection.id),
                Err(error) => tracing::warn!(
                    "web tunnel failed: {} after {seconds}s: {error}",
                    connection.id
                ),
            }
        }))
}

type Backend = (
    InstructionReader<ReadHalf<BackendStream>>,
    WriteHalf<BackendStream>,
    String,
);
async fn connect(
    link: &crate::BackendLink,
    connection: &crate::store::Connection,
    size: Size,
) -> Result<Backend, String> {
    let stream = open_stream(link).await?;
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = InstructionReader::new(read);
    write
        .write_all(&Instruction::new("select", [&connection.protocol]).encode())
        .await
        .map_err(|e| e.to_string())?;
    let args = reader
        .read()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("backend closed")?;
    if args.opcode != "args" || args.args.is_empty() {
        return Err("backend did not advertise args".into());
    }
    let mut instructions = vec![
        Instruction::new(
            "size",
            [
                size.width.clamp(1, 4096).to_string(),
                size.height.clamp(1, 4096).to_string(),
                size.dpi.clamp(48, 192).to_string(),
            ],
        ),
        Instruction::new("audio", Vec::<String>::new()),
        Instruction::new("video", Vec::<String>::new()),
        Instruction::new("image", ["image/png"]),
    ];
    let mut values = vec![args.args[0].clone()];
    values.extend(
        args.args[1..]
            .iter()
            .map(|name| connection.params.get(name).cloned().unwrap_or_default()),
    );
    instructions.push(Instruction::new("connect", values));
    for instruction in instructions {
        write
            .write_all(&instruction.encode())
            .await
            .map_err(|e| e.to_string())?;
    }
    let ready = reader
        .read()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("backend closed before ready")?;
    if ready.opcode != "ready" || ready.args.len() != 1 {
        return Err("backend did not acknowledge connection".into());
    }
    Ok((reader, write, ready.args[0].clone()))
}

async fn send(socket: &mut WebSocket, instruction: Instruction) -> Result<(), String> {
    let text = String::from_utf8(instruction.encode()).map_err(|e| e.to_string())?;
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| "browser write timeout".to_string())?
    .map_err(|e| e.to_string())
}

async fn bridge(
    mut socket: WebSocket,
    (mut reader, mut writer, id): Backend,
    expires: std::time::Instant,
    cancel: tokio_util::sync::CancellationToken,
    shutdown: tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    send(&mut socket, Instruction::new("", [id])).await?;
    let mut parser = Parser::new();
    let mut peer_closed = false;
    let expiry = tokio::time::sleep_until(tokio::time::Instant::from_std(expires));
    tokio::pin!(expiry);
    'bridge: loop {
        tokio::select! {
            _=cancel.cancelled()=>break,
            _=shutdown.cancelled()=>break,
            _=&mut expiry=>break,
            instruction=reader.read()=>{
                let Some(instruction)=instruction.map_err(|e|e.to_string())? else {break};
                let closed=instruction.opcode=="disconnect";
                send(&mut socket,instruction).await?;
                if closed {break}
            }
            message=socket.recv()=>{
                let Some(message)=message else {break};
                match message.map_err(|e|e.to_string())? {
                    Message::Text(text)=>{
                        parser.feed(text.as_bytes()).map_err(|e|e.to_string())?;
                        while let Some(instruction)=parser.next_instruction() {
                            if instruction.opcode.is_empty() && instruction.args.len()==2 && instruction.args[0]=="ping" {
                                send(&mut socket,instruction).await?;
                            } else if matches!(instruction.opcode.as_str(),"key"|"mouse"|"size"|"clipboard"|"blob"|"end"|"sync"|"ack"|"nop"|"disconnect") {
                                tokio::time::timeout(Duration::from_secs(5),writer.write_all(&instruction.encode())).await
                                    .map_err(|_|"backend write timeout".to_string())?.map_err(|e|e.to_string())?;
                                if instruction.opcode == "disconnect" { break 'bridge; }
                            } else {return Err("unsupported browser instruction".into())}
                        }
                    }
                    Message::Close(_)=>{
                        // Tungstenite 自動排入回覆，必須 flush 才能完成關閉交握。
                        socket.flush().await.map_err(|e|e.to_string())?;
                        peer_closed = true;
                        break
                    },
                    Message::Ping(bytes)=>{socket.send(Message::Pong(bytes)).await.map_err(|e|e.to_string())?;}
                    Message::Pong(_)=>{},
                    Message::Binary(_)=>return Err("binary messages are unsupported".into()),
                }
            }
        }
    }
    let _ = writer.shutdown().await;
    if !peer_closed {
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: 1000,
                reason: "".into(),
            })))
            .await;
    }
    Ok(())
}
