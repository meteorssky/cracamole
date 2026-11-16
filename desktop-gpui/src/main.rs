#![recursion_limit = "512"]
//! cracamole 桌面介面（GPUI 版）原型：連線庫畫面。
//! 資料來自 `cracamole-store`（`CRACAMOLE_DESKTOP_DATA_DIR`），未設定時顯示範例連線。

mod remote;

// 與 Win32 版共用同一份私有連線庫（私有 ACL、單一程序鎖）；不複製其邏輯。
#[allow(dead_code)]
#[path = "../../crates/cracamole-desktop/src/windows_private.rs"]
mod windows_private;
#[allow(dead_code)]
#[path = "../../crates/cracamole-desktop/src/windows_store.rs"]
mod windows_store;

use cracamole_store::PublicConnection;
use remote::{Event, RemoteInput};
use cracamole_store::credentials::Credentials;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::test::TestSupportExt;
use gpui_kit::*;
use std::collections::{BTreeMap, BTreeSet};

const BG: u32 = 0x0f1117;
const PANEL: u32 = 0x151821;
const CARD: u32 = 0x1a1e29;
const BORDER: u32 = 0x262b38;
const TEXT: u32 = 0xf4f4f5;
const MUTED: u32 = 0x9aa1b1;
const ACCENT: u32 = 0x7c5cff;
const OK: u32 = 0x22c55e;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    All,
    Pinned,
}

/// 等待使用者核對指紋的 SSH 連線。
struct Pending {
    connection: cracamole_store::Connection,
    trust_param: &'static str,
    trust_value: String,
    fingerprint: String,
    detail: Option<String>,
}

/// 進行中的遠端工作階段。
struct Live {
    input: tokio::sync::mpsc::Sender<RemoteInput>,
    cancel: tokio_util::sync::CancellationToken,
    frame: Option<Arc<RenderImage>>,
    /// 目前影格的像素尺寸。
    size: (u32, u32),
    /// 已要求遠端使用的桌面尺寸。
    requested: (u32, u32),
    /// 目前按下的滑鼠按鈕（Guacamole 遮罩：1 左、2 中、4 右）。
    mask: u32,
    read_only: bool,
    allow_copy: bool,
    allow_paste: bool,
    /// 同時只允許一個貼上作業。
    paste_limit: Arc<tokio::sync::Semaphore>,
    status: String,
    ended: bool,
}

/// 登入前的狀態；未登入時不載入任何連線資料。
struct Auth {
    credentials: Option<Arc<Credentials>>,
    startup_error: Option<String>,
    data_dir: Option<PathBuf>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    error: Option<String>,
    attempts: Vec<Instant>,
    busy: bool,
}

struct Library {
    collapsed: BTreeSet<String>,
    /// 正在更名的群組。
    renaming: Option<String>,
    group_name: Entity<InputState>,
    group: Entity<InputState>,
    /// 正在編輯的既有連線（None 表示新增）。
    editing_id: Option<String>,
    confirm_delete: bool,
    store: Option<Arc<windows_store::WindowsStore>>,
    auth: Option<Auth>,
    secret: Entity<InputState>,
    /// 遠端畫面容器的實際範圍（繪製時量測），供座標換算與調整遠端解析度。
    view: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    pending: Option<Pending>,
    probing: bool,
    notice: Option<String>,
    live: Option<Live>,
    focus: FocusHandle,
    connections: Vec<PublicConnection>,
    pinned: Vec<String>,
    selected: Option<String>,
    tab: Tab,
    search: Entity<InputState>,
    editing: bool,
    protocol: &'static str,
    error: Option<String>,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
}

const PROTOCOLS: [(&str, &str); 4] = [("ssh", "22"), ("rdp", "3389"), ("vnc", "5900"), ("telnet", "23")];

fn sample(id: &str, name: &str, protocol: &str, host: &str, user: &str) -> PublicConnection {
    let mut params = BTreeMap::new();
    params.insert("hostname".to_string(), host.to_string());
    params.insert("username".to_string(), user.to_string());
    PublicConnection {
        id: id.into(),
        name: name.into(),
        protocol: protocol.into(),
        params,
        configured_secrets: vec![],
    }
}

/// 範例連線：僅供開發預覽與測試；正式連線一律來自登入後開啟的私有連線庫。
fn load() -> Vec<PublicConnection> {
    vec![
        sample("1", "prod-web-01", "ssh", "10.0.1.11", "deploy"),
        sample("2", "prod-db-01", "ssh", "10.0.1.21", "dba"),
        sample("3", "win-rdp-01", "rdp", "10.0.2.15", "alice"),
        sample("4", "mac-studio-01", "vnc", "10.0.2.30", "studio"),
        sample("5", "edge-router-01", "telnet", "10.0.3.1", "admin"),
    ]
}

