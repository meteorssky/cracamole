//! 遠端工作階段：把共用的 `cracamole_desktop::driver` 影格與輸入接到 GPUI。

use cracamole_desktop::driver::{self, DriverError, Frame};
pub use cracamole_desktop::driver::Input as RemoteInput;
use cracamole_store::{Connection, PublicConnection};
use gpui_kit::{Keystroke, RenderImage};
use std::sync::{Arc, OnceLock};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

pub enum Event {
    Frame(Arc<Frame>),
    /// 遠端複製的文字。
    Clipboard(Arc<str>),
    /// 工作階段結束；`Some` 為可顯示的失敗原因（不含遠端提供的文字）。
    Ended(Option<String>),
}

pub struct Remote {
    pub input: mpsc::Sender<RemoteInput>,
    pub cancel: CancellationToken,
    pub events: mpsc::UnboundedReceiver<Event>,
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// 取得可連線的完整設定：有私有連線庫時讀取含機密的版本，否則只用公開欄位。
pub fn load_connection(
    public: &PublicConnection,
    store: Option<&crate::windows_store::WindowsStore>,
) -> Connection {
    if let Some(store) = store {
        if let Ok(Some(connection)) = runtime().block_on(store.get(&public.id)) {
            return connection;
        }
    }
    Connection {
        id: public.id.clone(),
        name: public.name.clone(),
        protocol: public.protocol.clone(),
        params: public.params.clone(),
    }
}

/// 在共用的 runtime 上執行 future（呼叫端不可在 tokio 執行緒內）。
pub fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    runtime().block_on(future)
}

/// 在共用 runtime 上驗證管理員帳密；密碼交給驗證後即清除。
pub fn verify(
    credentials: Arc<cracamole_store::credentials::Credentials>,
    username: String,
    password: String,
) -> tokio::sync::oneshot::Receiver<Result<bool, cracamole_store::credentials::VerifyError>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    runtime().spawn(async move {
        let _ = tx.send(credentials.verify(&username, password).await);
    });
    rx
}

/// 伺服器身分探測結果（SSH 主機金鑰或 RDP 憑證）。
pub struct Identity {
    /// 顯示給使用者核對的指紋。
    pub fingerprint: String,
    /// 使用者信任後要寫入連線的參數名稱與值。
    pub trust_param: &'static str,
    pub trust_value: String,
    /// 補充說明（例如 RDP 憑證主體）。
    pub detail: Option<String>,
    /// 系統信任來源（known_hosts）已包含此主機。
    pub trusted: bool,
    pub matches: bool,
}

/// 這個連線是否需要先核對伺服器身分。
pub fn needs_identity(connection: &Connection) -> bool {
    let has = |key: &str| connection.params.get(key).is_some_and(|v| !v.is_empty());
    match connection.protocol.as_str() {
        "ssh" => !has("host-key"),
        "rdp" => !has("server-cert-sha256") && !has("ignore-cert"),
        _ => false,
    }
}

/// 探測伺服器身分（不傳送任何驗證資訊）；失敗只回傳固定訊息，不含遠端文字。
pub fn probe(
    protocol: String,
    host: String,
    port: u16,
    configured: Option<String>,
) -> tokio::sync::oneshot::Receiver<Result<Identity, String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    runtime().spawn(async move {
        let failure = "無法取得伺服器身分，請確認主機位址與連接埠。".to_string();
        let result = if protocol == "rdp" {
            guacd::probe_certificate(&host, port, 10)
                .await
                .map(|cert| Identity {
                    fingerprint: cert.sha256.clone(),
                    trust_param: "server-cert-sha256",
                    trust_value: cert.sha256,
                    detail: Some(cert.subject),
                    trusted: false,
                    matches: false,
                })
                .map_err(|_| failure)
        } else {
            guacd::probe_host_key(&host, port, configured.as_deref(), 10)
                .await
                .map(|info| Identity {
                    fingerprint: info.fingerprint,
                    trust_param: "host-key",
                    trust_value: info.public_key,
                    detail: None,
                    trusted: info.trusted,
                    matches: info.matches,
                })
                .map_err(|_| failure)
        };
        let _ = tx.send(result);
    });
    rx
}

