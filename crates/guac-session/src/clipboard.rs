//! Clipboard limits apply across all streams, before protocol plugins see the data.

use crate::SessionError;
use guac_protocol::Status;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::Instant;

const MAX_STREAMS: usize = 4;
const IDLE_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) struct Clipboards {
    streams: HashMap<String, (Vec<u8>, Instant)>,
    bytes: usize,
    limit: usize,
}

fn invalid(message: &str) -> SessionError {
    SessionError::plugin(Status::ClientBadRequest, message)
}

impl Clipboards {
    pub fn new(limit: usize) -> Self {
        Self {
            streams: HashMap::new(),
            bytes: 0,
            limit,
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.streams
            .values()
            .map(|(_, activity)| *activity + IDLE_TIMEOUT)
            .min()
    }

    pub fn open(&mut self, id: &str, mime: Option<&str>) -> Result<(), SessionError> {
        if id.parse::<u32>().is_err() || self.streams.contains_key(id) {
            return Err(invalid("Invalid or duplicate clipboard stream."));
        }
        if mime != Some("text/plain") {
            return Err(SessionError::plugin(
                Status::ClientBadType,
                "Only plain text clipboard is supported.",
            ));
        }
        if self.streams.len() >= MAX_STREAMS {
            return Err(SessionError::plugin(
                Status::ClientOverrun,
                "Too many clipboard streams.",
            ));
        }
        self.streams
            .insert(id.to_string(), (Vec::new(), Instant::now()));
        Ok(())
    }

    pub fn append(&mut self, id: &str, bytes: &[u8]) -> Result<(), SessionError> {
        let (buffer, activity) = self
            .streams
            .get_mut(id)
            .ok_or_else(|| invalid("Unknown clipboard stream."))?;
        let total = self
            .bytes
            .checked_add(bytes.len())
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| {
                SessionError::plugin(Status::ClientOverrun, "Clipboard budget exceeded.")
            })?;
        buffer.extend_from_slice(bytes);
        self.bytes = total;
        *activity = Instant::now();
        Ok(())
    }

    pub fn finish(&mut self, id: &str) -> Result<String, SessionError> {
        let (buffer, _) = self
            .streams
            .remove(id)
            .ok_or_else(|| invalid("Unknown clipboard stream."))?;
        self.bytes -= buffer.len();
        String::from_utf8(buffer).map_err(|_| invalid("Clipboard is not valid UTF-8."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_budget_is_enforced_and_released() {
        let mut clips = Clipboards::new(8);
        clips.open("1", Some("text/plain")).unwrap();
        clips.open("2", Some("text/plain")).unwrap();
        clips.append("1", b"12345").unwrap();
        assert!(clips.append("2", b"6789").is_err());
        assert_eq!(clips.finish("1").unwrap(), "12345");
        clips.append("2", b"6789").unwrap();
        assert_eq!(clips.finish("2").unwrap(), "6789");
        assert!(clips.deadline().is_none());
    }

    #[test]
    fn stream_count_duplicates_mime_and_unknown_streams_are_rejected() {
        let mut clips = Clipboards::new(256);
        assert!(clips.open("bad", Some("text/plain")).is_err());
        assert!(clips.open("0", Some("text/html")).is_err());
        for id in 0..MAX_STREAMS {
            clips.open(&id.to_string(), Some("text/plain")).unwrap();
        }
        assert!(clips.open("4", Some("text/plain")).is_err());
        assert!(clips.open("0", Some("text/plain")).is_err());
        assert!(clips.append("unknown", b"x").is_err());
        assert!(clips.finish("unknown").is_err());
        clips.finish("0").unwrap();
        clips.open("4", Some("text/plain")).unwrap();
    }
}