impl Library {
    /// 開發預覽與測試：範例連線、免登入、不持久化。
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_connections(window, cx, load())
    }

    /// 正式啟動：先登入，成功後才開啟私有連線庫並載入連線。
    fn locked(
        window: &mut Window,
        cx: &mut Context<Self>,
        credentials: Result<Arc<Credentials>, String>,
        data_dir: Option<PathBuf>,
    ) -> Self {
        let mut this = Self::with_connections(window, cx, Vec::new());
        let username = cx.new(|cx| InputState::new(window, cx).placeholder("帳號"));
        let password = cx.new(|cx| InputState::new(window, cx).placeholder("密碼").masked(true));
        cx.subscribe_in(&password, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.try_login(window, cx);
            }
        })
        .detach();
        let (credentials, startup_error) = match credentials {
            Ok(credentials) => (Some(credentials), None),
            Err(message) => (None, Some(message)),
        };
        this.auth = Some(Auth {
            credentials,
            startup_error,
            data_dir,
            username,
            password,
            error: None,
            attempts: Vec::new(),
            busy: false,
        });
        this
    }

    fn with_connections(
        window: &mut Window,
        cx: &mut Context<Self>,
        connections: Vec<PublicConnection>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜尋名稱、主機或協定"));
        cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()).detach();
        let secret = cx.new(|cx| InputState::new(window, cx).placeholder("密碼（選填）").masked(true));
        let pinned = connections.iter().take(2).map(|c| c.id.clone()).collect();
        let field = |window: &mut Window, cx: &mut Context<Self>, hint: &'static str| {
            cx.new(|cx| InputState::new(window, cx).placeholder(hint))
        };
        let name = field(window, cx, "連線名稱");
        let host = field(window, cx, "主機名稱或位址");
        let port = field(window, cx, "連接埠（留空使用預設）");
        let user = field(window, cx, "帳號");
        Self {
            collapsed: BTreeSet::new(),
            renaming: None,
            group_name: cx.new(|cx| InputState::new(window, cx).placeholder("新的群組名稱（留空＝移至未分類）")),
            group: cx.new(|cx| InputState::new(window, cx).placeholder("群組（選填）")),
            editing_id: None,
            confirm_delete: false,
            store: None,
            auth: None,
            secret,
            view: Default::default(),
            pending: None,
            probing: false,
            notice: None,
            live: None,
            focus: cx.focus_handle(),
            connections,
            pinned,
            selected: None,
            tab: Tab::All,
            search,
            editing: false,
            protocol: "ssh",
            error: None,
            name,
            host,
            port,
            user,
        }
    }

    fn try_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(auth) = self.auth.as_mut() else {
            return;
        };
        if auth.busy {
            return;
        }
        let Some(credentials) = auth.credentials.clone() else {
            return;
        };
        let now = Instant::now();
        auth.attempts.retain(|at| now.duration_since(*at) < Duration::from_secs(60));
        if auth.attempts.len() >= 10 {
            auth.error = Some("登入次數過多，請稍後再試。".into());
            cx.notify();
            return;
        }
        auth.attempts.push(now);
        let username = auth.username.read(cx).value().to_string();
        let password = auth.password.read(cx).value().to_string();
        auth.password.clone().update(cx, |state, cx| state.set_value("", window, cx));
        auth.busy = true;
        auth.error = None;
        let result = remote::verify(credentials, username, password);
        cx.spawn_in(window, async move |this, cx| {
            let result = result.await;
            let _ = this.update_in(cx, |this, window, cx| this.on_login(result, window, cx));
        })
        .detach();
        cx.notify();
    }

    fn on_login(
        &mut self,
        result: Result<Result<bool, cracamole_store::credentials::VerifyError>, tokio::sync::oneshot::error::RecvError>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(auth) = self.auth.as_mut() else {
            return;
        };
        auth.busy = false;
        match result {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => {
                auth.error = Some("帳號或密碼不正確。".into());
                cx.notify();
                return;
            }
            _ => {
                auth.error = Some("無法驗證帳密，請稍後再試。".into());
                cx.notify();
                return;
            }
        }
        let Some(directory) = auth.data_dir.clone() else {
            auth.error = Some("找不到桌面資料目錄。請設定 CRACAMOLE_DESKTOP_DATA_DIR。".into());
            cx.notify();
            return;
        };
        match remote::block_on(windows_store::WindowsStore::open(directory)) {
            Err(_) => {
                auth.error = Some("無法開啟私有桌面連線庫，請檢查資料目錄權限或其他桌面程式。".into());
            }
            Ok(store) => {
                let store = Arc::new(store);
                self.connections = remote::block_on(store.list()).unwrap_or_default();
                self.pinned = store.pinned().unwrap_or_default().into_iter().collect();
                self.store = Some(store);
                self.auth = None;
            }
        }
        cx.notify();
    }

    fn toggle_pin(&mut self, id: &str, cx: &mut Context<Self>) {
        let pinned = !self.pinned.iter().any(|p| p == id);
        if let Some(store) = &self.store {
            if store.set_pinned(id, pinned).is_err() {
                self.notice = Some("無法儲存釘選狀態。".into());
                cx.notify();
                return;
            }
        }
        self.pinned.retain(|p| p != id);
        if pinned {
            self.pinned.push(id.to_string());
        }
        cx.notify();
    }

    /// 連線到目前選取的項目。
    fn connect_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.live.is_some() {
            return;
        }
        let Some(public) = self
            .connections
            .iter()
            .find(|c| Some(&c.id) == self.selected.as_ref())
        else {
            return;
        };
        let connection = remote::load_connection(public, self.store.as_deref());
        self.notice = None;
        // SSH 沒有已設定的主機金鑰時，先探測並讓使用者核對指紋。
        if remote::needs_identity(&connection) {
            self.begin_probe(connection, window, cx);
            return;
        }
        self.start_session(connection, window, cx);
    }

    fn begin_probe(
        &mut self,
        connection: cracamole_store::Connection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = connection.params.get("hostname").cloned().unwrap_or_default();
        let port = connection
            .params
            .get("port")
            .and_then(|port| port.parse().ok())
            .unwrap_or(if connection.protocol == "rdp" { 3389 } else { 22 });
        self.probing = true;
        let result = remote::probe(connection.protocol.clone(), host, port);
        cx.spawn_in(window, async move |this, cx| {
            let result = result.await.unwrap_or_else(|_| Err("探測已取消。".to_string()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.on_probe(connection, result, window, cx)
            });
        })
        .detach();
        cx.notify();
    }

    fn on_probe(
        &mut self,
        connection: cracamole_store::Connection,
        result: Result<remote::Identity, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.probing = false;
        match result {
            Err(message) => self.notice = Some(message),
            // 系統 known_hosts 已信任且相符：直接連線。
            Ok(info) if info.trusted && info.matches => {
                self.start_session(connection, window, cx);
                return;
            }
            Ok(info) if info.trusted => {
                self.notice = Some("主機金鑰與已信任的不符，已拒絕連線。".to_string());
            }
            Ok(info) => {
                self.pending = Some(Pending {
                    connection,
                    trust_param: info.trust_param,
                    trust_value: info.trust_value,
                    fingerprint: info.fingerprint,
                    detail: info.detail,
                });
            }
        }
        cx.notify();
    }

    /// 使用者核對指紋後信任：金鑰或憑證只用於這一次連線，不寫入任何檔案。
    fn trust_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let mut connection = pending.connection;
        connection
            .params
            .insert(pending.trust_param.into(), pending.trust_value);
        self.start_session(connection, window, cx);
    }

    fn start_session(
        &mut self,
        connection: cracamole_store::Connection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let flag = |key: &str| connection.params.get(key).is_some_and(|v| v == "true");
        let (read_only, allow_copy, allow_paste) = (
            flag("read-only"),
            !flag("disable-copy"),
            !flag("disable-paste"),
        );
        let view = self.view.get();
        let (width, height) = (f32::from(view.size.width), f32::from(view.size.height));
        let requested = if width >= 320.0 && height >= 240.0 {
            (width as u32, height as u32)
        } else {
            (960, 540)
        };
        let mut session = remote::start(connection, requested.0, requested.1);
        self.live = Some(Live {
            input: session.input.clone(),
            cancel: session.cancel.clone(),
            frame: None,
            size: (0, 0),
            requested,
            mask: 0,
            read_only,
            allow_copy,
            allow_paste,
            paste_limit: Arc::new(tokio::sync::Semaphore::new(1)),
            status: "連線中…".into(),
            ended: false,
        });
        window.focus(&self.focus, cx);
        cx.spawn(async move |this, cx| {
            while let Some(event) = session.events.recv().await {
                if this.update(cx, |this, cx| this.on_event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        match event {
            Event::Frame(frame) => {
                if let Some(image) = remote::to_image(&frame) {
                    live.size = (frame.width as u32, frame.height as u32);
                    live.frame = Some(image);
                    live.status = "已連線".into();
                }
            }
            Event::Clipboard(text) => {
                // 遠端複製的文字寫入本機剪貼簿（僅在未停用複製時）。
                if live.allow_copy {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                    live.status = "已更新系統剪貼簿。".into();
                }
            }
            Event::Ended(message) => {
                live.ended = true;
                live.status = message.unwrap_or_else(|| "連線已結束".into());
            }
        }
        cx.notify();
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        if let Some(live) = self.live.take() {
            live.cancel.cancel();
        }
        cx.notify();
    }

    /// 依顯示區大小調整遠端桌面尺寸（尺寸改變超過 8 像素才送出）。
    fn sync_remote_size(&mut self) {
        let view = self.view.get();
        let (width, height) = (
            f32::from(view.size.width) as u32,
            f32::from(view.size.height) as u32,
        );
        let Some(live) = self.live.as_mut().filter(|live| !live.ended && live.frame.is_some()) else {
            return;
        };
        if let Some((width, height)) = remote::resize_target(live.requested, (width, height)) {
            if live
                .input
                .try_send(RemoteInput::Resize { width, height })
                .is_ok()
            {
                live.requested = (width, height);
            }
        }
    }

    /// 滑鼠事件：`button` 為 Some((遮罩位元, 是否按下))；位置換算成遠端座標。
    fn send_mouse(&mut self, position: Point<Pixels>, button: Option<(u32, bool)>) {
        let view = self.view.get();
        let Some(live) = self.live.as_mut().filter(|live| !live.ended) else {
            return;
        };
        let local = (
            f32::from(position.x - view.origin.x),
            f32::from(position.y - view.origin.y),
        );
        let view_size = (f32::from(view.size.width), f32::from(view.size.height));
        let Some((x, y)) = remote::to_remote(local, view_size, live.size) else {
            return;
        };
        if let Some((bit, pressed)) = button {
            if pressed {
                live.mask |= bit;
            } else {
                live.mask &= !bit;
            }
        }
        let _ = live.input.try_send(RemoteInput::Mouse {
            x,
            y,
            mask: live.mask,
        });
    }

    /// 把本機剪貼簿文字貼到遠端：受唯讀與 `disable-paste` 限制，過濾控制字元並限制大小。
    fn paste_local(&mut self, cx: &mut Context<Self>) {
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        let Some(live) = self.live.as_mut().filter(|live| !live.ended) else {
            return;
        };
        live.status = if live.read_only || !live.allow_paste {
            "此連線不允許貼上到遠端。".into()
        } else {
            match text.map(|text| remote::sanitize_paste(&text)) {
                None => "剪貼簿沒有文字。".into(),
                Some(text) if text.is_empty() => "剪貼簿沒有可貼上的文字。".into(),
                Some(text) if text.len() > remote::PASTE_LIMIT => {
                    "剪貼簿超過 256 KiB，請先縮短內容。".into()
                }
                Some(text) => match live.paste_limit.clone().try_acquire_owned() {
                    Err(_) => "上一次貼上尚未完成。".into(),
                    Ok(permit) => {
                        let text = zeroize::Zeroizing::new(text);
                        match live.input.try_send(RemoteInput::Paste { text, permit }) {
                            Ok(()) => "已送出貼上內容。".into(),
                            Err(_) => "遠端輸入佇列已滿，請稍後再試。".into(),
                        }
                    }
                },
            }
        };
        cx.notify();
    }

    fn send_key(&mut self, keystroke: &Keystroke) {
        let Some(live) = self.live.as_ref().filter(|live| !live.ended) else {
            return;
        };
        for (keysym, pressed) in remote::key_events(keystroke) {
            // 佇列已滿時丟棄輸入，不阻塞介面。
            let _ = live.input.try_send(RemoteInput::Key { keysym, pressed });
        }
    }

    /// 以欄位內容建立草稿並以共用規則驗證；成功時加入清單並關閉編輯器。
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = |state: &Entity<InputState>| state.read(cx).value().trim().to_string();
        let default_port = PROTOCOLS
            .iter()
            .find(|(name, _)| *name == self.protocol)
            .map_or("", |(_, port)| *port);
        let port = text(&self.port);
        let port = if port.is_empty() { default_port.to_string() } else { port };
        let secret = self.secret.read(cx).value().to_string();
        let base = self
            .editing_id
            .as_ref()
            .and_then(|id| self.connections.iter().find(|c| &c.id == id));
        let params = merged_params(
            base,
            self.protocol,
            &text(&self.host),
            &port,
            &text(&self.user),
            &secret,
        );
        let (group, member) = (text(&self.group), text(&self.name));
        if !group.is_empty() && (!valid_group(&group) || member.contains('/')) {
            self.error = Some("群組與名稱不可含斜線或控制字元，群組不超過 116 個字元。".into());
            cx.notify();
            return;
        }
        let full_name = if group.is_empty() { member } else { format!("{group} / {member}") };
        let draft = cracamole_store::Draft {
            name: full_name,
            protocol: self.protocol.to_string(),
            params,
        };
        let outcome: Result<(), String> = if let Some(store) = self.store.clone() {
            // 登入後：由私有連線庫驗證並寫入，再重新載入清單。
            match remote::block_on(store.save(self.editing_id.as_deref(), draft)) {
                Ok(Some(_)) => {
                    self.connections = remote::block_on(store.list()).unwrap_or_default();
                    Ok(())
                }
                Ok(None) => Err("找不到要更新的連線。".to_string()),
                Err(windows_store::SaveError::Invalid(invalid)) => Err(invalid.message.to_string()),
                Err(windows_store::SaveError::Io(_)) => Err("無法儲存到私有連線庫。".to_string()),
            }
        } else {
            cracamole_store::validate(&draft)
                .map_err(|invalid| invalid.message.to_string())
                .map(|()| {
                    let existing = self
                        .editing_id
                        .as_ref()
                        .and_then(|id| self.connections.iter_mut().find(|c| &c.id == id));
                    if let Some(connection) = existing {
                        connection.name = draft.name;
                        connection.protocol = draft.protocol;
                        connection.params = draft.params;
                    } else {
                        self.connections.push(PublicConnection {
                            id: format!("new-{}", self.connections.len() + 1),
                            name: draft.name,
                            protocol: draft.protocol,
                            params: draft.params,
                            configured_secrets: vec![],
                        });
                    }
                })
        };
        match outcome {
            Err(message) => self.error = Some(message),
            Ok(()) => {
                self.editing = false;
                self.editing_id = None;
                self.error = None;
                self.clear_editor(window, cx);
            }
        }
        cx.notify();
    }

    /// 以選取的連線預填編輯器；密碼欄位留空表示保留原值。
    fn edit_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self
            .connections
            .iter()
            .find(|c| Some(&c.id) == self.selected.as_ref())
        else {
            return;
        };
        let value = |key: &str| connection.params.get(key).cloned().unwrap_or_default();
        let (id, name, protocol) = (
            connection.id.clone(),
            connection.name.clone(),
            connection.protocol.clone(),
        );
        let (group, member) = split_group(&name);
        let (group, member) = (group.unwrap_or("").to_string(), member.to_string());
        let fields = [
            (member, &self.name),
            (group, &self.group),
            (value("hostname"), &self.host),
            (value("port"), &self.port),
            (value("username"), &self.user),
            (String::new(), &self.secret),
        ];
        for (text, state) in fields {
            state.clone().update(cx, |state, cx| state.set_value(text, window, cx));
        }
        self.protocol = PROTOCOLS
            .iter()
            .find(|(name, _)| *name == protocol)
            .map_or("ssh", |(name, _)| *name);
        self.editing = true;
        self.editing_id = Some(id);
        self.confirm_delete = false;
        self.error = None;
        cx.notify();
    }

    /// 刪除選取的連線（已在介面確認過）。
    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        self.confirm_delete = false;
        let Some(id) = self.selected.clone() else {
            return;
        };
        if let Some(store) = self.store.clone() {
            match remote::block_on(store.delete(&id)) {
                Ok(_) => {
                    self.connections = remote::block_on(store.list()).unwrap_or_default();
                    let _ = store.set_pinned(&id, false);
                }
                Err(_) => {
                    self.notice = Some("無法從私有連線庫刪除。".into());
                    cx.notify();
                    return;
                }
            }
        } else {
            self.connections.retain(|c| c.id != id);
        }
        self.pinned.retain(|p| p != &id);
        self.selected = None;
        self.notice = None;
        cx.notify();
    }

    /// 清空編輯器欄位（含密碼）。
    fn clear_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for state in [&self.name, &self.group, &self.host, &self.port, &self.user, &self.secret] {
            state.clone().update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    fn visible(&self, cx: &App) -> Vec<PublicConnection> {
        let query = self.search.read(cx).value().to_lowercase();
        self.connections
            .iter()
            .filter(|c| self.tab == Tab::All || self.pinned.contains(&c.id))
            .filter(|c| {
                query.is_empty()
                    || c.name.to_lowercase().contains(&query)
                    || c.protocol.contains(&query)
                    || c.params.get("hostname").is_some_and(|h| h.contains(&query))
            })
            .map(|c| PublicConnection {
                id: c.id.clone(),
                name: c.name.clone(),
                protocol: c.protocol.clone(),
                params: c.params.clone(),
                configured_secrets: c.configured_secrets.clone(),
            })
            .collect()
    }
}

impl Library {
    fn login_view(&self, cx: &mut Context<Self>) -> Div {
        let auth = self.auth.as_ref().expect("login view requires auth state");
        let label = |text: &'static str| div().text_size(px(13.)).text_color(rgb(MUTED)).child(text);
        let message = auth.startup_error.clone().or_else(|| auth.error.clone());
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(BG))
            .child(
                div()
                    .w(px(420.))
                    .p_6()
                    .rounded(px(16.))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(CARD))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_size(px(22.)).text_color(rgb(TEXT)).child("cracamole"))
                    .child(label("請以管理員帳號登入。"))
                    .child(label("帳號"))
                    .child(Input::new(&auth.username))
                    .child(label("密碼"))
                    .child(Input::new(&auth.password))
                    .children(message.map(|message| {
                        div().text_size(px(13.)).text_color(rgb(0xf87171)).child(message)
                    }))
                    .child(
                        div()
                            .id("login")
                            .h(px(40.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(10.))
                            .bg(rgb(if auth.busy || auth.credentials.is_none() { CARD } else { ACCENT }))
                            .text_color(rgb(0xffffff))
                            .cursor_pointer()
                            .child(if auth.busy { "驗證中…" } else { "登入" })
                            .on_click(cx.listener(|this, _, window, cx| this.try_login(window, cx)))
                            .test_support()
                            .aria_label("登入"),
                    ),
            )
    }

    fn trust_card(&self, pending: &Pending, cx: &mut Context<Self>) -> Div {
        let host = pending.connection.params.get("hostname").cloned().unwrap_or_default();
        let button = |id: &'static str, text: &'static str, primary: bool| {
            div()
                .id(id)
                .px_5()
                .h(px(38.))
                .flex()
                .items_center()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(if primary { ACCENT } else { BORDER }))
                .bg(rgb(if primary { ACCENT } else { PANEL }))
                .text_color(rgb(if primary { 0xffffff } else { TEXT }))
                .cursor_pointer()
                .child(text)
        };
        div()
            .w(px(560.))
            .p_6()
            .rounded(px(16.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(CARD))
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_size(px(18.)).text_color(rgb(TEXT)).child("核對主機身分"))
            .child(
                div()
                    .text_color(rgb(MUTED))
                    .child(format!("尚未信任 {host} 的身分。請與伺服器管理者透過可信管道提供的指紋比對，一致後才信任。")),
            )
            .children(pending.detail.clone().map(|detail| {
                div().text_size(px(13.)).text_color(rgb(MUTED)).child(format!("憑證主體：{detail}"))
            }))
            .child(
                div()
                    .id("fingerprint")
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .bg(rgb(BG))
                    .text_color(rgb(TEXT))
                    .child(pending.fingerprint.clone())
                    .test_support()
                    .aria_label(pending.fingerprint.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        button("trust-cancel", "取消", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pending = None;
                                cx.notify();
                            }))
                            .test_support()
                            .aria_label("取消"),
                    )
                    .child(
                        button("trust", "信任並連線", true)
                            .on_click(cx.listener(|this, _, window, cx| this.trust_pending(window, cx)))
                            .test_support()
                            .aria_label("信任並連線"),
                    ),
            )
    }

    /// 群組標題：點選收合／展開；「更名」可重新命名整個群組。
    fn group_header(
        &self,
        group: Option<&str>,
        count: usize,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = group.unwrap_or("未分類").to_string();
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .pt_2()
            .text_size(px(13.))
            .text_color(rgb(MUTED));
        let toggle_name = title.clone();
        let toggle = div()
            .id(SharedString::from(format!("group-{title}")))
            .flex_1()
            .cursor_pointer()
            .child(format!("{} {title}（{count}）", if collapsed { "▸" } else { "▾" }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.collapsed.remove(&toggle_name) {
                    this.collapsed.insert(toggle_name.clone());
                }
                cx.notify();
            }))
            .test_support()
            .aria_label(title.clone());
        header = header.child(toggle);
        if let Some(group) = group {
            let old = group.to_string();
            header = header.child(
                div()
                    .id(SharedString::from(format!("rename-{group}")))
                    .px_2()
                    .cursor_pointer()
                    .child("更名")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.group_name
                            .clone()
                            .update(cx, |state, cx| state.set_value(old.clone(), window, cx));
                        this.renaming = Some(old.clone());
                        this.error = None;
                        cx.notify();
                    }))
                    .test_support()
                    .aria_label(format!("更名群組 {group}")),
            );
        }
        header.into_any_element()
    }

    fn connection_row(
        &self,
        c: &PublicConnection,
        label: &str,
        selected: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = c.id.clone();
        let is_selected = selected == Some(c.id.as_str());
        let is_pinned = self.pinned.contains(&c.id);
        let host = c.params.get("hostname").cloned().unwrap_or_default();
        let user = c.params.get("username").cloned().unwrap_or_default();
        div()
            .id(SharedString::from(format!("row-{}", c.id)))
            .test_support()
            .aria_label(c.name.clone())
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(if is_selected { ACCENT } else { PANEL }))
            .bg(rgb(if is_selected { 0x1e1b3a } else { PANEL }))
            .hover(|s| s.bg(rgb(CARD)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(id.clone());
                cx.notify();
            }))
            .child(div().size(px(8.)).rounded_full().bg(rgb(MUTED)))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .bg(rgb(0x20243a))
                    .text_color(rgb(protocol_color(&c.protocol)))
                    .text_size(px(12.))
                    .child(c.protocol.to_uppercase()),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(div().text_color(rgb(TEXT)).child(label.to_string()))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(MUTED))
                            .child(format!("{user}@{host}")),
                    ),
            )
            .child({
                let pin_id = c.id.clone();
                div()
                    .id(SharedString::from(format!("pin-{}", c.id)))
                    .px_1()
                    .cursor_pointer()
                    .text_color(rgb(if is_pinned { 0xfbbf24 } else { MUTED }))
                    .child(if is_pinned { "★" } else { "☆" })
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_pin(&pin_id, cx)))
                    .test_support()
                    .aria_label(if is_pinned { "取消釘選" } else { "釘選" })
            })
            .into_any_element()
    }

    /// 群組更名卡片；新名稱留空表示把該群組的連線移至未分類。
    fn rename_card(&self, old: &str, cx: &mut Context<Self>) -> Div {
        let button = |id: &'static str, text: &'static str, primary: bool| {
            div()
                .id(id)
                .px_5()
                .h(px(38.))
                .flex()
                .items_center()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(if primary { ACCENT } else { BORDER }))
                .bg(rgb(if primary { ACCENT } else { PANEL }))
                .text_color(rgb(if primary { 0xffffff } else { TEXT }))
                .cursor_pointer()
                .child(text)
        };
        div()
            .w(px(520.))
            .p_6()
            .rounded(px(16.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(CARD))
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_size(px(18.)).text_color(rgb(TEXT)).child(format!("更名群組「{old}」")))
            .child(Input::new(&self.group_name))
            .children(self.error.clone().map(|message| {
                div().text_size(px(13.)).text_color(rgb(0xf87171)).child(message)
            }))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        button("rename-cancel", "取消", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.renaming = None;
                                this.error = None;
                                cx.notify();
                            }))
                            .test_support()
                            .aria_label("取消更名"),
                    )
                    .child(
                        button("rename-save", "儲存", true)
                            .on_click(cx.listener(|this, _, _, cx| this.rename_group(cx)))
                            .test_support()
                            .aria_label("儲存群組名稱"),
                    ),
            )
    }

    fn rename_group(&mut self, cx: &mut Context<Self>) {
        let Some(old) = self.renaming.clone() else {
            return;
        };
        let new = self.group_name.read(cx).value().trim().to_string();
        let outcome: Result<(), String> = if let Some(store) = self.store.clone() {
            match remote::block_on(store.rename_group(&old, &new)) {
                Ok(_) => {
                    self.connections = remote::block_on(store.list()).unwrap_or_default();
                    Ok(())
                }
                Err(windows_store::SaveError::Invalid(invalid)) => Err(invalid.message.to_string()),
                Err(windows_store::SaveError::Io(_)) => Err("無法儲存到私有連線庫。".to_string()),
            }
        } else if !valid_group(&new) {
            Err("群組名稱不可含斜線或控制字元，且不超過 116 個字元。".to_string())
        } else {
            for connection in &mut self.connections {
                let (group, member) = split_group(&connection.name);
                if group == Some(old.as_str()) {
                    connection.name = if new.is_empty() {
                        member.to_string()
                    } else {
                        format!("{new} / {member}")
                    };
                }
            }
            Ok(())
        };
        match outcome {
            Err(message) => self.error = Some(message),
            Ok(()) => {
                if self.collapsed.remove(&old) && !new.is_empty() {
                    self.collapsed.insert(new);
                }
                self.renaming = None;
                self.error = None;
            }
        }
        cx.notify();
    }

    /// 連線中才顯示：把本機剪貼簿貼到遠端。
    fn paste_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(live) = self.live.as_ref().filter(|live| !live.ended) else {
            return div().into_any_element();
        };
        let allowed = !live.read_only && live.allow_paste;
        div()
            .id("paste")
            .px_4()
            .h(px(36.))
            .flex()
            .items_center()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(PANEL))
            .text_color(rgb(if allowed { TEXT } else { MUTED }))
            .cursor_pointer()
            .child("貼上")
            .on_click(cx.listener(|this, _, _, cx| this.paste_local(cx)))
            .test_support()
            .aria_label("貼上到遠端")
            .into_any_element()
    }

    /// 編輯與刪除（刪除需再次確認）；未選取、連線中或編輯中時不顯示。
    fn manage_buttons(&self, has_selection: bool, cx: &mut Context<Self>) -> AnyElement {
        if !has_selection || self.live.is_some() || self.editing || self.pending.is_some() {
            return div().into_any_element();
        }
        let small = |id: &'static str, text: &'static str, danger: bool| {
            div()
                .id(id)
                .px_4()
                .h(px(36.))
                .flex()
                .items_center()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(if danger { 0x7f1d1d } else { BORDER }))
                .bg(rgb(if danger { 0x3b1d24 } else { PANEL }))
                .text_color(rgb(if danger { 0xfca5a5 } else { TEXT }))
                .cursor_pointer()
                .child(text)
        };
        if self.confirm_delete {
            return div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().text_size(px(13.)).text_color(rgb(0xfca5a5)).child("確認刪除這筆連線？"))
                .child(
                    small("delete-confirm", "刪除", true)
                        .on_click(cx.listener(|this, _, _, cx| this.delete_selected(cx)))
                        .test_support()
                        .aria_label("確認刪除"),
                )
                .child(
                    small("delete-cancel", "取消", false)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_delete = false;
                            cx.notify();
                        }))
                        .test_support()
                        .aria_label("取消刪除"),
                )
                .into_any_element();
        }
        div()
            .flex()
            .gap_2()
            .child(
                small("edit", "編輯", false)
                    .on_click(cx.listener(|this, _, window, cx| this.edit_selected(window, cx)))
                    .test_support()
                    .aria_label("編輯"),
            )
            .child(
                small("delete", "刪除", true)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_delete = true;
                        cx.notify();
                    }))
                    .test_support()
                    .aria_label("刪除"),
            )
            .into_any_element()
    }

    fn action_button(&self, can_connect: bool, cx: &mut Context<Self>) -> AnyElement {
        let live = self.live.is_some();
        let text = if live { "中斷" } else { "連線" };
        // 單一元素識別：點擊途中重繪也不會落到另一個按鈕。
        let id = "session-action";
        let enabled = live || can_connect;
        let button = div()
            .id(id)
            .px_5()
            .h(px(36.))
            .flex()
            .items_center()
            .rounded(px(10.))
            .bg(rgb(if live { 0x3b1d24 } else if enabled { ACCENT } else { CARD }))
            .text_color(rgb(if enabled { 0xffffff } else { MUTED }))
            .child(text);
        if !enabled {
            return button.test_support().aria_label(text).into_any_element();
        }
        button
            .cursor_pointer()
            .on_click(cx.listener(|this, _, window, cx| {
                // 以點擊當下的狀態決定動作。
                if this.live.is_some() {
                    this.disconnect(cx);
                } else {
                    this.connect_selected(window, cx);
                }
            }))
            .test_support()
            .aria_label(text)
            .into_any_element()
    }

    fn editor_card(&self, cx: &mut Context<Self>) -> Div {
        let label = |text: &'static str| div().text_size(px(13.)).text_color(rgb(MUTED)).child(text);
        let mut protocols = div().flex().gap_2();
        for (name, _) in PROTOCOLS {
            let id: &'static str = match name {
                "ssh" => "protocol-ssh",
                "rdp" => "protocol-rdp",
                "vnc" => "protocol-vnc",
                _ => "protocol-telnet",
            };
            protocols = protocols.child(
                pill(id, name.to_uppercase(), self.protocol == name)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.protocol = name;
                        cx.notify();
                    }))
                    .test_support()
                    .aria_label(name),
            );
        }
        let button = |id: &'static str, text: &'static str, primary: bool| {
            div()
                .id(id)
                .px_5()
                .h(px(38.))
                .flex()
                .items_center()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(if primary { ACCENT } else { BORDER }))
                .bg(rgb(if primary { ACCENT } else { PANEL }))
                .text_color(rgb(if primary { 0xffffff } else { TEXT }))
                .cursor_pointer()
                .child(text)
        };
        div()
            .w(px(520.))
            .p_6()
            .rounded(px(16.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(CARD))
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_size(px(18.)).text_color(rgb(TEXT)).child(if self.editing_id.is_some() { "編輯連線" } else { "新增連線" }))
            .child(label("協定"))
            .child(protocols)
            .child(label("群組（選填）"))
            .child(Input::new(&self.group))
            .child(label("名稱"))
            .child(Input::new(&self.name))
            .child(label("主機"))
            .child(Input::new(&self.host))
            .child(label("密碼（選填，保存於私有連線庫）"))
            .child(Input::new(&self.secret))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(label("連接埠"))
                            .child(Input::new(&self.port)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(label("帳號"))
                            .child(Input::new(&self.user)),
                    ),
            )
            .children(self.error.clone().map(|message| {
                div().text_size(px(13.)).text_color(rgb(0xf87171)).child(message)
            }))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        button("cancel", "取消", false)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.editing = false;
                                this.editing_id = None;
                                this.error = None;
                                this.clear_editor(window, cx);
                                cx.notify();
                            }))
                            .test_support()
                            .aria_label("取消"),
                    )
                    .child(
                        button("save", "儲存", true)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))
                            .test_support()
                            .aria_label("儲存"),
                    ),
            )
    }
}