pub fn start(connection: Connection, width: u32, height: u32) -> Remote {
    let (input, input_rx) = mpsc::channel(256);
    let (frames_tx, mut frames_rx) = watch::channel(None);
    let (clipboard_tx, mut clipboard_rx) = watch::channel(None::<Arc<str>>);
    let (event_tx, events) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    runtime().spawn(async move {
        let frames_events = event_tx.clone();
        let forwarder = tokio::spawn(async move {
            while frames_rx.changed().await.is_ok() {
                let frame: Option<Arc<Frame>> = frames_rx.borrow_and_update().clone();
                if let Some(frame) = frame {
                    if frames_events.send(Event::Frame(frame)).is_err() {
                        break;
                    }
                }
            }
        });
        let clipboard_events = event_tx.clone();
        let clipboard_forwarder = tokio::spawn(async move {
            while clipboard_rx.changed().await.is_ok() {
                let text: Option<Arc<str>> = clipboard_rx.borrow_and_update().clone();
                if let Some(text) = text {
                    if clipboard_events.send(Event::Clipboard(text)).is_err() {
                        break;
                    }
                }
            }
        });
        let result = driver::run(
            &connection,
            width,
            height,
            input_rx,
            frames_tx,
            clipboard_tx,
            token,
        )
        .await;
        let _ = forwarder.await;
        let _ = clipboard_forwarder.await;
        let message = match result {
            Ok(()) => None,
            Err(DriverError::Remote(status)) => Some(format!("遠端連線錯誤（{status:?}）")),
            Err(DriverError::Other(_)) => Some("連線失敗或異常中斷".to_string()),
        };
        let _ = event_tx.send(Event::Ended(message));
    });
    Remote {
        input,
        cancel,
        events,
    }
}

