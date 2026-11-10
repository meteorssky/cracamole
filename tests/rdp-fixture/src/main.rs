//! 本機測試專用 RDP 伺服器：只綁定 loopback，憑證每次啟動時產生。
use ironrdp_server::{
    BitmapUpdate, CredentialDecision, CredentialValidationError, CredentialValidator, Credentials,
    DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, RdpServer,
    RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler,
};
use sha2::{Digest, Sha256};
use std::{
    num::{NonZeroU16, NonZeroUsize},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
};
use tokio::net::TcpListener;
mod clipboard;

struct Display(
    Arc<AtomicBool>,
    Option<PathBuf>,
    PathBuf,
    tokio::sync::watch::Sender<(u16, u16)>,
);
struct Updates {
    count: u32,
    stress_directory: Option<PathBuf>,
    size: tokio::sync::watch::Receiver<(u16, u16)>,
}

#[async_trait::async_trait]
impl RdpServerDisplay for Display {
    async fn size(&mut self) -> DesktopSize {
        let (width, height) = *self.3.borrow();
        DesktopSize { width, height }
    }
    async fn updates(&mut self) -> anyhow::Result<Box<dyn RdpServerDisplayUpdates>> {
        self.0.store(true, Ordering::SeqCst);
        Ok(Box::new(Updates {
            count: 0,
            stress_directory: self.1.clone(),
            size: self.3.subscribe(),
        }))
    }
    fn request_layout(&mut self, layout: ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout) {
        if let Some(monitor) = layout
            .monitors()
            .iter()
            .find(|monitor| monitor.is_primary())
        {
            let (width, height) = monitor.dimensions();
            let _ = std::fs::write(self.2.join("layout"), format!("{width}x{height}"));
            if let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) {
                self.3.send_replace((width, height));
            }
        }
    }
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>> {
        if self.count != 0 {
            tokio::select! {
                changed = self.size.changed() => {
                    changed?;
                    let (width, height) = *self.size.borrow_and_update();
                    return Ok(Some(DisplayUpdate::Resize(DesktopSize { width, height })));
                }
                _ = async {
                    if self.stress_directory.is_none() || self.count >= 100 {
                        std::future::pending::<()>().await;
                    } else {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                } => {}
            }
        }
        self.count += 1;
        if let Some(directory) = &self.stress_directory {
            std::fs::write(directory.join("frames"), self.count.to_string())?;
        }
        let (width, height) = *self.size.borrow_and_update();
        // The test server's bitmap encoder requires a width divisible by four;
        // the negotiated desktop itself may have any even width.
        let bitmap_width = width & !3;
        let mut data = Vec::with_capacity(usize::from(bitmap_width) * usize::from(height) * 4);
        let red = if width < 500 && (width != 320 || height != 240) {
            0x73
        } else if width != 320 || height != 240 {
            0x53
        } else if self.stress_directory.is_some() && self.count.is_multiple_of(2) {
            0xd2
        } else {
            0xd3
        };
        for _ in 0..usize::from(bitmap_width) * usize::from(height) {
            data.extend_from_slice(&[0x19, 0x71, red, 0xff]);
        }
        Ok(Some(DisplayUpdate::Bitmap(BitmapUpdate {
            x: 0,
            y: 0,
            width: NonZeroU16::new(bitmap_width).unwrap(),
            height: NonZeroU16::new(height).unwrap(),
            format: PixelFormat::BgrA32,
            data: data.into(),
            stride: NonZeroUsize::new(usize::from(bitmap_width) * 4).unwrap(),
        })))
    }
}

struct InputHandler {
    directory: PathBuf,
    mouse: Arc<AtomicU8>,
    clipboard_sender: clipboard::SharedSender,
    clipboard_updated: Arc<AtomicBool>,
    focus_test_step: u8,
    secure_attention_step: usize,
}