/// 連線名稱的群組：`群組/成員`；沒有群組（或群組名稱為空）時回傳 (None, 原名稱)。
fn split_group(name: &str) -> (Option<&str>, &str) {
    match name.split_once('/') {
        Some((group, member)) if !group.trim().is_empty() => (Some(group.trim()), member.trim()),
        _ => (None, name),
    }
}

/// 群組名稱是否合法：不可含斜線或控制字元，長度不超過 116。
fn valid_group(group: &str) -> bool {
    group.chars().count() <= 116 && !group.contains('/') && !group.chars().any(char::is_control)
}

/// 編輯時的參數：保留未在表單出現的設定（網域、安全模式等）；
/// 協定或目的地改變時移除舊的信任（host-key／server-cert-sha256），避免沿用到新主機。
fn merged_params(
    base: Option<&PublicConnection>,
    protocol: &str,
    host: &str,
    port: &str,
    user: &str,
    secret: &str,
) -> BTreeMap<String, String> {
    let mut params = base.map(|b| b.params.clone()).unwrap_or_default();
    let destination_changed = base.is_some_and(|b| {
        b.protocol != protocol
            || b.params.get("hostname").map(String::as_str) != Some(host)
            || b.params.get("port").map(String::as_str) != Some(port)
    });
    if destination_changed {
        params.remove("host-key");
        params.remove("server-cert-sha256");
    }
    params.insert("hostname".to_string(), host.to_string());
    params.insert("port".to_string(), port.to_string());
    if user.is_empty() {
        params.remove("username");
    } else {
        params.insert("username".to_string(), user.to_string());
    }
    if !secret.is_empty() {
        params.insert("password".to_string(), secret.to_string());
    }
    params
}

