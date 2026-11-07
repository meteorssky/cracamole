//! Text clipboard (CLIPRDR) bridge between the RDP server and the Guacamole client.

use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use tokio::sync::mpsc::UnboundedSender;

/// Largest clipboard text accepted from the server (characters).
pub const MAX_TEXT: usize = 1 << 20;

/// Events raised by the CLIPRDR callbacks and handled by the session loop.
#[derive(Debug)]
pub enum ClipEvent {
    /// The server wants our current format list.
    FormatListRequested,
    /// The server copied something; `true` when it offers Unicode text.
    RemoteCopied(bool),
    /// The server wants our clipboard contents in this format.
    DataRequested(ClipboardFormatId),
    /// Text received from the server, line endings already normalised to `\n`.
    RemoteText(String),
}

#[derive(Debug)]
pub struct Backend {
    tx: UnboundedSender<ClipEvent>,
}

impl Backend {
    pub fn new(tx: UnboundedSender<ClipEvent>) -> Self {
        Self { tx }
    }

    fn emit(&self, event: ClipEvent) {
        // The receiver only disappears when the session ends.
        let _ = self.tx.send(event);
    }
}

ironrdp_core::impl_as_any!(Backend);

/// Windows uses CRLF; the Guacamole client expects LF.
pub fn to_local(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Convert LF to the CRLF that Windows applications expect.
pub fn to_remote(text: &str) -> String {
    to_local(text).replace('\n', "\r\n")
}

impl CliprdrBackend for Backend {
    fn temporary_directory(&self) -> &str {
        "/tmp"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        self.emit(ClipEvent::FormatListRequested);
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        let has_text = available_formats
            .iter()
            .any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT);
        self.emit(ClipEvent::RemoteCopied(has_text));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.emit(ClipEvent::DataRequested(request.format));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            return;
        }
        if let Ok(text) = response.to_unicode_string() {
            let text = to_local(&text);
            if text.chars().count() <= MAX_TEXT {
                self.emit(ClipEvent::RemoteText(text));
            }
        }
    }

    // File transfer is not supported; requests are simply never answered.
    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_round_trip() {
        assert_eq!(to_local("a\r\nb\rc\n"), "a\nb\nc\n");
        assert_eq!(to_remote("a\nb\r\nc"), "a\r\nb\r\nc");
    }

    #[test]
    fn remote_text_is_forwarded_and_errors_are_not() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut backend = Backend::new(tx);
        backend.on_format_data_response(FormatDataResponse::new_unicode_string("hi\r\nthere"));
        match rx.try_recv() {
            Ok(ClipEvent::RemoteText(t)) => assert_eq!(t, "hi\nthere"),
            other => panic!("unexpected {other:?}"),
        }
        backend.on_format_data_response(FormatDataResponse::new_error());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn only_unicode_text_triggers_a_paste() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut backend = Backend::new(tx);
        backend.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
        backend.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_HDROP)]);
        assert!(matches!(rx.try_recv(), Ok(ClipEvent::RemoteCopied(true))));
        assert!(matches!(rx.try_recv(), Ok(ClipEvent::RemoteCopied(false))));
    }
}