struct Validator(PathBuf);
#[async_trait::async_trait]
impl CredentialValidator for Validator {
    async fn validate(
        &self,
        credentials: &Credentials,
    ) -> Result<CredentialDecision, CredentialValidationError> {
        if credentials.username == "fixture"
            && credentials.password == "fixture-secret"
            && credentials.domain.is_none()
        {
            std::fs::write(self.0.join("authenticated"), b"ok")
                .map_err(CredentialValidationError::new)?;
            Ok(CredentialDecision::Accept)
        } else {
            std::fs::write(self.0.join("rejected"), b"rejected")
                .map_err(CredentialValidationError::new)?;
            Ok(CredentialDecision::Reject)
        }
    }
}
impl RdpServerInputHandler for InputHandler {
    fn keyboard(&mut self, event: KeyboardEvent) {
        let expected = match self.secure_attention_step {
            0 => matches!(event, KeyboardEvent::Pressed { code: 0x1d, extended: false }),
            1 => matches!(event, KeyboardEvent::Pressed { code: 0x38, extended: false }),
            2 => matches!(event, KeyboardEvent::Pressed { code: 0x53, extended: true }),
            3 => matches!(event, KeyboardEvent::Released { code: 0x53, extended: true }),
            4 => matches!(event, KeyboardEvent::Released { code: 0x38, extended: false }),
            5 => matches!(event, KeyboardEvent::Released { code: 0x1d, extended: false }),
            _ => false,
        };
        if expected {
            self.secure_attention_step += 1;
            if self.secure_attention_step == 6 {
                let _ = std::fs::write(self.directory.join("secure-attention"), b"ok");
            }
        } else {
            self.secure_attention_step = 0;
        }
        let expected = match self.focus_test_step {
            0 => matches!(
                event,
                KeyboardEvent::Pressed {
                    code: 0x1d,
                    extended: false
                }
            ),
            1 => matches!(
                event,
                KeyboardEvent::Pressed {
                    code: 0x1e,
                    extended: false
                }
            ),
            2 => matches!(
                event,
                KeyboardEvent::Released {
                    code: 0x1e,
                    extended: false
                }
            ),
            3 => matches!(
                event,
                KeyboardEvent::Released {
                    code: 0x1d,
                    extended: false
                }
            ),
            4 => matches!(
                event,
                KeyboardEvent::Pressed {
                    code: 0x4b,
                    extended: true
                }
            ),
            5 => matches!(
                event,
                KeyboardEvent::Released {
                    code: 0x4b,
                    extended: true
                }
            ),
            _ => false,
        };
        if expected {
            self.focus_test_step += 1;
            if self.focus_test_step == 4 {
                let _ = std::fs::write(self.directory.join("focus-release"), b"ok");
            }
            if self.focus_test_step == 6 {
                let _ = std::fs::write(self.directory.join("focus-arrow-release"), b"ok");
            }
        }
        if matches!(
            event,
            KeyboardEvent::Pressed {
                code: 0x42,
                extended: false
            }
        ) {
            let _ = std::fs::write(self.directory.join("copy-trigger"), b"triggered");
            self.clipboard_updated.store(true, Ordering::SeqCst);
            if let Some(sender) = self.clipboard_sender.lock().unwrap().as_ref() {
                let formats = vec![ironrdp_cliprdr::pdu::ClipboardFormat::new(
                    ironrdp_cliprdr::pdu::ClipboardFormatId::CF_UNICODETEXT,
                )];
                let _ = sender.send(ironrdp_server::ServerEvent::Clipboard(
                    ironrdp_cliprdr::backend::ClipboardMessage::SendInitiateCopy(formats),
                ));
            }
        }
        if matches!(
            event,
            KeyboardEvent::Pressed {
                code: 0x2c,
                extended: false
            }
        ) {
            let _ = std::fs::write(self.directory.join("key"), b"z");
        }
    }
    fn mouse(&mut self, event: MouseEvent) {
        let bit = match event {
            MouseEvent::Move { x: 42, y: 31 } => 1,
            MouseEvent::LeftPressed => 2,
            MouseEvent::LeftReleased => 4,
            _ => 0,
        };
        if bit != 0 && (self.mouse.fetch_or(bit, Ordering::SeqCst) | bit) == 7 {
            let _ = std::fs::write(self.directory.join("mouse"), b"ok");
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("missing fixture directory"))?,
    );
    let mode = std::env::args_os().nth(2);
    let interactive_clipboard = matches!(mode.as_deref(), Some(value) if value == std::ffi::OsStr::new("frames-clipboard") || value == std::ffi::OsStr::new("nla-frames-clipboard"));
    let clipboard_enabled = interactive_clipboard
        || matches!(mode.as_deref(), Some(value) if value == std::ffi::OsStr::new("clipboard") || value == std::ffi::OsStr::new("nla-clipboard"));
    let stress = interactive_clipboard
        || matches!(mode.as_deref(), Some(value) if value == std::ffi::OsStr::new("frames") || value == std::ffi::OsStr::new("nla-frames"));
    let nla = matches!(mode.as_deref(), Some(value) if value == std::ffi::OsStr::new("nla") || value == std::ffi::OsStr::new("nla-frames") || value == std::ffi::OsStr::new("nla-clipboard") || value == std::ffi::OsStr::new("nla-frames-clipboard"));
    let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
    let public_key = identity.key_pair.public_key_raw().to_vec();
    let certificate = identity.cert.der().clone();
    let fingerprint = Sha256::digest(certificate.as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let key = ironrdp_server::tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(
        identity.key_pair.serialize_der(),
    );
    let config = ironrdp_server::tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key.into())?;
    let acceptor = ironrdp_server::tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let connected = Arc::new(AtomicBool::new(false));
    let clipboard_sender: clipboard::SharedSender = Arc::new(std::sync::Mutex::new(None));
    let clipboard_updated = Arc::new(AtomicBool::new(false));
    let (display_size, _) = tokio::sync::watch::channel((320, 240));
    let builder = RdpServer::builder().with_addr(addr);
    let builder = if nla {
        builder.with_hybrid(acceptor, public_key)
    } else {
        builder.with_tls(acceptor)
    };
    let mut server = builder
        .with_input_handler(InputHandler {
            directory: directory.clone(),
            mouse: Arc::new(AtomicU8::new(0)),
            clipboard_sender: clipboard_sender.clone(),
            clipboard_updated: clipboard_updated.clone(),
            focus_test_step: 0,
            secure_attention_step: 0,
        })
        .with_display_handler(Display(
            connected.clone(),
            stress.then(|| directory.clone()),
            directory.clone(),
            display_size,
        ))
        .with_cliprdr_factory(clipboard_enabled.then(|| {
            Box::new(clipboard::Factory::new(
                directory.clone(),
                clipboard_sender,
                clipboard_updated,
                interactive_clipboard,
            )) as Box<dyn ironrdp_server::CliprdrServerFactory>
        }))
        .with_credential_validator(Some(Arc::new(Validator(directory.clone()))))
        .build();
    if nla {
        server.set_credentials(Some(Credentials {
            username: "fixture".into(),
            password: "fixture-secret".into(),
            domain: None,
        }));
    }
    std::fs::write(
        directory.join("ready"),
        format!("{}\n{}\n", addr.port(), fingerprint),
    )?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(90);
    let mut completed_connections = 0_u32;
    loop {
        let Ok(accepted) = tokio::time::timeout_at(deadline, listener.accept()).await else {
            break;
        };
        let (stream, _) = accepted?;
        connected.store(false, Ordering::SeqCst);
        let result = tokio::time::timeout_at(deadline, server.run_connection(stream)).await;
        completed_connections += 1;
        std::fs::write(
            directory.join("completed"),
            completed_connections.to_string(),
        )?;
        if connected.swap(false, Ordering::SeqCst) {
            std::fs::write(directory.join("closed"), b"closed")?;
        }
        if result.is_err() {
            break;
        }
    }
    Ok(())
}