fn protocol_color(protocol: &str) -> u32 {
    match protocol {
        "ssh" => 0x60a5fa,
        "rdp" => 0x22d3ee,
        "vnc" => 0xfbbf24,
        _ => 0xc4b5fd,
    }
}

fn pill(id: &'static str, label: String, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .h(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(px(10.))
        .border_1()
        .border_color(rgb(if active { ACCENT } else { BORDER }))
        .bg(rgb(if active { 0x24204a } else { PANEL }))
        .text_color(rgb(if active { TEXT } else { MUTED }))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x20243a)))
        .child(label)
}

impl Render for Library {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.auth.is_some() {
            return self.login_view(cx).into_any_element();
        }
        self.sync_remote_size();
        let rows = self.visible(cx);
        let total = self.connections.len();
        let pinned = self.pinned.len();
        let selected = self.selected.clone();
        let selected_name = rows
            .iter()
            .find(|c| Some(&c.id) == selected.as_ref())
            .map(|c| c.name.clone());

        let searching = !self.search.read(cx).value().is_empty();
        let mut ungrouped: Vec<&PublicConnection> = Vec::new();
        let mut groups: BTreeMap<String, Vec<&PublicConnection>> = BTreeMap::new();
        for c in &rows {
            match split_group(&c.name).0 {
                Some(group) => groups.entry(group.to_string()).or_default().push(c),
                None => ungrouped.push(c),
            }
        }
        let mut list = div()
            .id("list")
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .overflow_y_scroll()
            .flex_1();
        if !groups.is_empty() && !ungrouped.is_empty() {
            list = list.child(self.group_header(None, ungrouped.len(), false, cx));
        }
        for c in ungrouped {
            list = list.child(self.connection_row(c, &c.name, selected.as_deref(), cx));
        }
        for (group, members) in groups {
            let collapsed = !searching && self.collapsed.contains(&group);
            list = list.child(self.group_header(Some(&group), members.len(), collapsed, cx));
            if !collapsed {
                for c in members {
                    let label = split_group(&c.name).1.to_string();
                    list = list.child(self.connection_row(c, &label, selected.as_deref(), cx));
                }
            }
        }

