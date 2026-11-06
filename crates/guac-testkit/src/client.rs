use guac_protocol::{Instruction, Parser};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub struct HandshakeOutcome {
    /// The `args` instruction's elements: protocol version first, then argument names.
    pub args: Vec<String>,
    pub connection_id: String,
}

/// A scriptable Guacamole client for tests.
pub struct SimClient {
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    parser: Parser,
    pub timeout: Duration,
    /// Every instruction received so far, in order (for assertions/golden recording).
    pub log: Vec<Instruction>,
}

impl SimClient {
    pub fn new<S>(stream: S) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (r, w) = tokio::io::split(stream);
        Self {
            reader: Box::new(r),
            writer: Box::new(w),
            parser: Parser::new(),
            timeout: Duration::from_secs(5),
            log: vec![],
        }
    }

    pub async fn send(&mut self, ins: Instruction) -> std::io::Result<()> {
        self.writer.write_all(&ins.encode()).await?;
        self.writer.flush().await
    }

    pub async fn send_raw(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes).await?;
        self.writer.flush().await
    }

    /// Next instruction, `None` on EOF; panics on timeout or parse error (test failure).
    pub async fn recv(&mut self) -> Option<Instruction> {
        let mut buf = [0u8; 4096];
        loop {
            if let Some(i) = self.parser.next_instruction() {
                self.log.push(i.clone());
                return Some(i);
            }
            let n = tokio::time::timeout(self.timeout, self.reader.read(&mut buf))
                .await
                .expect("SimClient: timed out waiting for server")
                .unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.parser
                .feed(&buf[..n])
                .expect("SimClient: server sent malformed protocol");
        }
    }

    /// Like `recv` but returns `None` instead of panicking when nothing arrives within `d`.
    pub async fn recv_within(&mut self, d: Duration) -> Option<Instruction> {
        let old = self.timeout;
        self.timeout = d;
        let mut buf = [0u8; 4096];
        let out = loop {
            if let Some(i) = self.parser.next_instruction() {
                self.log.push(i.clone());
                break Some(i);
            }
            match tokio::time::timeout(d, self.reader.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => self.parser.feed(&buf[..n]).expect("malformed"),
                _ => break None,
            }
        };
        self.timeout = old;
        out
    }

    /// Receive until an instruction with `opcode` arrives; returns it.
    pub async fn expect(&mut self, opcode: &str) -> Instruction {
        loop {
            let i = self
                .recv()
                .await
                .unwrap_or_else(|| panic!("SimClient: EOF while waiting for {opcode}"));
            if i.opcode == opcode {
                return i;
            }
        }
    }

    /// Receive instructions until (and including) the next `sync`, reply to it, return them.
    pub async fn frame(&mut self) -> Vec<Instruction> {
        let mut v = vec![];
        loop {
            let i = self.recv().await.expect("EOF while waiting for sync");
            let is_sync = i.opcode == "sync";
            if is_sync {
                let ts = i.args[0].clone();
                self.send(Instruction::new("sync", [ts])).await.unwrap();
            }
            v.push(i);
            if is_sync {
                return v;
            }
        }
    }

    /// Full client handshake. `values` fills connect values by argument name (others empty).
    pub async fn handshake(
        &mut self,
        protocol: &str,
        values: &[(&str, &str)],
        width: u32,
        height: u32,
    ) -> HandshakeOutcome {
        self.send(Instruction::new("select", [protocol]))
            .await
            .unwrap();
        let args = self.expect("args").await.args;
        self.send(Instruction::new(
            "size",
            [width.to_string(), height.to_string(), "96".into()],
        ))
        .await
        .unwrap();
        self.send(Instruction::new("audio", Vec::<String>::new()))
            .await
            .unwrap();
        self.send(Instruction::new("video", Vec::<String>::new()))
            .await
            .unwrap();
        self.send(Instruction::new("image", ["image/png", "image/jpeg"]))
            .await
            .unwrap();
        self.send(Instruction::new("timezone", ["UTC"]))
            .await
            .unwrap();
        let mut connect = vec![args[0].clone()];
        for name in &args[1..] {
            let v = values
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| *v)
                .unwrap_or("");
            connect.push(v.to_string());
        }
        self.send(Instruction::new("connect", connect))
            .await
            .unwrap();
        let ready = self.expect("ready").await;
        HandshakeOutcome {
            args,
            connection_id: ready.args[0].clone(),
        }
    }
}
