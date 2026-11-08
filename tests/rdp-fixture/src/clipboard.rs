//! 僅在明確啟用時提供固定測試文字，不接觸系統剪貼簿。
use ironrdp_cliprdr::{
    backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory},
    pdu::{
        ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
        FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
    },
};
use ironrdp_core::IntoOwned;
use ironrdp_server::{CliprdrServerFactory, ServerEvent, ServerEventSender};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::mpsc::UnboundedSender;

pub type SharedSender = Arc<Mutex<Option<UnboundedSender<ServerEvent>>>>;
pub struct Factory {
    directory: PathBuf,
    sender: SharedSender,
    updated: Arc<AtomicBool>,
    fixed_only: bool,
}

impl Factory {
    pub fn new(
        directory: PathBuf,
        sender: SharedSender,
        updated: Arc<AtomicBool>,
        fixed_only: bool,
    ) -> Self {
        Self {
            directory,
            sender,
            updated,
            fixed_only,
        }
    }
}

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        *self.sender.lock().unwrap() = Some(sender);
    }
}
impl CliprdrServerFactory for Factory {}
impl CliprdrBackendFactory for Factory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(Backend {
            directory: self.directory.clone(),
            sender: self.sender.lock().unwrap().clone().unwrap(),
            updated: self.updated.clone(),
            fixed_only: self.fixed_only,
        })
    }
}

#[derive(Debug)]
struct Backend {
    directory: PathBuf,
    sender: UnboundedSender<ServerEvent>,
    updated: Arc<AtomicBool>,
    fixed_only: bool,
}
ironrdp_core::impl_as_any!(Backend);
impl Backend {
    fn send(&self, message: ClipboardMessage) {
        let _ = self.sender.send(ServerEvent::Clipboard(message));
    }
}
impl CliprdrBackend for Backend {
    fn temporary_directory(&self) -> &str {
        "."
    }
    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
    }
    fn on_ready(&mut self) {
        self.updated.store(false, Ordering::SeqCst);
        let _ = std::fs::write(self.directory.join("clipboard-ready"), b"ready");
        self.send(ClipboardMessage::SendInitiateCopy(vec![
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ]));
    }
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        if formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_UNICODETEXT)
        {
            self.send(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }
    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let data = if request.format == ClipboardFormatId::CF_UNICODETEXT {
            FormatDataResponse::new_unicode_string(if self.updated.load(Ordering::SeqCst) {
                "調整後遠端🙂\r\n第二行"
            } else {
                "遠端測試🙂\r\n第二行"
            })
        } else {
            FormatDataResponse::new_error()
        };
        self.send(ClipboardMessage::SendFormatData(data.into_owned()));
    }
    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if !response.is_error() {
            if let Ok(text) = response.to_unicode_string() {
                if self.fixed_only {
                    if text == "遠端測試🙂\r\n第二行" {
                        let _ = std::fs::write(self.directory.join("clipboard-matched"), b"ok");
                    } else {
                        let _ = std::fs::write(
                            self.directory.join("clipboard-unexpected"),
                            b"mismatch",
                        );
                    }
                } else {
                    let _ = std::fs::write(self.directory.join("clipboard"), text);
                }
            }
        }
    }
    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_only_records_only_matching_marker() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "cracamole-clipboard-fixture-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0.join("clipboard-matched"));
                let _ = std::fs::remove_file(self.0.join("clipboard-unexpected"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut backend = Backend {
            directory: directory.clone(),
            sender,
            updated: Arc::new(AtomicBool::new(false)),
            fixed_only: true,
        };
        backend.on_format_data_response(FormatDataResponse::new_unicode_string(
            "unexpected fixture data",
        ));
        assert_eq!(
            std::fs::read(directory.join("clipboard-unexpected")).unwrap(),
            b"mismatch"
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        backend.on_format_data_response(FormatDataResponse::new_unicode_string(
            "遠端測試🙂\r\n第二行",
        ));
        assert_eq!(
            std::fs::read(directory.join("clipboard-matched")).unwrap(),
            b"ok"
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
    }
}