        let sidebar = div()
            .w(px(380.))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_4()
                    .child(div().size(px(10.)).rounded_full().bg(rgb(ACCENT)))
                    .child(div().text_size(px(20.)).text_color(rgb(TEXT)).child("cracamole")),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_4()
                    .pb_4()
                    .child(div().flex_1().child(Input::new(&self.search)))
                    .child(
                        div()
                            .id("add")
                            .px_4()
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .rounded(px(10.))
                            .bg(rgb(ACCENT))
                            .text_color(rgb(0xffffff))
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.9))
                            .child("新增")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.editing = true;
                                this.editing_id = None;
                                this.confirm_delete = false;
                                this.error = None;
                                cx.notify();
                            }))
                            .test_support()
                            .aria_label("新增連線"),
                    ),
            )
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .text_size(px(13.))
                    .text_color(rgb(MUTED))
                    .child("連線庫"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_4()
                    .pb_3()
                    .child(
                        pill("tab-all", format!("全部  {total}"), self.tab == Tab::All).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.tab = Tab::All;
                                cx.notify();
                            }),
                        )
                        .test_support()
                        .aria_label("全部"),
                    )
                    .child(
                        pill("tab-pinned", format!("已釘選  {pinned}"), self.tab == Tab::Pinned)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab = Tab::Pinned;
                                cx.notify();
                            }))
                            .test_support()
                            .aria_label("已釘選"),
                    ),
            )
            .child(div().h(px(1.)).bg(rgb(BORDER)))
            .child(list);

        let editor = self.editor_card(cx);
        let subtitle = if selected_name.is_some() {
            "按「連線」開始工作階段"
        } else {
            "從左側連線庫選擇主機開始工作"
        };
        let stage = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .py_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .text_color(rgb(TEXT))
                                    .child(selected_name.clone().unwrap_or_else(|| "尚未選擇連線".into())),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(MUTED))
                                    .child(if self.probing {
                                        "正在核對主機身分…".to_string()
                                    } else if let Some(notice) = &self.notice {
                                        notice.clone()
                                    } else {
                                        self.live.as_ref().map_or(subtitle.to_string(), |l| l.status.clone())
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.manage_buttons(selected_name.is_some(), cx))
                            .child(self.paste_button(cx))
                            .child(self.action_button(selected_name.is_some(), cx)),
                    ),
            )
            .child(
                div().flex_1().flex().items_center().justify_center().child(if let Some(live) = &self.live {
                    div()
                        .id("session")
                        .track_focus(&self.focus)
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| {
                            this.send_key(&event.keystroke);
                        }))
                        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, _| {
                            this.send_mouse(event.position, None);
                        }))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.send_mouse(event.position, Some((1, true)));
                        }))
                        .on_mouse_up(MouseButton::Left, cx.listener(|this, event: &MouseUpEvent, _, _| {
                            this.send_mouse(event.position, Some((1, false)));
                        }))
                        .on_mouse_down(MouseButton::Middle, cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.send_mouse(event.position, Some((2, true)));
                        }))
                        .on_mouse_up(MouseButton::Middle, cx.listener(|this, event: &MouseUpEvent, _, _| {
                            this.send_mouse(event.position, Some((2, false)));
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.send_mouse(event.position, Some((4, true)));
                        }))
                        .on_mouse_up(MouseButton::Right, cx.listener(|this, event: &MouseUpEvent, _, _| {
                            this.send_mouse(event.position, Some((4, false)));
                        }))
                        .child({
                            let view = self.view.clone();
                            canvas(move |bounds, _, _| view.set(bounds), |_, _, _, _| {})
                                .absolute()
                                .size_full()
                        })
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgb(0x000000))
                        .child(match &live.frame {
                            Some(image) => {
                                let view = self.view.get();
                                let (_, _, w, h) = remote::display_rect(
                                    (f32::from(view.size.width), f32::from(view.size.height)),
                                    live.size,
                                );
                                img(image.clone()).w(px(w)).h(px(h)).into_any_element()
                            }
                            None => div().text_color(rgb(MUTED)).child(live.status.clone()).into_any_element(),
                        })
                        .test_support()
                        .aria_label("遠端工作階段")
                        .into_any_element()
                } else if let Some(pending) = &self.pending {
                    self.trust_card(pending, cx).into_any_element()
                } else if let Some(old) = &self.renaming {
                    self.rename_card(old, cx).into_any_element()
                } else if self.editing {
                    editor.into_any_element()
                } else {
                    div()
                        .w(px(520.))
                        .p_6()
                        .rounded(px(16.))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(CARD))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .child(div().size(px(10.)).rounded_full().bg(rgb(OK)))
                        .child(
                            div()
                                .text_size(px(20.))
                                .text_color(rgb(TEXT))
                                .child("歡迎使用 cracamole"),
                        )
                        .child(
                            div()
                                .text_color(rgb(MUTED))
                                .child("選擇左側連線，或搜尋主機與輸入連線位址即可開始。"),
                        )
                        .into_any_element()
                }),
            );

        div()
            .size_full()
            .flex()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .child(sidebar)
            .child(stage)
            .into_any_element()
    }
}

fn main() {
    let username = std::env::var("CRACAMOLE_ADMIN_USERNAME").unwrap_or_else(|_| "admin".into());
    let credentials = match std::env::var("CRACAMOLE_ADMIN_PASSWORD") {
        Ok(password) => remote::block_on(Credentials::new(username, password))
            .map(Arc::new),
        Err(_) => Err("請先設定 CRACAMOLE_ADMIN_PASSWORD（至少 12 字元）再啟動。".to_string()),
    };
    let data_dir = std::env::var_os("CRACAMOLE_DESKTOP_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("cracamole-desktop"))
        });
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
            let view = cx.new(|cx| Library::locked(window, cx, credentials, data_dir));
            cx.new(|cx| gpui_kit::component::Root::new(view, window, cx))
        })
        .expect("window");
    });
}

#[cfg(test)]
mod tests {
    use super::Library;
    use gpui_kit::test::{TestAppContextExt, TestWindowExt};
    use gpui_kit::{
        px, size, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions,
    };