/// RGBA 影格轉為 GPUI 使用的 BGRA 影像。
pub fn to_image(frame: &Frame) -> Option<Arc<RenderImage>> {
    let mut bgra = frame.rgba.to_vec();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(frame.width as u32, frame.height as u32, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// 影格在顯示區內的位置（不放大，過大時等比縮小並置中）。
/// 回傳 (左偏移, 上偏移, 寬, 高)；顯示區尚未量測（0）時以原尺寸、左上角計算。
pub fn display_rect(view: (f32, f32), frame: (u32, u32)) -> (f32, f32, f32, f32) {
    let (fw, fh) = (frame.0 as f32, frame.1 as f32);
    if fw <= 0.0 || fh <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    if view.0 <= 0.0 || view.1 <= 0.0 {
        return (0.0, 0.0, fw, fh);
    }
    let scale = (view.0 / fw).min(view.1 / fh).min(1.0);
    let (w, h) = (fw * scale, fh * scale);
    ((view.0 - w) / 2.0, (view.1 - h) / 2.0, w, h)
}

/// 顯示區內的位置（相對於顯示區左上角）→ 遠端座標；位於影像外時回傳 None。
pub fn to_remote(local: (f32, f32), view: (f32, f32), frame: (u32, u32)) -> Option<(u32, u32)> {
    let (left, top, w, h) = display_rect(view, frame);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (x, y) = (local.0 - left, local.1 - top);
    if x < 0.0 || y < 0.0 || x >= w || y >= h {
        return None;
    }
    Some((
        ((x * frame.0 as f32 / w) as u32).min(frame.0 - 1),
        ((y * frame.1 as f32 / h) as u32).min(frame.1 - 1),
    ))
}

/// 顯示區改變後是否要調整遠端桌面：太小（< 320×240）或差距不超過 8 像素時不動。
pub fn resize_target(requested: (u32, u32), view: (u32, u32)) -> Option<(u32, u32)> {
    if view.0 < 320 || view.1 < 240 {
        return None;
    }
    (view.0.abs_diff(requested.0) > 8 || view.1.abs_diff(requested.1) > 8).then_some(view)
}

/// 貼到遠端前移除控制字元（保留換行、歸位與 Tab）。
pub fn sanitize_paste(text: &str) -> String {
    text.chars()
        .filter(|c| matches!(c, '\n' | '\r' | '\t') || !c.is_control())
        .collect()
}

pub const PASTE_LIMIT: usize = 256 * 1024;

const CONTROL_L: u32 = 0xffe3;

/// GPUI 按鍵 → X11 keysym 的按下／放開序列；無法對應時回傳空序列。
pub fn key_events(keystroke: &Keystroke) -> Vec<(u32, bool)> {
    let named = match keystroke.key.as_str() {
        "enter" => Some(0xff0d),
        "backspace" => Some(0xff08),
        "tab" => Some(0xff09),
        "escape" => Some(0xff1b),
        "delete" => Some(0xffff),
        "home" => Some(0xff50),
        "left" => Some(0xff51),
        "up" => Some(0xff52),
        "right" => Some(0xff53),
        "down" => Some(0xff54),
        "pageup" => Some(0xff55),
        "pagedown" => Some(0xff56),
        "end" => Some(0xff57),
        "space" => Some(0x20),
        _ => None,
    };
    let control = keystroke.modifiers.control;
    let symbol = named.or_else(|| {
        let text = if control {
            Some(keystroke.key.as_str())
        } else {
            keystroke.key_char.as_deref()
        };
        let mut chars = text?.chars();
        let first = chars.next()?;
        if chars.next().is_some() || first.is_control() {
            return None;
        }
        Some(if (first as u32) <= 0xff {
            first as u32
        } else {
            0x0100_0000 | first as u32
        })
    });
    let Some(symbol) = symbol else {
        return Vec::new();
    };
    if control {
        vec![
            (CONTROL_L, true),
            (symbol, true),
            (symbol, false),
            (CONTROL_L, false),
        ]
    } else {
        vec![(symbol, true), (symbol, false)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::Modifiers;

    fn stroke(key: &str, key_char: Option<&str>, control: bool) -> Keystroke {
        Keystroke {
            modifiers: Modifiers {
                control,
                ..Default::default()
            },
            key: key.to_string(),
            key_char: key_char.map(str::to_string),
        }
    }

    #[test]
    fn paste_text_drops_control_characters_but_keeps_line_breaks() {
        assert_eq!(sanitize_paste("a\u{1b}[31mb\u{7f}\u{0}c\r\n\td"), "a[31mbc\r\n\td");
        assert_eq!(sanitize_paste("遠端測試🙂"), "遠端測試🙂");
        assert_eq!(sanitize_paste("\u{1b}\u{9b}"), "");
    }

    #[test]
    fn remote_resizes_only_for_meaningful_view_changes() {
        assert_eq!(resize_target((960, 540), (960, 540)), None);
        assert_eq!(resize_target((960, 540), (965, 545)), None);
        assert_eq!(resize_target((960, 540), (1200, 700)), Some((1200, 700)));
        assert_eq!(resize_target((960, 540), (100, 700)), None);
        assert_eq!(resize_target((960, 540), (0, 0)), None);
    }

    #[test]
    fn pointer_positions_map_to_remote_pixels() {
        // 顯示區比影格大：置中、不放大。
        assert_eq!(to_remote((340.0 + 42.0, 230.0 + 31.0), (1000.0, 700.0), (320, 240)), Some((42, 31)));
        assert_eq!(display_rect((1000.0, 700.0), (320, 240)), (340.0, 230.0, 320.0, 240.0));
        // 影格比顯示區大：等比縮小。
        assert_eq!(display_rect((480.0, 540.0), (960, 540)), (0.0, 135.0, 480.0, 270.0));
        assert_eq!(to_remote((240.0, 270.0), (480.0, 540.0), (960, 540)), Some((480, 270)));
        // 影像外的位置被忽略。
        assert_eq!(to_remote((10.0, 10.0), (1000.0, 700.0), (320, 240)), None);
        assert_eq!(to_remote((5.0, 5.0), (0.0, 0.0), (320, 240)), Some((5, 5)));
    }

    #[test]
    fn printable_named_and_control_keys_map_to_keysyms() {
        assert_eq!(
            key_events(&stroke("a", Some("a"), false)),
            vec![(0x61, true), (0x61, false)]
        );
        assert_eq!(key_events(&stroke("enter", None, false))[0], (0xff0d, true));
        assert_eq!(
            key_events(&stroke("c", None, true)),
            vec![(CONTROL_L, true), (0x63, true), (0x63, false), (CONTROL_L, false)]
        );
        // 中文字元使用 Unicode keysym。
        assert_eq!(key_events(&stroke("x", Some("中"), false))[0].0, 0x0100_0000 | 0x4e2d);
        assert!(key_events(&stroke("f13", None, false)).is_empty());
    }
}
