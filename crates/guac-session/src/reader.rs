use crate::SessionError;
use guac_protocol::{Instruction, ParseError, Parser};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Reads instructions from an async byte stream. Buffered bytes stay in the parser,
/// so the same reader can be handed from the `select` stage into the handshake and session.
pub struct InstructionReader<R> {
    inner: R,
    parser: Parser,
    buf: Box<[u8]>,
}

impl<R: AsyncRead + Unpin> InstructionReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            parser: Parser::new(),
            buf: vec![0u8; 8192].into_boxed_slice(),
        }
    }

    /// Next instruction, or `None` on clean EOF between instructions.
    pub async fn read(&mut self) -> Result<Option<Instruction>, SessionError> {
        loop {
            if let Some(i) = self.parser.next_instruction() {
                return Ok(Some(i));
            }
            let n = self.inner.read(&mut self.buf).await?;
            if n == 0 {
                return if self.parser.is_mid_instruction() {
                    Err(SessionError::Protocol(ParseError::BadTerminator))
                } else {
                    Ok(None)
                };
            }
            self.parser.feed(&self.buf[..n])?;
        }
    }
}