    fn open(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<Library>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let (window, content) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1200.), px(760.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Library::new(window, cx)),
            )
            .expect("open test window");
            (window.into(), content)
        })
    }

    #[gpui_kit::test]
    fn library_lists_sample_connections_and_selects_on_click(cx: &mut TestAppContext) {
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(content.read(cx).connections.len(), 5);
            window.click("row-2", cx);
            window.render_frame(cx);
            assert_eq!(content.read(cx).selected.as_deref(), Some("2"));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn pinned_tab_filters_the_list(cx: &mut TestAppContext) {
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(content.read(cx).visible(cx).len(), 5);
            window.click("tab-pinned", cx);
            window.render_frame(cx);
            assert_eq!(content.read(cx).visible(cx).len(), 2);
            assert!(window.try_find("row-5").is_none());
            window.click("tab-all", cx);
            window.render_frame(cx);
            assert_eq!(content.read(cx).visible(cx).len(), 5);
        })
        .unwrap();
    }

    fn fill(
        content: &Entity<Library>,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::App,
        values: [&str; 4],
    ) {
        let (name, host, port, user) = {
            let library = content.read(cx);
            (
                library.name.clone(),
                library.host.clone(),
                library.port.clone(),
                library.user.clone(),
            )
        };
        for (state, value) in [name, host, port, user].into_iter().zip(values) {
            state.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    #[gpui_kit::test]
    fn editor_rejects_invalid_input_and_saves_a_valid_connection(cx: &mut TestAppContext) {
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("add", cx);
            window.render_frame(cx);
            assert!(content.read(cx).editing);

            // 缺少主機與名稱：驗證失敗且不新增。
            window.click("save", cx);
            window.render_frame(cx);
            assert!(content.read(cx).error.is_some());
            assert_eq!(content.read(cx).connections.len(), 5);

            // 無效連接埠同樣被共用驗證拒絕。
            fill(&content, window, cx, ["lab-rdp", "10.9.9.9", "99999", ""]);
            window.click("protocol-rdp", cx);
            window.click("save", cx);
            window.render_frame(cx);
            assert!(content.read(cx).error.is_some());
            assert_eq!(content.read(cx).connections.len(), 5);

            // 有效輸入：使用 RDP 預設連接埠並關閉編輯器。
            fill(&content, window, cx, ["lab-rdp", "10.9.9.9", "", "bob"]);
            window.click("save", cx);
            window.render_frame(cx);
            let library = content.read(cx);
            assert!(!library.editing && library.error.is_none());
            let saved = library.connections.last().unwrap();
            assert_eq!(saved.protocol, "rdp");
            assert_eq!(saved.params.get("port").map(String::as_str), Some("3389"));
            assert_eq!(saved.params.get("username").map(String::as_str), Some("bob"));
            assert_eq!(library.connections.len(), 6);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn cancel_closes_the_editor_without_saving(cx: &mut TestAppContext) {
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("add", cx);
            window.render_frame(cx);
            window.click("cancel", cx);
            window.render_frame(cx);
            assert!(!content.read(cx).editing);
            assert_eq!(content.read(cx).connections.len(), 5);
        })
        .unwrap();
    }

    #[path = "../../../crates/proto-ssh/tests/common/mod.rs"]
    mod ssh_fixture;

    fn ssh_sample(port: u16) -> cracamole_store::PublicConnection {
        let mut connection = super::sample("s1", "lab-ssh", "ssh", "127.0.0.1", "alice");
        connection.params.insert("port".into(), port.to_string());
        connection.params.insert("password".into(), "secret".into());
        connection
    }

    #[gpui_kit::test]
    fn ssh_requires_fingerprint_confirmation_before_authenticating(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let server = super::remote::block_on(ssh_fixture::start_server(Default::default()));
        let expected = server
            .host_key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string();
        let port = server.addr.port();
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            content.update(cx, |library, _| {
                library.connections.push(ssh_sample(port));
                library.selected = Some("s1".into());
            });
            window.render_frame(cx);
            window.click("session-action", cx);
        })
        .unwrap();

        wait_until(cx, "出現指紋核對", |cx| {
            cx.read_entity(&content, |library, _| library.pending.is_some())
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(content.read(cx).pending.as_ref().unwrap().fingerprint, expected);
            assert!(window.try_find("fingerprint").is_some());
            // 核對前不得傳送任何驗證資訊，也不得建立工作階段。
            assert!(server.rec.lock().unwrap().auth_methods_tried.is_empty());
            assert!(content.read(cx).live.is_none());
            window.click("trust-cancel", cx);
            window.render_frame(cx);
            assert!(content.read(cx).pending.is_none());
        })
        .unwrap();
        assert!(server.rec.lock().unwrap().auth_methods_tried.is_empty());
    }

    #[gpui_kit::test]
    fn ssh_session_connects_after_trusting_the_fingerprint(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let server = super::remote::block_on(ssh_fixture::start_server(Default::default()));
        let port = server.addr.port();
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            content.update(cx, |library, _| {
                library.connections.push(ssh_sample(port));
                library.selected = Some("s1".into());
            });
            window.render_frame(cx);
            window.click("session-action", cx);
        })
        .unwrap();
        wait_until(cx, "出現指紋核對", |cx| {
            cx.read_entity(&content, |library, _| library.pending.is_some())
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("trust", cx);
        })
        .unwrap();
        wait_until(cx, "收到 SSH 畫面", |cx| {
            cx.read_entity(&content, |library, _| {
                library.live.as_ref().is_some_and(|live| live.frame.is_some())
            })
        });
        wait_until(cx, "伺服器記錄 shell", |_| server.rec.lock().unwrap().shell);
        assert_eq!(
            server.rec.lock().unwrap().auth_user.as_deref(),
            Some("alice")
        );
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("y", cx);
        })
        .unwrap();
        wait_until(cx, "伺服器收到按鍵", |_| {
            server.rec.lock().unwrap().received.contains(&b'y')
        });
    }

    /// 程序內的 Telnet 伺服器：送出一行文字，並記錄收到的位元組。
    fn telnet_fixture() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = received.clone();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.write_all(b"hello cracamole\r\n$ ");
                let mut buffer = [0u8; 256];
                while let Ok(count) = stream.read(&mut buffer) {
                    if count == 0 {
                        break;
                    }
                    sink.lock().unwrap().extend_from_slice(&buffer[..count]);
                }
            }
        });
        (port, received)
    }

    fn wait_until(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        for _ in 0..200 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("逾時：{what}");
    }

    #[gpui_kit::test]
    fn telnet_session_shows_frames_sends_keys_and_disconnects(cx: &mut TestAppContext) {
        // 測試含真實的網路 I/O，允許其他執行緒喚醒 GPUI 任務。
        cx.executor().allow_parking();
        let (port, received) = telnet_fixture();
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            content.update(cx, |library, _| {
                let mut connection = super::sample("t1", "lab-telnet", "telnet", "127.0.0.1", "");
                connection.params.insert("port".into(), port.to_string());
                library.connections.push(connection);
                library.selected = Some("t1".into());
            });
            window.render_frame(cx);
            window.click("session-action", cx);
        })
        .unwrap();

        wait_until(cx, "收到第一張畫面", |cx| {
            cx.read_entity(&content, |library, _| {
                library.live.as_ref().is_some_and(|live| live.frame.is_some())
            })
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("session").is_some());
            window.press("x", cx);
        })
        .unwrap();
        wait_until(cx, "伺服器收到按鍵", |_| received.lock().unwrap().contains(&b'x'));

        cx.update_window(window, |_, window, cx| {
            window.click("session-action", cx);
            let right_after = content.read(cx).live.is_none();
            window.render_frame(cx);
            assert!(right_after, "點擊後 live 未清除");
            assert!(content.read(cx).live.is_none(), "重繪後 live 又出現");
        })
        .unwrap();
    }

    /// RDP 測試伺服器（tests/rdp-fixture）；未建置時回傳 None，測試略過。
    struct RdpFixture {
        child: std::process::Child,
        directory: tempfile::TempDir,
        port: u16,
        pin: String,
    }
    impl Drop for RdpFixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn rdp_fixture() -> Option<RdpFixture> {
        let binary = std::env::var_os("CRACAMOLE_RDP_FIXTURE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../tests/rdp-fixture/target/release/cracamole-rdp-fixture.exe")
            });
        if !binary.exists() {
            eprintln!("略過：找不到 RDP 測試伺服器 {}", binary.display());
            return None;
        }
        let directory = tempfile::tempdir().unwrap();
        let child = std::process::Command::new(binary)
            .arg(directory.path())
            .arg("frames")
            .spawn()
            .unwrap();
        let mut fixture = RdpFixture {
            child,
            directory,
            port: 0,
            pin: String::new(),
        };
        for _ in 0..400 {
            if let Ok(text) = std::fs::read_to_string(fixture.directory.path().join("ready")) {
                let mut lines = text.lines();
                if let (Some(Ok(port)), Some(pin)) =
                    (lines.next().map(str::parse::<u16>), lines.next())
                {
                    fixture.port = port;
                    fixture.pin = pin.to_owned();
                    return Some(fixture);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        panic!("RDP 測試伺服器啟動逾時");
    }

    #[gpui_kit::test]
    fn rdp_confirms_the_certificate_then_shows_frames_and_forwards_input(cx: &mut TestAppContext) {
        let Some(fixture) = rdp_fixture() else {
            return;
        };
        cx.executor().allow_parking();
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            content.update(cx, |library, _| {
                let mut connection = super::sample("r1", "lab-rdp", "rdp", "127.0.0.1", "fixture");
                connection.params.insert("port".into(), fixture.port.to_string());
                connection.params.insert("security".into(), "tls".into());
                connection.params.insert("password".into(), "fixture-secret".into());
                library.connections.push(connection);
                library.selected = Some("r1".into());
            });
            window.render_frame(cx);
            window.click("session-action", cx);
        })
        .unwrap();

        wait_until(cx, "出現憑證核對", |cx| {
            cx.read_entity(&content, |library, _| library.pending.is_some())
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let pending = content.read(cx).pending.as_ref().unwrap();
            assert!(pending.fingerprint.eq_ignore_ascii_case(&fixture.pin));
            assert!(pending.detail.as_deref().is_some_and(|d| !d.is_empty()));
            assert!(content.read(cx).live.is_none());
            window.click("trust", cx);
        })
        .unwrap();

        wait_until(cx, "收到 RDP 畫面", |cx| {
            cx.read_entity(&content, |library, _| {
                library.live.as_ref().is_some_and(|live| live.frame.is_some())
            })
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(content.read(cx).live.as_ref().unwrap().size, (320, 240));
            // 鍵盤：z → 伺服器記錄 scancode 0x2c。
            window.press("z", cx);
            // 滑鼠：移到遠端 (42, 31) 後按下並放開左鍵。
            let bounds = window.find("session").bounds();
            let (left, top, _, _) = super::remote::display_rect(
                (f32::from(bounds.size.width), f32::from(bounds.size.height)),
                (320, 240),
            );
            let offset = gpui_kit::point(px(left + 42.5), px(top + 31.5));
            window.click_at("session", offset, cx);
        })
        .unwrap();
        wait_until(cx, "伺服器收到按鍵與滑鼠", |_| {
            fixture.directory.path().join("key").exists()
                && fixture.directory.path().join("mouse").exists()
        });
    }

    /// 最小的程序內 RFB 伺服器：無驗證、64×48 的紅色畫面，記錄用戶端訊息。
    fn vnc_fixture() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};

        async fn message(s: &mut TcpStream) -> Option<Vec<u8>> {
            let kind = s.read_u8().await.ok()?;
            let mut out = vec![kind];
            let mut body = match kind {
                0 => vec![0u8; 19],
                3 => vec![0u8; 9],
                4 => vec![0u8; 7],
                5 => vec![0u8; 5],
                2 => {
                    let mut head = [0u8; 3];
                    s.read_exact(&mut head).await.ok()?;
                    out.extend_from_slice(&head);
                    vec![0u8; usize::from(u16::from_be_bytes([head[1], head[2]])) * 4]
                }
                6 => {
                    let mut head = [0u8; 7];
                    s.read_exact(&mut head).await.ok()?;
                    out.extend_from_slice(&head);
                    vec![0u8; u32::from_be_bytes([head[3], head[4], head[5], head[6]]) as usize]
                }
                _ => return None,
            };
            s.read_exact(&mut body).await.ok()?;
            out.extend(body);
            Some(out)
        }

        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = log.clone();
        let port = super::remote::block_on(async move {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let (w, h) = (64u16, 48u16);
                s.write_all(b"RFB 003.008\n").await.unwrap();
                let mut version = [0u8; 12];
                s.read_exact(&mut version).await.unwrap();
                s.write_all(&[1, 1]).await.unwrap(); // 安全類型：None
                s.read_u8().await.unwrap();
                s.write_all(&0u32.to_be_bytes()).await.unwrap();
                s.read_u8().await.unwrap(); // ClientInit
                let mut init = vec![];
                init.extend_from_slice(&w.to_be_bytes());
                init.extend_from_slice(&h.to_be_bytes());
                init.extend_from_slice(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
                init.extend_from_slice(&6u32.to_be_bytes());
                init.extend_from_slice(b"My Mac");
                s.write_all(&init).await.unwrap();
                for _ in 0..3 {
                    message(&mut s).await.unwrap(); // SetPixelFormat、SetEncodings、更新請求
                }
                // 一張 Raw 更新：整面紅色（記憶體順序 B、G、R、X）。
                let mut update = vec![0, 0, 0, 1];
                for value in [0u16, 0, w, h] {
                    update.extend_from_slice(&value.to_be_bytes());
                }
                update.extend_from_slice(&0i32.to_be_bytes());
                for _ in 0..(usize::from(w) * usize::from(h)) {
                    update.extend_from_slice(&[0, 0, 255, 0]);
                }
                s.write_all(&update).await.unwrap();
                while let Some(m) = message(&mut s).await {
                    sink.lock().unwrap().push(m);
                }
            });
            port
        });
        (port, log)
    }

    #[gpui_kit::test]
    fn vnc_session_paints_the_framebuffer_and_forwards_keys_and_pointer(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let (port, log) = vnc_fixture();
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            content.update(cx, |library, _| {
                let mut connection = super::sample("v1", "lab-vnc", "vnc", "127.0.0.1", "");
                connection.params.insert("port".into(), port.to_string());
                library.connections.push(connection);
                library.selected = Some("v1".into());
            });
            window.render_frame(cx);
            // VNC 沒有身分核對步驟，直接建立工作階段。
            window.click("session-action", cx);
            assert!(content.read(cx).pending.is_none());
        })
        .unwrap();

        wait_until(cx, "收到 VNC 畫面", |cx| {
            cx.read_entity(&content, |library, _| {
                library.live.as_ref().is_some_and(|live| live.frame.is_some())
            })
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let library = content.read(cx);
            let live = library.live.as_ref().unwrap();
            assert_eq!(live.size, (64, 48));
            let pixels = live.frame.as_ref().unwrap().as_bytes(0).unwrap();
            assert_eq!(&pixels[..3], &[0, 0, 255], "左上角應為紅色（BGRA）");
            window.press("a", cx);
            let bounds = window.find("session").bounds();
            let (left, top, _, _) = super::remote::display_rect(
                (f32::from(bounds.size.width), f32::from(bounds.size.height)),
                (64, 48),
            );
            window.click_at(
                "session",
                gpui_kit::point(px(left + 10.5), px(top + 5.5)),
                cx,
            );
        })
        .unwrap();
        wait_until(cx, "伺服器收到按鍵與滑鼠", |_| {
            let log = log.lock().unwrap();
            let key = log
                .iter()
                .any(|m| m[0] == 4 && m[1] == 1 && m[4..8] == 0x61u32.to_be_bytes());
            let pointer = log
                .iter()
                .any(|m| m[0] == 5 && m[1] == 1 && m[2..4] == 10u16.to_be_bytes() && m[4..6] == 5u16.to_be_bytes());
            key && pointer
        });
    }

    #[gpui_kit::test]
    fn login_gates_the_library_then_saves_and_pins_in_the_private_store(cx: &mut TestAppContext) {
        use gpui_kit::component::input::InputState;
        cx.executor().allow_parking();
        let credentials = super::remote::block_on(super::Credentials::new(
            "admin".into(),
            "correct-horse-battery".into(),
        ))
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path().join("desktop");
        let (window, content) = cx.update(|cx| {
            gpui_kit::init(cx);
            let dir = data_dir.clone();
            let (window, content) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1200.), px(760.)),
                    })),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    cx.new(|cx| {
                        Library::locked(window, cx, Ok(std::sync::Arc::new(credentials)), Some(dir))
                    })
                },
            )
            .expect("open test window");
            (gpui_kit::AnyWindowHandle::from(window), content)
        });

        let fill = |cx: &mut TestAppContext, user: &str, password: &str| {
            cx.update_window(window, |_, window, cx| {
                let (u, p) = {
                    let auth = content.read(cx).auth.as_ref().unwrap();
                    (auth.username.clone(), auth.password.clone())
                };
                u.update(cx, |s: &mut InputState, cx| s.set_value(user, window, cx));
                p.update(cx, |s: &mut InputState, cx| s.set_value(password, window, cx));
                window.render_frame(cx);
                window.click("login", cx);
            })
            .unwrap();
        };

        // 未登入：沒有任何連線資料，也看不到連線庫畫面。
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("login").is_some());
            assert!(window.try_find("add").is_none());
            assert!(content.read(cx).connections.is_empty());
            assert!(content.read(cx).store.is_none());
        })
        .unwrap();

        // 錯誤密碼：維持鎖定、顯示固定訊息，且不建立資料目錄。
        fill(cx, "admin", "wrong-password-123");
        wait_until(cx, "收到驗證結果", |cx| {
            cx.read_entity(&content, |library, _| {
                library.auth.as_ref().is_some_and(|auth| auth.error.is_some())
            })
        });
        cx.update_window(window, |_, _, cx| {
            let library = content.read(cx);
            assert_eq!(
                library.auth.as_ref().unwrap().error.as_deref(),
                Some("帳號或密碼不正確。")
            );
            assert!(library.store.is_none());
            // 送出後密碼欄位已清空。
            assert!(library.auth.as_ref().unwrap().password.read(cx).value().is_empty());
        })
        .unwrap();
        assert!(!data_dir.exists(), "驗證失敗不得建立資料目錄");

        // 正確密碼：開啟私有連線庫。
        fill(cx, "admin", "correct-horse-battery");
        wait_until(cx, "登入完成", |cx| {
            cx.read_entity(&content, |library, _| library.auth.is_none())
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(content.read(cx).store.is_some());
            assert!(content.read(cx).connections.is_empty());
            assert!(window.try_find("add").is_some());

            // 新增含密碼的連線 → 寫入私有連線庫。
            window.click("add", cx);
            window.render_frame(cx);
            let (name, host, secret) = {
                let library = content.read(cx);
                (library.name.clone(), library.host.clone(), library.secret.clone())
            };
            name.update(cx, |s: &mut InputState, cx| s.set_value("lab-persist", window, cx));
            host.update(cx, |s: &mut InputState, cx| s.set_value("10.1.2.3", window, cx));
            secret.update(cx, |s: &mut InputState, cx| s.set_value("s3cret-value", window, cx));
            window.click("save", cx);
            window.render_frame(cx);
            let library = content.read(cx);
            assert!(!library.editing && library.error.is_none());
            assert_eq!(library.connections.len(), 1);
            // 密碼不在公開欄位，只標記為已設定。
            let saved = &library.connections[0];
            assert!(saved.configured_secrets.iter().any(|s| s == "password"));
            assert!(!saved.params.contains_key("password"));
            // 儲存後編輯器欄位（含密碼）已清空。
            assert!(library.secret.read(cx).value().is_empty());
            let id = saved.id.clone();
            let store = library.store.clone().unwrap();
            let full = super::remote::block_on(store.get(&id)).unwrap().unwrap();
            assert_eq!(full.params.get("password").map(String::as_str), Some("s3cret-value"));

            // 釘選會持久化。
            window.click(format!("pin-{id}"), cx);
            window.render_frame(cx);
            assert!(store.pinned().unwrap().contains(&id));
        })
        .unwrap();
        let persisted = std::fs::read_to_string(data_dir.join("connections.json")).unwrap();
        assert!(persisted.contains("lab-persist"));

        // 編輯：密碼留空時保留原值；刪除：連線庫與檔案都移除。
        cx.update_window(window, |_, window, cx| {
            let id = content.read(cx).connections[0].id.clone();
            let store = content.read(cx).store.clone().unwrap();
            window.click(format!("row-{id}"), cx);
            window.render_frame(cx);
            window.click("edit", cx);
            window.render_frame(cx);
            let name = content.read(cx).name.clone();
            name.update(cx, |s: &mut InputState, cx| s.set_value("lab-renamed", window, cx));
            window.click("save", cx);
            window.render_frame(cx);
            let library = content.read(cx);
            assert!(library.error.is_none() && !library.editing);
            assert_eq!(library.connections.len(), 1);
            assert_eq!(library.connections[0].name, "lab-renamed");
            let full = super::remote::block_on(store.get(&id)).unwrap().unwrap();
            assert_eq!(full.params.get("password").map(String::as_str), Some("s3cret-value"));

            window.click("delete", cx);
            window.render_frame(cx);
            window.click("delete-confirm", cx);
            window.render_frame(cx);
            assert!(content.read(cx).connections.is_empty());
            assert!(super::remote::block_on(store.get(&id)).unwrap().is_none());
            assert!(!store.pinned().unwrap().contains(&id));
        })
        .unwrap();
        let persisted = std::fs::read_to_string(data_dir.join("connections.json")).unwrap();
        assert!(!persisted.contains("lab-renamed") && !persisted.contains("s3cret-value"));

        // 群組更名：寫入私有連線庫，保留成員的機密。
        cx.update_window(window, |_, window, cx| {
            let store = content.read(cx).store.clone().unwrap();
            for (name, secret) in [("ops/one", "p1"), ("ops/two", "p2")] {
                let mut params = std::collections::BTreeMap::new();
                params.insert("hostname".to_string(), "10.0.0.9".to_string());
                params.insert("port".to_string(), "23".to_string());
                params.insert("password".to_string(), secret.to_string());
                let draft = cracamole_store::Draft {
                    name: name.into(),
                    protocol: "telnet".into(),
                    params,
                };
                super::remote::block_on(store.save(None, draft)).unwrap();
            }
            content.update(cx, |library, _| {
                library.connections = super::remote::block_on(store.list()).unwrap_or_default();
            });
            window.render_frame(cx);
            window.click("rename-ops", cx);
            window.render_frame(cx);
            let input = content.read(cx).group_name.clone();
            input.update(cx, |s: &mut InputState, cx| s.set_value("infra", window, cx));
            window.click("rename-save", cx);
            window.render_frame(cx);
            let library = content.read(cx);
            assert!(library.error.is_none(), "{:?}", library.error);
            let mut names: Vec<_> = library.connections.iter().map(|c| c.name.clone()).collect();
            names.sort();
            assert_eq!(names, ["infra / one", "infra / two"]);
            let one = library.connections.iter().find(|c| c.name == "infra / one").unwrap();
            let full = super::remote::block_on(store.get(&one.id)).unwrap().unwrap();
            assert_eq!(full.params.get("password").map(String::as_str), Some("p1"));
        })
        .unwrap();
    }

    #[test]
    fn editing_keeps_other_settings_and_drops_trust_when_the_destination_changes() {
        use super::merged_params;
        let mut base = super::sample("e1", "x", "rdp", "10.0.0.5", "alice");
        base.params.insert("port".into(), "3389".into());
        base.params.insert("domain".into(), "EXAMPLE".into());
        base.params.insert("server-cert-sha256".into(), "ab".repeat(32));

        // 目的地不變：保留信任與其他設定；清空帳號會移除 username。
        let same = merged_params(Some(&base), "rdp", "10.0.0.5", "3389", "", "");
        assert_eq!(same.get("server-cert-sha256"), base.params.get("server-cert-sha256"));
        assert_eq!(same.get("domain").map(String::as_str), Some("EXAMPLE"));
        assert!(!same.contains_key("username"));
        assert!(!same.contains_key("password"));

        // 主機、連接埠或協定改變：移除舊的信任，其他設定保留。
        for (protocol, host, port) in [("rdp", "10.0.0.6", "3389"), ("rdp", "10.0.0.5", "3390"), ("vnc", "10.0.0.5", "3389")] {
            let moved = merged_params(Some(&base), protocol, host, port, "alice", "new-secret");
            assert!(!moved.contains_key("server-cert-sha256"), "{protocol} {host} {port}");
            assert_eq!(moved.get("domain").map(String::as_str), Some("EXAMPLE"));
            assert_eq!(moved.get("password").map(String::as_str), Some("new-secret"));
        }

        // 新增：只有表單欄位。
        let fresh = merged_params(None, "ssh", "h", "22", "u", "");
        assert_eq!(fresh.len(), 3);
    }

    #[gpui_kit::test]
    fn edit_prefills_the_form_and_delete_needs_confirmation(cx: &mut TestAppContext) {
        use gpui_kit::component::input::InputState;
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("edit").is_none(), "未選取時不顯示編輯與刪除");
            window.click("row-1", cx);
            window.render_frame(cx);

            window.click("edit", cx);
            window.render_frame(cx);
            let library = content.read(cx);
            assert!(library.editing && library.editing_id.as_deref() == Some("1"));
            assert_eq!(library.name.read(cx).value().as_ref(), "prod-web-01");
            assert_eq!(library.host.read(cx).value().as_ref(), "10.0.1.11");
            assert_eq!(library.user.read(cx).value().as_ref(), "deploy");

            let name = library.name.clone();
            name.update(cx, |s: &mut InputState, cx| s.set_value("prod-web-renamed", window, cx));
            window.click("save", cx);
            window.render_frame(cx);
            let library = content.read(cx);
            assert!(!library.editing && library.editing_id.is_none());
            assert_eq!(library.connections.len(), 5);
            assert_eq!(library.connections[0].name, "prod-web-renamed");

            // 刪除：先出現確認，取消不刪除，確認才刪除。
            window.click("delete", cx);
            window.render_frame(cx);
            assert!(window.try_find("delete-confirm").is_some());
            window.click("delete-cancel", cx);
            window.render_frame(cx);
            assert_eq!(content.read(cx).connections.len(), 5);
            window.click("delete", cx);
            window.render_frame(cx);
            window.click("delete-confirm", cx);
            window.render_frame(cx);
            let library = content.read(cx);
            assert_eq!(library.connections.len(), 4);
            assert!(library.selected.is_none());
            assert!(library.connections.iter().all(|c| c.id != "1"));
        })
        .unwrap();
    }

    /// 注意：TestAppContext 使用記憶體內的假剪貼簿，這些測試不會讀寫 Windows 系統剪貼簿。
    #[gpui_kit::test]
    fn clipboard_flows_both_ways_and_honours_policy(cx: &mut TestAppContext) {
        use gpui_kit::ClipboardItem;
        cx.executor().allow_parking();
        let (port, received) = telnet_fixture();
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("paste").is_none(), "未連線時沒有貼上按鈕");
            content.update(cx, |library, _| {
                let mut connection = super::sample("t1", "lab-telnet", "telnet", "127.0.0.1", "");
                connection.params.insert("port".into(), port.to_string());
                library.connections.push(connection);
                library.selected = Some("t1".into());
            });
            window.render_frame(cx);
            window.click("session-action", cx);
        })
        .unwrap();
        wait_until(cx, "收到第一張畫面", |cx| {
            cx.read_entity(&content, |library, _| {
                library.live.as_ref().is_some_and(|live| live.frame.is_some())
            })
        });

        // 本機 → 遠端：含控制字元的文字被過濾後送出。
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            cx.write_to_clipboard(ClipboardItem::new_string("echo \u{1b}[31m遠端🙂".into()));
            window.click("paste", cx);
        })
        .unwrap();
        wait_until(cx, "伺服器收到貼上文字", |_| {
            let bytes = received.lock().unwrap().clone();
            String::from_utf8_lossy(&bytes).contains("echo [31m遠端🙂")
        });
        assert!(!received.lock().unwrap().contains(&0x1b), "控制字元不得送到遠端");

        // 停用貼上：不送出任何資料。
        let before = received.lock().unwrap().len();
        cx.update_window(window, |_, window, cx| {
            content.update(cx, |library, _| library.live.as_mut().unwrap().allow_paste = false);
            cx.write_to_clipboard(ClipboardItem::new_string("blocked".into()));
            window.click("paste", cx);
            window.render_frame(cx);
            assert_eq!(
                content.read(cx).live.as_ref().unwrap().status,
                "此連線不允許貼上到遠端。"
            );
        })
        .unwrap();
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(received.lock().unwrap().len(), before);

        // 遠端 → 本機：寫入剪貼簿；停用複製時不寫入。
        cx.update_window(window, |_, _, cx| {
            content.update(cx, |library, cx| {
                library.on_event(super::Event::Clipboard("遠端複製的文字".into()), cx)
            });
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
                Some("遠端複製的文字")
            );
            content.update(cx, |library, cx| {
                library.live.as_mut().unwrap().allow_copy = false;
                library.on_event(super::Event::Clipboard("不應寫入".into()), cx)
            });
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
                Some("遠端複製的文字")
            );
        })
        .unwrap();
    }

    #[test]
    fn group_names_split_on_the_first_slash_only() {
        use super::{split_group, valid_group};
        assert_eq!(split_group("核心機房/db-01"), (Some("核心機房"), "db-01"));
        assert_eq!(split_group(" ops / web "), (Some("ops"), "web"));
        assert_eq!(split_group("plain"), (None, "plain"));
        assert_eq!(split_group("/oops"), (None, "/oops"));
        assert!(valid_group("") && valid_group("辦公網段"));
        assert!(!valid_group("a/b") && !valid_group("a\u{7}b"));
        assert!(!valid_group(&"x".repeat(117)));
    }

    #[gpui_kit::test]
    fn groups_collapse_rename_and_compose_names_in_the_editor(cx: &mut TestAppContext) {
        use gpui_kit::component::input::InputState;
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            content.update(cx, |library, _| {
                library.connections.clear();
                library.pinned.clear();
                for (id, name) in [("a", "核心/db"), ("b", "核心/web"), ("c", "辦公/mac"), ("d", "lonely")] {
                    library.connections.push(super::sample(id, name, "ssh", "10.0.0.1", "u"));
                }
            });
            window.render_frame(cx);
            // 有群組時，沒有群組的項目歸在「未分類」。
            for header in ["未分類", "核心", "辦公"] {
                assert!(window.try_find(format!("group-{header}")).is_some(), "{header}");
            }

            // 收合：成員被隱藏；再點一次展開。
            window.click("group-核心", cx);
            window.render_frame(cx);
            assert!(window.try_find("row-a").is_none() && window.try_find("row-c").is_some());
            window.click("group-核心", cx);
            window.render_frame(cx);
            assert!(window.try_find("row-a").is_some());

            // 更名整個群組。
            window.click("rename-核心", cx);
            window.render_frame(cx);
            assert_eq!(content.read(cx).group_name.read(cx).value().as_ref(), "核心");
            let input = content.read(cx).group_name.clone();
            input.update(cx, |s: &mut InputState, cx| s.set_value("a/b", window, cx));
            window.click("rename-save", cx);
            window.render_frame(cx);
            assert!(content.read(cx).error.is_some(), "含斜線的群組名稱要被拒絕");
            input.update(cx, |s: &mut InputState, cx| s.set_value("資料中心", window, cx));
            window.click("rename-save", cx);
            window.render_frame(cx);
            let names: Vec<_> = content.read(cx).connections.iter().map(|c| c.name.clone()).collect();
            assert_eq!(names, ["資料中心 / db", "資料中心 / web", "辦公/mac", "lonely"]);
            assert!(content.read(cx).renaming.is_none());

            // 留空：把該群組移至未分類。
            window.click("rename-辦公", cx);
            window.render_frame(cx);
            let input = content.read(cx).group_name.clone();
            input.update(cx, |s: &mut InputState, cx| s.set_value("", window, cx));
            window.click("rename-save", cx);
            window.render_frame(cx);
            assert_eq!(content.read(cx).connections[2].name, "mac");

            // 編輯器：群組與名稱合成 `群組/名稱`；編輯時再拆開。
            window.click("add", cx);
            window.render_frame(cx);
            let (group, name, host) = {
                let library = content.read(cx);
                (library.group.clone(), library.name.clone(), library.host.clone())
            };
            group.update(cx, |s: &mut InputState, cx| s.set_value("lab", window, cx));
            name.update(cx, |s: &mut InputState, cx| s.set_value("node-1", window, cx));
            host.update(cx, |s: &mut InputState, cx| s.set_value("10.9.0.1", window, cx));
            window.click("save", cx);
            window.render_frame(cx);
            let library = content.read(cx);
            assert!(library.error.is_none(), "{:?}", library.error);
            let saved = library.connections.last().unwrap();
            assert_eq!(saved.name, "lab / node-1");
            let id = saved.id.clone();
            window.click(format!("row-{id}"), cx);
            window.render_frame(cx);
            window.click("edit", cx);
            window.render_frame(cx);
            let library = content.read(cx);
            assert_eq!(library.group.read(cx).value().as_ref(), "lab");
            assert_eq!(library.name.read(cx).value().as_ref(), "node-1");
        })
        .unwrap();
    }
}
