#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod windows_clipboard;
#[cfg(windows)]
mod windows_editor;
#[cfg(windows)]
mod windows_paste;
#[cfg(windows)]
mod windows_private;
#[cfg(windows)]
mod windows_store;

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_ui::run() {
        native_windows_gui::simple_message("cracamole", &format!("無法啟動視窗：{error}"));
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("原生桌面介面目前僅支援 Windows。");
}

#[cfg(windows)]
#[allow(deprecated)] // NWG 的 Windows 訊息計時器不會啟動忙等執行緒。
mod windows_ui {
    use crate::windows_editor::Editor;
    use crate::windows_paste::{PasteJob, PasteReader};
    use crate::windows_store::{SaveError, WindowsStore};
    use cracamole_desktop::driver::{self, Frame, Input};
    use cracamole_desktop::geometry::centered_image_point;
    use cracamole_desktop::trust::{ProbeError, Probes};
    use cracamole_store::{credentials::Credentials, Draft, PublicConnection};
    use native_windows_gui as nwg;
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
        rc::Rc,
        sync::Arc,
        time::{Duration, Instant},
    };
    use tokio::sync::{mpsc, watch};
    use tokio_util::sync::CancellationToken;
    use zeroize::Zeroize;

    struct Active {
        cancel: CancellationToken,
        input: mpsc::Sender<Input>,
        frames: watch::Receiver<Option<Arc<Frame>>>,
        clipboard: watch::Receiver<Option<Arc<str>>>,
        input_policy: watch::Receiver<Option<(bool, bool)>>,
        shown_frame: bool,
        displayed_size: Option<(u32, u32)>,
        paste_limit: Arc<tokio::sync::Semaphore>,
        task: tokio::task::JoinHandle<Result<(), String>>,
    }

    enum Identity {
        Ssh(guacd::HostKeyInfo),
        Rdp(guacd::CertInfo),
    }

    struct IdentityProbe {
        cancel: CancellationToken,
        target: PublicConnection,
        task: tokio::task::JoinHandle<Result<Identity, ProbeError>>,
    }

    impl Drop for IdentityProbe {
        fn drop(&mut self) {
            self.cancel.cancel();
        }
    }

    impl Drop for Active {
        fn drop(&mut self) {
            self.cancel.cancel();
        }
    }

    fn special_key(key: u32) -> Option<u32> {
        match key {
            0x08 => Some(0xff08),
            0x09 => Some(0xff09),
            0x0d => Some(0xff0d),
            0x1b => Some(0xff1b),
            0x21 => Some(0xff55),
            0x22 => Some(0xff56),
            0x23 => Some(0xff57),
            0x24 => Some(0xff50),
            0x25 => Some(0xff51),
            0x26 => Some(0xff52),
            0x27 => Some(0xff53),
            0x28 => Some(0xff54),
            0x2d => Some(0xff63),
            0x2e => Some(0xffff),
            0x10 => Some(0xffe1),
            0x11 => Some(0xffe3),
            0x12 => Some(0xffe9),
            0x70..=0x7b => Some(0xffbe + key - 0x70),
            _ => None,
        }
    }

    fn control_key(key: u32, altgr: bool) -> Option<u32> {
        if altgr {
            return None;
        }
        match key {
            0x41..=0x5a => Some(key + 0x20),
            0x30..=0x39 => Some(key),
            // Windows resolves OEM keys using the input thread's keyboard layout.
            0xba..=0xe2 => {
                use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
                    MapVirtualKeyW, MAPVK_VK_TO_CHAR,
                };
                let value = unsafe { MapVirtualKeyW(key, MAPVK_VK_TO_CHAR) };
                (0x20..=0x7e).contains(&value).then_some(value)
            }
            _ => None,
        }
    }

    fn right_alt_is_down() -> bool {
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0xa5) < 0 }
    }

    // Windows represents AltGr as a synthetic Ctrl plus right Alt. Do not
    // expose those modifiers to the remote while WM_CHAR supplies the text.
    fn altgr_event(
        key: u32,
        pressed: bool,
        right_alt: bool,
        active: &Cell<bool>,
        modifiers: &Cell<u8>,
    ) -> Option<Vec<(u32, bool)>> {
        if key == 0x12 && pressed && right_alt && modifiers.get() & 2 != 0 {
            active.set(true);
            modifiers.set(modifiers.get() | 4);
            return Some(vec![(0xffe3, false)]);
        }
        if key == 0x12 && !pressed && active.replace(false) {
            modifiers.set(modifiers.get() & !4);
            return Some(if modifiers.get() & 2 != 0 {
                vec![(0xffe3, true)]
            } else {
                vec![]
            });
        }
        if key == 0x11 && active.get() {
            if pressed {
                modifiers.set(modifiers.get() | 2);
            } else {
                modifiers.set(modifiers.get() & !2);
            }
            return Some(vec![]);
        }
        None
    }

    fn send_key(active: &RefCell<Option<Active>>, key: u32, pressed: bool) {
        if let Some(session) = active.borrow().as_ref() {
            if session
                .input
                .try_send(Input::Key {
                    keysym: key,
                    pressed,
                })
                .is_err()
            {
                // A lost release could leave a remote modifier stuck; terminate instead.
                session.cancel.cancel();
            }
        }
    }

    fn release_keys(
        active: &RefCell<Option<Active>>,
        modifiers: &Cell<u8>,
        altgr: &Cell<bool>,
        control_keys: &RefCell<BTreeMap<u32, u32>>,
    ) -> bool {
        // Windows may deliver WM_KILLFOCUS synchronously while a timer handler
        // is updating the active session; let the next timer tick retry.
        if active.try_borrow().is_err() || control_keys.try_borrow_mut().is_err() {
            return false;
        }
        for (_, keysym) in std::mem::take(&mut *control_keys.borrow_mut()) {
            send_key(active, keysym, false);
        }
        let held = modifiers.replace(0);
        altgr.set(false);
        for (mask, key) in [(1, 0xffe1), (2, 0xffe3), (4, 0xffe9)] {
            if held & mask != 0 {
                send_key(active, key, false);
            }
        }
        true
    }

    fn finish_pending_key_release(
        pending: &Cell<bool>,
        active: &RefCell<Option<Active>>,
        modifiers: &Cell<u8>,
        altgr: &Cell<bool>,
        control_keys: &RefCell<BTreeMap<u32, u32>>,
    ) -> bool {
        if !pending.get() {
            return true;
        }
        if !release_keys(active, modifiers, altgr, control_keys) {
            return false;
        }
        pending.set(false);
        true
    }

    fn release_mouse(mask: &Cell<u32>) {
        if mask.replace(0) != 0 {
            nwg::GlobalCursor::release();
        }
    }

    fn lost_capture(mask: &Cell<u32>, last: &Cell<Option<(u32, u32)>>) -> Option<Input> {
        if mask.replace(0) == 0 {
            return None;
        }
        last.get().map(|(x, y)| Input::Mouse { x, y, mask: 0 })
    }

    fn layout_library(
        window: &nwg::Window,
        list: &nwg::ListBox<String>,
        buttons: [&nwg::Button; 8],
    ) {
        let (width, height) = window.size();
        let list_width = width.saturating_sub(48).max(96);
        let list_height = height.saturating_sub(360).max(120);
        list.set_size(list_width, list_height);
        let row = 128 + list_height as i32 + 16;
        buttons[0].set_position(24, row); // refresh
        buttons[1].set_position((width as i32 - 124).max(266), row); // logout
        buttons[2].set_position(145, row); // connect
        for (index, button) in buttons[3..].iter().enumerate() {
            button.set_position(24 + index as i32 * 121, row + 85);
        }
    }

    fn layout_session(
        window: &nwg::Window,
        image: &nwg::ImageFrame,
        input: &nwg::TextInput,
        logout: &nwg::Button,
        disconnect: &nwg::Button,
        paste: &nwg::Button,
    ) -> (u32, u32) {
        let (width, height) = window.size();
        let width = width.saturating_sub(48).clamp(96, 4096);
        let height = height.saturating_sub(193).clamp(64, 4096);
        image.set_size(width, height);
        let buttons_y = 128 + height as i32 + 8;
        disconnect.set_position(24, buttons_y);
        paste.set_position(145, buttons_y);
        logout.set_position((width as i32 - 76).max(266), buttons_y);
        // Keep the edit control focusable for keyboard and IME input without displaying typed text.
        input.set_position(24, buttons_y + 33);
        input.set_size(1, 1);
        (width, height)
    }

    fn frame_png(frame: &Frame) -> Result<Vec<u8>, String> {
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, frame.width as u32, frame.height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
            writer
                .write_image_data(&frame.rgba)
                .map_err(|e| e.to_string())?;
        }
        Ok(data)
    }

    fn connection_label(connection: &cracamole_store::PublicConnection) -> String {
        let registry = guacd::Registry::builtin();
        let info = registry
            .get(&connection.protocol)
            .map(|plugin| plugin.info());
        let label = info.map_or(connection.protocol.as_str(), |item| item.label);
        let host = connection.params.get("hostname").map_or("", String::as_str);
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        let user = connection.params.get("username").filter(|s| !s.is_empty());
        let port = connection
            .params
            .get("port")
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| {
                info.and_then(|item| {
                    (item.default_port != 0).then(|| item.default_port.to_string())
                })
            });
        let endpoint = format!(
            "{}{}{}",
            user.map_or(String::new(), |s| format!("{s}@")),
            host,
            port.map_or(String::new(), |s| format!(":{s}"))
        );
        let trusted = ["host-key", "server-cert-sha256"].iter().any(|key| {
            connection
                .params
                .get(*key)
                .is_some_and(|value| !value.is_empty())
        });
        let readonly = connection
            .params
            .get("read-only")
            .is_some_and(|value| value == "true");
        format!(
            "[{label}] {} — {endpoint}{}{}",
            connection.name,
            if trusted { " · 已信任" } else { "" },
            if readonly { " · 唯讀" } else { "" }
        )
    }

    fn refresh_connections(
        runtime: &tokio::runtime::Runtime,
        store: &WindowsStore,
        list: &nwg::ListBox<String>,
        status: &nwg::Label,
        ids: &RefCell<Vec<String>>,
    ) {
        match runtime.block_on(store.list()) {
            Ok(connections) => {
                let count = connections.len();
                *ids.borrow_mut() = connections.iter().map(|c| c.id.clone()).collect();
                list.set_collection(
                    connections
                        .into_iter()
                        .map(|c| connection_label(&c))
                        .collect(),
                );
                if count > 0 {
                    list.set_selection(Some(0));
                }
                status.set_text(&format!("已載入 {count} 筆桌面連線。"));
            }
            Err(error) => {
                ids.borrow_mut().clear();
                list.set_collection(Vec::new());
                let _ = error;
                status.set_text("無法讀取桌面連線設定，請檢查資料目錄權限。");
            }
        }
    }

    fn session_lifetime(value: Option<&str>) -> Result<Duration, &'static str> {
        let Some(value) = value else {
            return Ok(Duration::from_secs(8 * 60 * 60));
        };
        value
            .parse::<u64>()
            .ok()
            .filter(|seconds| (1..=8 * 60 * 60).contains(seconds))
            .map(Duration::from_secs)
            .ok_or("CRACAMOLE_DESKTOP_SESSION_MAX_SECS 必須是 1 至 28800 的整數")
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let password = std::env::var("CRACAMOLE_ADMIN_PASSWORD")
            .map_err(|_| "請設定 CRACAMOLE_ADMIN_PASSWORD（至少 12 個字元）")?;
        let username = std::env::var("CRACAMOLE_ADMIN_USERNAME").unwrap_or_else(|_| "admin".into());
        let lifetime_setting = match std::env::var("CRACAMOLE_DESKTOP_SESSION_MAX_SECS") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err("CRACAMOLE_DESKTOP_SESSION_MAX_SECS 必須是 1 至 28800 的整數".into());
            }
        };
        let session_lifetime = session_lifetime(lifetime_setting.as_deref())?;
        let runtime = Rc::new(tokio::runtime::Runtime::new()?);
        let credentials = Rc::new(runtime.block_on(Credentials::new(username.clone(), password))?);
        nwg::init()?;
        nwg::Font::set_global_family("Segoe UI")?;
        let mut window = nwg::Window::default();
        let mut heading = nwg::Label::default();
        let mut explanation = nwg::Label::default();
        let mut username_label = nwg::Label::default();
        let mut username_input = nwg::TextInput::default();
        let mut password_label = nwg::Label::default();
        let mut password_input = nwg::TextInput::default();
        let mut login = nwg::Button::default();
        let mut close = nwg::Button::default();
        nwg::Window::builder()
            .size((640, 550))
            .title("cracamole — Windows 原生版（開發中）")
            .build(&mut window)?;
        nwg::Label::builder()
            .text("cracamole Windows 原生介面")
            .position((24, 24))
            .size((560, 30))
            .parent(&window)
            .build(&mut heading)?;
        nwg::Label::builder()
            .text("請以管理員帳號登入。桌面介面仍在開發中。")
            .position((24, 70))
            .size((570, 45))
            .parent(&window)
            .build(&mut explanation)?;
        nwg::Label::builder()
            .text("帳號")
            .position((24, 132))
            .size((75, 28))
            .parent(&window)
            .build(&mut username_label)?;
        nwg::TextInput::builder()
            .text(&username)
            .position((110, 128))
            .size((300, 28))
            .parent(&window)
            .build(&mut username_input)?;
        nwg::Label::builder()
            .text("密碼")
            .position((24, 174))
            .size((75, 28))
            .parent(&window)
            .build(&mut password_label)?;
        nwg::TextInput::builder()
            .password(Some('●'))
            .position((110, 170))
            .size((300, 28))
            .parent(&window)
            .build(&mut password_input)?;
        nwg::Button::builder()
            .text("登入")
            .position((110, 230))
            .size((100, 32))
            .parent(&window)
            .build(&mut login)?;
        nwg::Button::builder()
            .text("關閉")
            .position((230, 230))
            .size((100, 32))
            .parent(&window)
            .build(&mut close)?;
        password_input.set_focus();
        let mut connections = nwg::ListBox::<String>::default();
        let mut refresh = nwg::Button::default();
        let mut logout = nwg::Button::default();
        let mut connect = nwg::Button::default();
        let mut disconnect = nwg::Button::default();
        let mut paste = nwg::Button::default();
        let mut add = nwg::Button::default();
        let mut edit = nwg::Button::default();
        let mut delete = nwg::Button::default();
        let mut verify = nwg::Button::default();
        let mut duplicate = nwg::Button::default();
        let mut image = nwg::ImageFrame::default();
        let mut remote_input = nwg::TextInput::default();
        nwg::ListBox::builder()
            .position((24, 128))
            .size((570, 190))
            .parent(&window)
            .build(&mut connections)?;
        connections.set_visible(false);
        nwg::Button::builder()
            .text("重新載入")
            .position((24, 335))
            .size((100, 32))
            .parent(&window)
            .build(&mut refresh)?;
        refresh.set_visible(false);
        nwg::Button::builder()
            .text("登出")
            .position((145, 335))
            .size((100, 32))
            .parent(&window)
            .build(&mut logout)?;
        logout.set_visible(false);
        nwg::Button::builder()
            .text("連線")
            .position((266, 335))
            .size((100, 32))
            .parent(&window)
            .build(&mut connect)?;
        connect.set_visible(false);
        nwg::Button::builder()
            .text("中斷連線")
            .position((387, 335))
            .size((100, 32))
            .parent(&window)
            .build(&mut disconnect)?;
        disconnect.set_visible(false);
        nwg::Button::builder()
            .text("貼上")
            .position((508, 335))
            .size((86, 32))
            .parent(&window)
            .build(&mut paste)?;
        paste.set_visible(false);
        nwg::ImageFrame::builder()
            .position((24, 128))
            .size((570, 190))
            .parent(&window)
            .build(&mut image)?;
        image.set_visible(false);
        nwg::TextInput::builder()
            .password(Some('●'))
            .position((24, 380))
            .size((570, 28))
            .parent(&window)
            .build(&mut remote_input)?;
        remote_input.set_visible(false);
        for (button, title, x) in [
            (&mut add, "新增", 24),
            (&mut edit, "編輯", 145),
            (&mut delete, "刪除", 266),
            (&mut verify, "核對身分", 387),
            (&mut duplicate, "複製", 508),
        ] {
            nwg::Button::builder()
                .text(title)
                .position((x, 420))
                .size((100, 32))
                .parent(&window)
                .build(button)?;
            button.set_visible(false);
        }
        let editor = Editor::new(&window)?;
        let editor_text_handles = editor.text_fields();
        let editor_cancel_handle = editor.cancel.handle;
        let mut timer = nwg::Timer::default();
        nwg::Timer::builder()
            .parent(&window)
            .interval(100)
            .stopped(false)
            .build(&mut timer)?;
        let window = Rc::new(window);
        let event_window = Rc::clone(&window);
        let attempts = RefCell::new(Vec::<Instant>::new());
        let expires = Cell::new(None::<Instant>);
        let ids = RefCell::new(Vec::<String>::new());
        let active = Rc::new(RefCell::new(None::<Active>));
        let paste_reader = PasteReader::new();
        let paste_job = RefCell::new(None::<PasteJob>);
        let paste_id = Cell::new(0u64);
        let raw_active = Rc::clone(&active);
        let capture_active = Rc::clone(&active);
        let input_handle = remote_input.handle;
        let image_handle = image.handle;
        let pending_resize = Cell::new(None::<(u32, u32, Instant)>);
        let probing = RefCell::new(None::<IdentityProbe>);
        let probes = Arc::new(Probes::new(CancellationToken::new()));
        let store = RefCell::new(None::<Arc<WindowsStore>>);
        let modifiers = Rc::new(Cell::new(0u8));
        let altgr = Rc::new(Cell::new(false));
        let control_keys = Rc::new(RefCell::new(BTreeMap::<u32, u32>::new()));
        let focus_modifiers = Rc::clone(&modifiers);
        let focus_altgr = Rc::clone(&altgr);
        let focus_control_keys = Rc::clone(&control_keys);
        let pending_key_release = Rc::new(Cell::new(false));
        let focus_pending_key_release = Rc::clone(&pending_key_release);
        let mouse_mask = Rc::new(Cell::new(0u32));
        let capture_mask = Rc::clone(&mouse_mask);
        let last_mouse = Rc::new(Cell::new(None::<(u32, u32)>));
        let capture_last_mouse = Rc::clone(&last_mouse);
        let handler = nwg::full_bind_event_handler(&window.handle, move |event, data, handle| {
            if (event == nwg::Event::OnWindowClose && handle == event_window.handle)
                || (event == nwg::Event::OnButtonClick && handle == close.handle)
            {
                active.borrow_mut().take();
                paste_job.borrow_mut().take();
                pending_resize.set(None);
                probing.borrow_mut().take();
                release_mouse(&mouse_mask);
                modifiers.set(0);
                altgr.set(false);
                control_keys.borrow_mut().clear();
                image.set_bitmap(None);
                nwg::stop_thread_dispatch();
            } else if (event == nwg::Event::OnButtonClick && handle == login.handle)
                || (event == nwg::Event::OnKeyPress
                    && (handle == password_input.handle || handle == username_input.handle)
                    && matches!(data, nwg::EventData::OnKey(0x0d)))
            {
                let now = Instant::now();
                let mut attempts = attempts.borrow_mut();
                attempts.retain(|at| now.duration_since(*at) < Duration::from_secs(60));
                if attempts.len() >= 10 {
                    password_input.set_text("");
                    explanation.set_text("登入次數過多，請稍後再試。");
                    return;
                }
                attempts.push(now);
                let password = password_input.text();
                password_input.set_text("");
                match runtime.block_on(credentials.verify(&username_input.text(), password)) {
                    Ok(true) => {
                        let directory = std::env::var_os("CRACAMOLE_DESKTOP_DATA_DIR")
                            .map(std::path::PathBuf::from)
                            .or_else(|| {
                                std::env::var_os("LOCALAPPDATA").map(|base| {
                                    std::path::PathBuf::from(base).join("cracamole-desktop")
                                })
                            });
                        let Some(directory) = directory else {
                            explanation.set_text(
                                "找不到桌面資料目錄。請設定 CRACAMOLE_DESKTOP_DATA_DIR。",
                            );
                            return;
                        };
                        let opened = match runtime.block_on(WindowsStore::open(directory)) {
                            Ok(opened) => Arc::new(opened),
                            Err(_) => {
                                explanation.set_text(
                                    "無法開啟私有桌面連線庫，請檢查資料目錄權限或其他桌面程式。",
                                );
                                return;
                            }
                        };
                        expires.set(Some(Instant::now() + session_lifetime));
                        username_label.set_visible(false);
                        username_input.set_visible(false);
                        password_label.set_visible(false);
                        password_input.set_visible(false);
                        login.set_visible(false);
                        close.set_visible(false);
                        connections.set_visible(true);
                        refresh.set_visible(true);
                        logout.set_visible(true);
                        connect.set_visible(true);
                        add.set_visible(true);
                        edit.set_visible(true);
                        delete.set_visible(true);
                        verify.set_visible(true);
                        duplicate.set_visible(true);
                        layout_library(
                            &event_window,
                            &connections,
                            [
                                &refresh, &logout, &connect, &add, &edit, &delete, &verify,
                                &duplicate,
                            ],
                        );
                        refresh_connections(&runtime, &opened, &connections, &explanation, &ids);
                        *store.borrow_mut() = Some(opened);
                        connections.set_focus();
                    }
                    Ok(false) => explanation.set_text("帳號或密碼不正確。"),
                    Err(_) => explanation.set_text("無法驗證帳密，請稍後再試。"),
                }
            } else if (event == nwg::Event::OnButtonClick
                && (handle == refresh.handle || handle == logout.handle))
                || (event == nwg::Event::OnTimerTick
                    && handle == timer.handle
                    && expires.get().is_some_and(|end| Instant::now() >= end))
            {
                if handle == refresh.handle && expires.get().is_some_and(|end| Instant::now() < end)
                {
                    if let Some(store) = store.borrow().as_ref() {
                        refresh_connections(&runtime, store, &connections, &explanation, &ids);
                    }
                } else {
                    expires.set(None);
                    active.borrow_mut().take();
                    paste_job.borrow_mut().take();
                    pending_resize.set(None);
                    logout.set_position(145, 335);
                    probing.borrow_mut().take();
                    store.borrow_mut().take();
                    editor.show(false);
                    add.set_visible(false);
                    edit.set_visible(false);
                    delete.set_visible(false);
                    verify.set_visible(false);
                    duplicate.set_visible(false);
                    release_mouse(&mouse_mask);
                    modifiers.set(0);
                    altgr.set(false);
                    control_keys.borrow_mut().clear();
                    image.set_bitmap(None);
                    image.set_visible(false);
                    remote_input.set_text("");
                    remote_input.set_visible(false);
                    disconnect.set_visible(false);
                    paste.set_visible(false);
                    connect.set_visible(false);
                    ids.borrow_mut().clear();
                    connections.set_collection(Vec::new());
                    connections.set_visible(false);
                    refresh.set_visible(false);
                    logout.set_visible(false);
                    username_label.set_visible(true);
                    username_input.set_visible(true);
                    password_label.set_visible(true);
                    password_input.set_visible(true);
                    login.set_visible(true);
                    close.set_visible(true);
                    explanation.set_text("已登出，請重新登入。");
                }
            } else if event == nwg::Event::OnButtonClick
                && handle == verify.handle
                && expires.get().is_some_and(|end| Instant::now() < end)
                && active.borrow().is_none()
            {
                if probing.borrow().is_some() {
                    explanation.set_text("主機身分探測進行中，請稍候。");
                    return;
                }
                let Some(id) = connections
                    .selection()
                    .and_then(|i| ids.borrow().get(i).cloned())
                else {
                    explanation.set_text("請先選擇一筆連線。");
                    return;
                };
                let Some(opened) = store.borrow().as_ref().cloned() else {
                    return;
                };
                let target = match runtime.block_on(opened.list()) {
                    Ok(entries) => entries.into_iter().find(|entry| entry.id == id),
                    Err(_) => None,
                };
                let Some(target) = target else {
                    explanation.set_text("無法讀取連線設定。");
                    return;
                };
                let default_port = match target.protocol.as_str() {
                    "ssh" => 22,
                    "rdp" => 3389,
                    _ => {
                        explanation.set_text("只有 SSH 與 RDP 需要核對主機身分。");
                        return;
                    }
                };
                let host = target.params.get("hostname").cloned().unwrap_or_default();
                let port = target
                    .params
                    .get("port")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(default_port);
                let configured = target
                    .params
                    .get("host-key")
                    .filter(|value| !value.is_empty())
                    .cloned();
                let protocol = target.protocol.clone();
                let cancel = CancellationToken::new();
                let task_cancel = cancel.clone();
                let probes = probes.clone();
                let task = runtime.spawn(async move {
                    if protocol == "ssh" {
                        probes
                            .ssh(&host, port, configured.as_deref(), &task_cancel)
                            .await
                            .map(Identity::Ssh)
                    } else {
                        probes
                            .rdp(&host, port, &task_cancel)
                            .await
                            .map(Identity::Rdp)
                    }
                });
                *probing.borrow_mut() = Some(IdentityProbe {
                    cancel,
                    target,
                    task,
                });
                explanation.set_text("正在取得主機身分；此探測不傳送帳密。");
            } else if ((event == nwg::Event::OnButtonClick && handle == connect.handle)
                || (event == nwg::Event::OnListBoxDoubleClick && handle == connections.handle)
                || (event == nwg::Event::OnKeyPress
                    && handle == connections.handle
                    && matches!(data, nwg::EventData::OnKey(0x0d))))
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                let Some(id) = connections
                    .selection()
                    .and_then(|index| ids.borrow().get(index).cloned())
                else {
                    explanation.set_text("請先選擇一筆連線。");
                    return;
                };
                let Some(store) = store.borrow().as_ref().cloned() else {
                    return;
                };
                probing.borrow_mut().take();
                let (width, height) = layout_session(
                    &event_window,
                    &image,
                    &remote_input,
                    &logout,
                    &disconnect,
                    &paste,
                );
                let (input, receiver) = mpsc::channel(64);
                let (frame_tx, frames) = watch::channel(None);
                let (clipboard_tx, clipboard) = watch::channel(None);
                let (input_policy_tx, input_policy) = watch::channel(None);
                let cancel = CancellationToken::new();
                let task_cancel = cancel.clone();
                let task = runtime.spawn(async move {
                    let connection = store
                        .get(&id)
                        .await
                        .map_err(|e| e.to_string())?
                        .ok_or("找不到連線")?;
                    let read_only = connection
                        .params
                        .get("read-only")
                        .is_some_and(|value| value == "true");
                    let paste_allowed = !read_only
                        && !connection
                            .params
                            .get("disable-paste")
                            .is_some_and(|value| value == "true");
                    input_policy_tx.send_replace(Some((read_only, paste_allowed)));
                    driver::run(
                        &connection,
                        width,
                        height,
                        receiver,
                        frame_tx,
                        clipboard_tx,
                        task_cancel,
                    )
                    .await
                });
                *active.borrow_mut() = Some(Active {
                    cancel,
                    input,
                    frames,
                    clipboard,
                    input_policy,
                    shown_frame: false,
                    displayed_size: None,
                    paste_limit: Arc::new(tokio::sync::Semaphore::new(1)),
                    task,
                });
                connections.set_visible(false);
                connect.set_visible(false);
                refresh.set_visible(false);
                add.set_visible(false);
                edit.set_visible(false);
                delete.set_visible(false);
                verify.set_visible(false);
                duplicate.set_visible(false);
                image.set_visible(true);
                remote_input.set_visible(true);
                remote_input.set_enabled(false);
                disconnect.set_visible(true);
                paste.set_visible(true);
                paste.set_enabled(false);
                explanation.set_text("正在連線；點選遠端畫面後可輸入。");
            } else if event == nwg::Event::OnButtonClick && handle == disconnect.handle {
                active.borrow_mut().take();
                paste_job.borrow_mut().take();
                pending_resize.set(None);
                logout.set_position(145, 335);
                release_mouse(&mouse_mask);
                modifiers.set(0);
                altgr.set(false);
                control_keys.borrow_mut().clear();
                image.set_bitmap(None);
                image.set_visible(false);
                remote_input.set_text("");
                remote_input.set_visible(false);
                disconnect.set_visible(false);
                paste.set_visible(false);
                connections.set_visible(true);
                connect.set_visible(true);
                refresh.set_visible(true);
                add.set_visible(true);
                edit.set_visible(true);
                delete.set_visible(true);
                verify.set_visible(true);
                duplicate.set_visible(true);
                layout_library(
                    &event_window,
                    &connections,
                    [
                        &refresh, &logout, &connect, &add, &edit, &delete, &verify, &duplicate,
                    ],
                );
                explanation.set_text("連線已結束。");
            } else if event == nwg::Event::OnButtonClick
                && (handle == add.handle || handle == edit.handle || handle == duplicate.handle)
                && expires.get().is_some_and(|end| Instant::now() < end)
                && active.borrow().is_none()
            {
                let Some(opened) = store.borrow().as_ref().cloned() else {
                    return;
                };
                let base = if handle == edit.handle || handle == duplicate.handle {
                    let Some(id) = connections
                        .selection()
                        .and_then(|index| ids.borrow().get(index).cloned())
                    else {
                        explanation.set_text("請先選擇一筆連線。");
                        return;
                    };
                    match runtime.block_on(opened.list()) {
                        Ok(entries) => entries.into_iter().find(|item| item.id == id),
                        Err(_) => {
                            explanation.set_text("無法讀取連線設定。");
                            return;
                        }
                    }
                } else {
                    None
                };
                if (handle == edit.handle || handle == duplicate.handle) && base.is_none() {
                    explanation.set_text("找不到連線。");
                    return;
                }
                probing.borrow_mut().take();
                connections.set_visible(false);
                refresh.set_visible(false);
                logout.set_visible(false);
                connect.set_visible(false);
                add.set_visible(false);
                edit.set_visible(false);
                delete.set_visible(false);
                verify.set_visible(false);
                duplicate.set_visible(false);
                if handle == duplicate.handle {
                    editor.begin_duplicate(base.expect("已檢查來源連線"));
                    explanation.set_text("複製公開設定；密碼與私鑰須重新輸入。");
                } else {
                    editor.begin(base);
                    explanation.set_text("留空機密會保留原值；SSH 私鑰請填入檔案路徑。");
                }
            } else if event == nwg::Event::OnButtonClick
                && handle == editor.browse_key.handle
                && editor.is_visible()
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                let mut dialog = nwg::FileDialog::default();
                if nwg::FileDialog::builder()
                    .title("選擇 SSH 私鑰檔案")
                    .action(nwg::FileDialogAction::Open)
                    .build(&mut dialog)
                    .is_err()
                {
                    explanation.set_text("無法開啟檔案選擇器，請手動輸入私鑰路徑。");
                    return;
                }
                if dialog.run(Some(&event_window.handle))
                    && editor.is_visible()
                    && expires.get().is_some_and(|end| Instant::now() < end)
                {
                    match dialog
                        .get_selected_item()
                        .ok()
                        .and_then(|path| path.into_string().ok())
                    {
                        Some(path) => editor.set_key_path(&path),
                        None => explanation.set_text("無法讀取所選路徑，請手動輸入私鑰路徑。"),
                    }
                }
            } else if event == nwg::Event::OnComboxBoxSelection {
                editor.protocol_changed(handle);
            } else if ((event == nwg::Event::OnButtonClick
                && (handle == editor.save.handle || handle == editor.cancel.handle))
                || (event == nwg::Event::OnKeyPress
                    && editor.is_text_field(handle)
                    && matches!(data, nwg::EventData::OnKey(0x0d | 0x1b))))
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                let Some(opened) = store.borrow().as_ref().cloned() else {
                    return;
                };
                if handle == editor.save.handle || matches!(data, nwg::EventData::OnKey(0x0d)) {
                    let (id, draft) = match editor.draft() {
                        Ok(draft) => draft,
                        Err(message) => {
                            explanation.set_text(message);
                            return;
                        }
                    };
                    match runtime.block_on(opened.save(id.as_deref(), draft)) {
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            explanation.set_text("找不到連線。");
                            return;
                        }
                        Err(SaveError::Invalid(error)) => {
                            explanation.set_text(error.message);
                            return;
                        }
                        Err(SaveError::Io(error)) => {
                            explanation.set_text(
                                if error.kind() == std::io::ErrorKind::PermissionDenied {
                                    "無法儲存，請檢查私有目錄權限。"
                                } else {
                                    "無法儲存連線設定。"
                                },
                            );
                            return;
                        }
                    }
                }
                editor.show(false);
                connections.set_visible(true);
                refresh.set_visible(true);
                logout.set_visible(true);
                connect.set_visible(true);
                add.set_visible(true);
                edit.set_visible(true);
                delete.set_visible(true);
                verify.set_visible(true);
                duplicate.set_visible(true);
                layout_library(
                    &event_window,
                    &connections,
                    [
                        &refresh, &logout, &connect, &add, &edit, &delete, &verify, &duplicate,
                    ],
                );
                refresh_connections(&runtime, &opened, &connections, &explanation, &ids);
            } else if event == nwg::Event::OnButtonClick
                && handle == delete.handle
                && expires.get().is_some_and(|end| Instant::now() < end)
                && active.borrow().is_none()
            {
                let Some(id) = connections
                    .selection()
                    .and_then(|index| ids.borrow().get(index).cloned())
                else {
                    explanation.set_text("請先選擇一筆連線。");
                    return;
                };
                let choice = nwg::modal_message(
                    &event_window.handle,
                    &nwg::MessageParams {
                        title: "刪除連線",
                        content: "確定刪除所選連線？",
                        buttons: nwg::MessageButtons::YesNo,
                        icons: nwg::MessageIcons::Question,
                    },
                );
                if choice != nwg::MessageChoice::Yes
                    || !expires.get().is_some_and(|end| Instant::now() < end)
                {
                    return;
                }
                if let Some(opened) = store.borrow().as_ref() {
                    match runtime.block_on(opened.delete(&id)) {
                        Ok(true) => {
                            refresh_connections(&runtime, opened, &connections, &explanation, &ids)
                        }
                        _ => explanation.set_text("無法刪除連線。"),
                    }
                }
            } else if event == nwg::Event::OnButtonClick
                && handle == paste.handle
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                if let Some(session) = active.borrow().as_ref() {
                    if !session
                        .input_policy
                        .borrow()
                        .is_some_and(|(_, allowed)| allowed)
                    {
                        explanation.set_text("此連線禁止貼上，或尚未準備完成。");
                        return;
                    }
                    let Ok(permit) = session.paste_limit.clone().try_acquire_owned() else {
                        explanation.set_text("上一次貼上尚未完成，請稍候。");
                        return;
                    };
                    let id = paste_id.get().wrapping_add(1);
                    paste_id.set(id);
                    match paste_reader.start(id, permit) {
                        Ok(job) => {
                            *paste_job.borrow_mut() = Some(job);
                            explanation.set_text("正在讀取剪貼簿。");
                        }
                        Err(error) => explanation.set_text(error),
                    }
                }
            } else if event == nwg::Event::OnResize
                && handle == event_window.handle
                && active.borrow().is_none()
                && store.borrow().is_some()
                && !editor.is_visible()
            {
                layout_library(
                    &event_window,
                    &connections,
                    [
                        &refresh, &logout, &connect, &add, &edit, &delete, &verify, &duplicate,
                    ],
                );
            } else if event == nwg::Event::OnResize
                && handle == event_window.handle
                && active.borrow().is_some()
            {
                let (width, height) = layout_session(
                    &event_window,
                    &image,
                    &remote_input,
                    &logout,
                    &disconnect,
                    &paste,
                );
                pending_resize.set(Some((width, height, Instant::now())));
            } else if event == nwg::Event::OnTimerTick && handle == timer.handle {
                if paste_job
                    .borrow()
                    .as_ref()
                    .is_some_and(|job| Instant::now() >= job.deadline)
                {
                    paste_job.borrow_mut().take();
                    explanation.set_text("讀取剪貼簿逾時，請稍後再試。");
                }
                while let Some(result) = paste_reader.try_recv() {
                    let matches = paste_job
                        .borrow()
                        .as_ref()
                        .is_some_and(|job| job.id == result.id);
                    if !matches {
                        continue;
                    }
                    paste_job.borrow_mut().take();
                    if let Some(session) = active.borrow().as_ref() {
                        match result.text {
                            Ok(text) => {
                                if session
                                    .input
                                    .try_send(Input::Paste {
                                        text,
                                        permit: result.permit,
                                    })
                                    .is_err()
                                {
                                    session.cancel.cancel();
                                    explanation.set_text("無法傳送貼上內容，連線已結束。");
                                } else {
                                    explanation.set_text("已送出貼上內容。");
                                    remote_input.set_focus();
                                }
                            }
                            Err(error) => explanation.set_text(error),
                        }
                    }
                }
                let finished = {
                    let mut probe = probing.borrow_mut();
                    if probe.as_ref().is_some_and(|job| job.task.is_finished()) {
                        probe.take()
                    } else {
                        None
                    }
                };
                if let Some(mut job) = finished {
                    let result = runtime.block_on(&mut job.task);
                    let proposal = match result {
                        Ok(Ok(Identity::Ssh(info))) if info.matches => {
                            explanation.set_text("SSH 主機金鑰與現有信任相符。");
                            None
                        }
                        Ok(Ok(Identity::Ssh(info)))
                            if info.trusted
                                || job
                                    .target
                                    .params
                                    .get("host-key")
                                    .is_some_and(|value| !value.is_empty()) =>
                        {
                            explanation.set_text("SSH 主機金鑰與現有信任不符，已拒絕取代。");
                            None
                        }
                        Ok(Ok(Identity::Ssh(info))) => Some((
                            "host-key",
                            info.public_key,
                            format!(
                                "請透過其他可信管道核對 SSH 指紋：\n\n{}\n\n確認後儲存此主機金鑰？",
                                info.fingerprint
                            ),
                        )),
                        Ok(Ok(Identity::Rdp(info))) => {
                            let existing = job
                                .target
                                .params
                                .get("server-cert-sha256")
                                .filter(|value| !value.is_empty());
                            if let Some(existing) = existing {
                                if guacd::normalize_fingerprint(existing).as_deref()
                                    == Some(info.sha256.as_str())
                                {
                                    explanation.set_text("RDP 憑證與現有指紋相符。");
                                } else {
                                    explanation.set_text("RDP 憑證與現有指紋不符，已拒絕取代。");
                                }
                                None
                            } else {
                                Some(("server-cert-sha256", info.sha256.clone(), format!("請透過其他可信管道核對 RDP 憑證指紋：\n\n{}\n\n主體：{}\n簽發者：{}\n\n確認後儲存此指紋？", info.sha256, info.subject, info.issuer)))
                            }
                        }
                        _ => {
                            explanation.set_text("無法取得主機身分，請確認服務與設定。");
                            None
                        }
                    };
                    if let Some((key, value, content)) = proposal {
                        let choice = nwg::modal_message(
                            &event_window.handle,
                            &nwg::MessageParams {
                                title: "核對主機身分",
                                content: &content,
                                buttons: nwg::MessageButtons::YesNo,
                                icons: nwg::MessageIcons::Question,
                            },
                        );
                        if choice == nwg::MessageChoice::Yes
                            && expires.get().is_some_and(|end| Instant::now() < end)
                        {
                            let opened = store.borrow().as_ref().cloned();
                            if let Some(opened) = opened {
                                let current =
                                    runtime.block_on(opened.list()).ok().and_then(|entries| {
                                        entries.into_iter().find(|entry| entry.id == job.target.id)
                                    });
                                if let Some(current) = current.filter(|entry| {
                                    entry.protocol == job.target.protocol
                                        && entry.params == job.target.params
                                        && entry.name == job.target.name
                                }) {
                                    let mut params = current.params;
                                    params.insert(key.into(), value);
                                    let draft = Draft {
                                        name: current.name,
                                        protocol: current.protocol,
                                        params,
                                    };
                                    if matches!(
                                        runtime.block_on(opened.save(Some(&current.id), draft)),
                                        Ok(Some(_))
                                    ) {
                                        explanation.set_text("已儲存主機身分。");
                                    } else {
                                        explanation.set_text("無法儲存主機身分。");
                                    }
                                } else {
                                    explanation.set_text("連線設定已變更，請重新核對。");
                                }
                            }
                        }
                    }
                }
                if !remote_input.focus() {
                    pending_key_release.set(true);
                }
                finish_pending_key_release(
                    &pending_key_release,
                    &active,
                    &modifiers,
                    &altgr,
                    &control_keys,
                );
                let mut state = active.borrow_mut();
                if let Some(session) = state.as_mut() {
                    let policy = *session.input_policy.borrow();
                    paste.set_enabled(policy.is_some_and(|(_, allowed)| allowed));
                    remote_input.set_enabled(policy.is_some_and(|(read_only, _)| !read_only));
                    if let Some((width, height, changed)) = pending_resize.get() {
                        if changed.elapsed() >= Duration::from_millis(200) {
                            match session.input.try_send(Input::Resize { width, height }) {
                                Ok(()) => pending_resize.set(None),
                                Err(mpsc::error::TrySendError::Full(_)) => {}
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    pending_resize.set(None);
                                    session.cancel.cancel();
                                }
                            }
                        }
                    }
                    if session.frames.has_changed().unwrap_or(false) {
                        if let Some(frame) = session.frames.borrow_and_update().as_ref() {
                            if let Ok(bitmap) = frame_png(frame).and_then(|bytes| {
                                nwg::Bitmap::from_bin(&bytes).map_err(|e| e.to_string())
                            }) {
                                image.set_bitmap(Some(&bitmap));
                                session.displayed_size =
                                    Some((frame.width as u32, frame.height as u32));
                                // STATIC may copy alpha bitmaps; only transfer ownership when it retained this handle.
                                let mut installed = None;
                                let mut icon = None;
                                image.image(&mut installed, &mut icon);
                                if installed
                                    .as_ref()
                                    .is_some_and(|value| value.handle == bitmap.handle)
                                {
                                    std::mem::forget(bitmap);
                                }
                                layout_session(
                                    &event_window,
                                    &image,
                                    &remote_input,
                                    &logout,
                                    &disconnect,
                                    &paste,
                                );
                                if !session.shown_frame {
                                    session.shown_frame = true;
                                    if policy.is_some_and(|(read_only, _)| read_only) {
                                        explanation.set_text("已連線（唯讀）；遠端輸入已停用。");
                                    } else {
                                        explanation.set_text("已連線；點選遠端畫面後可輸入。");
                                        remote_input.set_focus();
                                    }
                                }
                            }
                        }
                    }
                    if session.clipboard.has_changed().unwrap_or(false) {
                        if let Some(text) = session.clipboard.borrow_and_update().as_ref() {
                            let owner = event_window.handle.hwnd().unwrap().cast();
                            if let Err(error) = crate::windows_clipboard::write_text(owner, text) {
                                explanation.set_text(error);
                            } else {
                                explanation.set_text("已更新系統剪貼簿。");
                            }
                        }
                    }
                    if session.task.is_finished() {
                        state.take();
                        paste_job.borrow_mut().take();
                        pending_resize.set(None);
                        release_mouse(&mouse_mask);
                        modifiers.set(0);
                        altgr.set(false);
                        control_keys.borrow_mut().clear();
                        image.set_bitmap(None);
                        image.set_visible(false);
                        remote_input.set_text("");
                        remote_input.set_visible(false);
                        disconnect.set_visible(false);
                        paste.set_visible(false);
                        logout.set_position(145, 335);
                        connections.set_visible(true);
                        connect.set_visible(true);
                        refresh.set_visible(true);
                        add.set_visible(true);
                        edit.set_visible(true);
                        delete.set_visible(true);
                        verify.set_visible(true);
                        duplicate.set_visible(true);
                        layout_library(
                            &event_window,
                            &connections,
                            [
                                &refresh, &logout, &connect, &add, &edit, &delete, &verify,
                                &duplicate,
                            ],
                        );
                        explanation.set_text("連線已結束或失敗。");
                    }
                }
            } else if event == nwg::Event::OnMouseWheel
                && (handle == image.handle || handle == remote_input.handle)
            {
                if let (Some(session), nwg::EventData::OnMouseWheel(delta)) =
                    (active.borrow().as_ref(), &data)
                {
                    if session
                        .input_policy
                        .borrow()
                        .is_none_or(|(read_only, _)| read_only)
                    {
                        return;
                    }
                    let local = nwg::GlobalCursor::local_position(&image.handle, None);
                    let point = session
                        .displayed_size
                        .and_then(|frame| centered_image_point(image.size(), frame, local, false));
                    if let Some((x, y)) = point.filter(|_| *delta != 0) {
                        let mask = mouse_mask.get();
                        let wheel = if *delta > 0 { 8 } else { 16 };
                        let pressed = Input::Mouse {
                            x,
                            y,
                            mask: mask | wheel,
                        };
                        let released = Input::Mouse { x, y, mask };
                        if session.input.try_send(pressed).is_err()
                            || session.input.try_send(released).is_err()
                        {
                            session.cancel.cancel();
                        }
                    }
                }
            } else if handle == image.handle
                && (event == nwg::Event::OnMouseMove
                    || matches!(event, nwg::Event::OnMousePress(_)))
            {
                if let Some(session) = active.borrow().as_ref() {
                    if session
                        .input_policy
                        .borrow()
                        .is_none_or(|(read_only, _)| read_only)
                    {
                        return;
                    }
                    let local = nwg::GlobalCursor::local_position(&image.handle, None);
                    let Some(frame) = session.displayed_size else {
                        return;
                    };
                    let Some((x, y)) =
                        centered_image_point(image.size(), frame, local, mouse_mask.get() != 0)
                    else {
                        return;
                    };
                    last_mouse.set(Some((x, y)));
                    if let nwg::Event::OnMousePress(press) = event {
                        use nwg::MousePressEvent::*;
                        let (bit, down) = match press {
                            MousePressLeftDown => (1, true),
                            MousePressLeftUp => (1, false),
                            MousePressRightDown => (4, true),
                            MousePressRightUp => (4, false),
                        };
                        if !down && mouse_mask.get() & bit == 0 {
                            return;
                        }
                        if down {
                            if mouse_mask.get() == 0 {
                                nwg::GlobalCursor::set_capture(&image.handle);
                            }
                            mouse_mask.set(mouse_mask.get() | bit);
                            remote_input.set_focus();
                        } else {
                            mouse_mask.set(mouse_mask.get() & !bit);
                            if mouse_mask.get() == 0 {
                                nwg::GlobalCursor::release();
                            }
                        }
                    }
                    let is_button = matches!(event, nwg::Event::OnMousePress(_));
                    if session
                        .input
                        .try_send(Input::Mouse {
                            x,
                            y,
                            mask: mouse_mask.get(),
                        })
                        .is_err()
                        && is_button
                    {
                        session.cancel.cancel();
                    }
                }
            } else if handle == remote_input.handle
                && matches!(
                    event,
                    nwg::Event::OnKeyPress
                        | nwg::Event::OnKeyRelease
                        | nwg::Event::OnSysKeyPress
                        | nwg::Event::OnSysKeyRelease
                )
            {
                if !finish_pending_key_release(
                    &pending_key_release,
                    &active,
                    &modifiers,
                    &altgr,
                    &control_keys,
                ) {
                    return;
                }
                if let nwg::EventData::OnKey(key) = data {
                    let pressed =
                        matches!(event, nwg::Event::OnKeyPress | nwg::Event::OnSysKeyPress);
                    let right_alt = key == 0x12 && pressed && right_alt_is_down();
                    if let Some(events) = altgr_event(key, pressed, right_alt, &altgr, &modifiers) {
                        for (keysym, pressed) in events {
                            send_key(&active, keysym, pressed);
                        }
                        return;
                    }
                    if let Some(keysym) = special_key(key) {
                        let mask = match keysym {
                            0xffe1 => 1,
                            0xffe3 => 2,
                            0xffe9 => 4,
                            _ => 0,
                        };
                        if mask == 0 {
                            if pressed {
                                control_keys.borrow_mut().insert(key, keysym);
                            } else if control_keys.borrow_mut().remove(&key).is_none() {
                                return;
                            }
                        } else if pressed {
                            modifiers.set(modifiers.get() | mask);
                        } else {
                            modifiers.set(modifiers.get() & !mask);
                        }
                        send_key(&active, keysym, pressed);
                    } else if pressed && modifiers.get() & 2 != 0 {
                        if let Some(keysym) = control_key(key, altgr.get()) {
                            control_keys.borrow_mut().insert(key, keysym);
                            send_key(&active, keysym, true);
                        }
                    } else if !pressed {
                        if let Some(keysym) = control_keys.borrow_mut().remove(&key) {
                            send_key(&active, keysym, false);
                        }
                    }
                }
            } else if handle == remote_input.handle && event == nwg::Event::OnTextInput {
                let mut text = remote_input.text();
                if !text.is_empty() {
                    remote_input.set_text("");
                }
                text.zeroize();
            }
        });
        // NWG converts each WM_CHAR to a Rust char separately, replacing UTF-16
        // surrogate halves with '?'. Read the raw units before that conversion.
        let surrogate = Cell::new(None::<u16>);
        let raw_handler =
            nwg::bind_raw_event_handler(&input_handle, 0x10000, move |_, message, value, _| {
                if message == 0x0087 {
                    // WM_GETDLGCODE: let the remote input receive Tab, arrows,
                    // Enter and Escape instead of dialog keyboard navigation.
                    return Some(0x0001 | 0x0002 | 0x0004 | 0x0080);
                }
                if message == 0x0008 {
                    // WM_KILLFOCUS
                    surrogate.set(None);
                    focus_pending_key_release.set(true);
                    finish_pending_key_release(
                        &focus_pending_key_release,
                        &raw_active,
                        &focus_modifiers,
                        &focus_altgr,
                        &focus_control_keys,
                    );
                    return None;
                }
                if message != 0x0102 {
                    // WM_CHAR
                    return None;
                }
                if !finish_pending_key_release(
                    &focus_pending_key_release,
                    &raw_active,
                    &focus_modifiers,
                    &focus_altgr,
                    &focus_control_keys,
                ) {
                    return Some(0);
                }
                let unit = value as u16;
                let character = match unit {
                    0xd800..=0xdbff => {
                        surrogate.set(Some(unit));
                        None
                    }
                    0xdc00..=0xdfff => surrogate.take().and_then(|high| {
                        char::from_u32(
                            0x10000 + (((high as u32 - 0xd800) << 10) | (unit as u32 - 0xdc00)),
                        )
                    }),
                    _ => {
                        surrogate.set(None);
                        char::from_u32(unit as u32)
                    }
                };
                if let Some(character) = character {
                    let value = character as u32;
                    let keysym = match value {
                        0x20..=0xff => Some(value),
                        0x100..=0x10ffff => Some(0x0100_0000 | value),
                        _ => None,
                    };
                    if let Some(keysym) = keysym {
                        send_key(&raw_active, keysym, true);
                        send_key(&raw_active, keysym, false);
                    }
                }
                Some(0)
            })?;
        // NWG consumes Escape in text controls before emitting OnKeyPress.
        // Post the existing cancel action so it runs after the current window message.
        let mut editor_key_handlers = Vec::new();
        for (index, handle) in editor_text_handles.iter().enumerate() {
            let cancel = editor_cancel_handle.hwnd().ok_or("找不到取消按鈕")? as isize;
            let handler =
                nwg::bind_raw_event_handler(handle, 0x11000 + index, move |_, message, key, _| {
                    if message == 0x0100 && key == 0x1b {
                        unsafe {
                            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                                cancel as _,
                                0x00f5,
                                0,
                                0,
                            );
                        }
                        return Some(0);
                    }
                    None
                })?;
            editor_key_handlers.push(handler);
        }
        let capture_handler =
            nwg::bind_raw_event_handler(&image_handle, 0x10001, move |_, message, _, _| {
                if message == 0x0215 {
                    // WM_CAPTURECHANGED
                    if let Some(release) = lost_capture(&capture_mask, &capture_last_mouse) {
                        if let Some(session) = capture_active.borrow().as_ref() {
                            if session.input.try_send(release).is_err() {
                                session.cancel.cancel();
                            }
                        }
                    }
                }
                None
            })?;
        nwg::dispatch_thread_events();
        for handler in editor_key_handlers {
            nwg::unbind_raw_event_handler(&handler)?;
        }
        nwg::unbind_raw_event_handler(&capture_handler)?;
        nwg::unbind_raw_event_handler(&raw_handler)?;
        nwg::unbind_event_handler(&handler);
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn connection_label_shows_endpoint_trust_and_read_only() {
            let connection = cracamole_store::PublicConnection {
                id: "id".into(),
                name: "機房 / 伺服器".into(),
                protocol: "ssh".into(),
                params: std::collections::BTreeMap::from([
                    ("hostname".into(), "2001:db8::1".into()),
                    ("username".into(), "alice".into()),
                    ("host-key".into(), "key".into()),
                    ("read-only".into(), "true".into()),
                ]),
                configured_secrets: vec!["password".into()],
            };
            assert_eq!(
                connection_label(&connection),
                "[SSH] 機房 / 伺服器 — alice@[2001:db8::1]:22 · 已信任 · 唯讀"
            );
        }

        #[test]
        fn session_lifetime_rejects_invalid_explicit_configuration() {
            assert_eq!(session_lifetime(None).unwrap(), Duration::from_secs(28800));
            assert_eq!(session_lifetime(Some("1")).unwrap(), Duration::from_secs(1));
            assert_eq!(
                session_lifetime(Some("28800")).unwrap(),
                Duration::from_secs(28800)
            );
            for invalid in ["", "0", "28801", "-1", "1s", " 10", "999999999999999999999"] {
                assert!(
                    session_lifetime(Some(invalid)).is_err(),
                    "accepted {invalid:?}"
                );
            }
        }

        #[tokio::test]
        async fn focus_loss_releases_control_keys_before_modifiers() {
            let (input, mut received) = mpsc::channel(8);
            let (_, frames) = watch::channel(None);
            let (_, clipboard) = watch::channel(None);
            let (_, input_policy) = watch::channel(Some((false, true)));
            let active = RefCell::new(Some(Active {
                cancel: CancellationToken::new(),
                input,
                frames,
                clipboard,
                input_policy,
                shown_frame: false,
                displayed_size: None,
                paste_limit: Arc::new(tokio::sync::Semaphore::new(1)),
                task: tokio::spawn(async { Ok(()) }),
            }));
            let modifiers = Cell::new(2);
            let altgr = Cell::new(false);
            let keys = RefCell::new(BTreeMap::from([(0x41, u32::from(b'a'))]));
            let pending = Cell::new(true);
            {
                let _session_update = active.borrow_mut();
                assert!(!finish_pending_key_release(
                    &pending, &active, &modifiers, &altgr, &keys
                ));
                assert!(pending.get());
                assert_eq!(modifiers.get(), 2);
                assert!(received.try_recv().is_err());
            }
            assert!(finish_pending_key_release(
                &pending, &active, &modifiers, &altgr, &keys
            ));
            assert!(!pending.get());
            assert!(matches!(
                received.recv().await,
                Some(Input::Key {
                    keysym: 0x61,
                    pressed: false
                })
            ));
            assert!(matches!(
                received.recv().await,
                Some(Input::Key {
                    keysym: 0xffe3,
                    pressed: false
                })
            ));
            assert_eq!(modifiers.get(), 0);
            assert!(keys.borrow().is_empty());
            assert!(finish_pending_key_release(
                &pending, &active, &modifiers, &altgr, &keys
            ));
            assert!(received.try_recv().is_err());
        }

        #[test]
        fn right_alt_detection_uses_windows_thread_keyboard_state() {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
                GetKeyboardState, SetKeyboardState,
            };
            let mut original = [0u8; 256];
            unsafe {
                assert_ne!(GetKeyboardState(original.as_mut_ptr()), 0);
                let mut simulated = original;
                simulated[0xa5] = 0;
                assert_ne!(SetKeyboardState(simulated.as_ptr()), 0);
                let released = right_alt_is_down();
                simulated[0xa5] = 0x80;
                assert_ne!(SetKeyboardState(simulated.as_ptr()), 0);
                let pressed = right_alt_is_down();
                assert_ne!(SetKeyboardState(original.as_ptr()), 0);
                assert!(!released && pressed);
            }
        }

        #[test]
        fn altgr_temporarily_releases_remote_control_without_sending_a_shortcut() {
            let active = Cell::new(false);
            let modifiers = Cell::new(2);
            assert_eq!(
                altgr_event(0x12, true, true, &active, &modifiers),
                Some(vec![(0xffe3, false)])
            );
            assert!(active.get());
            assert_eq!(control_key(u32::from(b'A'), active.get()), None);
            assert_eq!(control_key(u32::from(b'2'), active.get()), None);
            assert_eq!(
                altgr_event(0x12, false, false, &active, &modifiers),
                Some(vec![(0xffe3, true)])
            );
            assert!(!active.get());
            assert_eq!(modifiers.get(), 2);
            assert_eq!(control_key(u32::from(b'A'), false), Some(u32::from(b'a')));
            assert_eq!(altgr_event(0x12, true, false, &active, &modifiers), None);

            assert!(altgr_event(0x12, true, true, &active, &modifiers).is_some());
            assert_eq!(
                altgr_event(0x11, false, false, &active, &modifiers),
                Some(vec![])
            );
            assert_eq!(
                altgr_event(0x12, false, false, &active, &modifiers),
                Some(vec![])
            );
            assert_eq!(modifiers.get(), 0);
        }

        #[test]
        fn altgr_text_reaches_shared_terminal_without_control_bytes() {
            let active = Cell::new(false);
            let modifiers = Cell::new(2);
            let mut terminal = guac_terminal::keyboard::Keyboard::default();
            let modes = guac_terminal::keyboard::KeyModes::default();
            terminal.process(0xffe3, true, &modes);
            for (key, pressed) in altgr_event(0x12, true, true, &active, &modifiers).unwrap() {
                terminal.process(key, pressed, &modes);
            }
            assert_eq!(control_key(u32::from(b'A'), active.get()), None);
            assert_eq!(
                terminal.process(0x0100_0105, true, &modes).send,
                "ą".as_bytes()
            );
            for (key, pressed) in altgr_event(0x12, false, false, &active, &modifiers).unwrap() {
                terminal.process(key, pressed, &modes);
            }
            assert_eq!(terminal.process(u32::from(b'a'), true, &modes).send, [1]);
        }

        #[test]
        fn frame_png_loads_into_windows_bitmap_with_expected_pixel() {
            use windows_sys::Win32::Graphics::Gdi::{
                CreateCompatibleDC, DeleteDC, GetPixel, SelectObject,
            };

            nwg::init().unwrap();
            let frame = Frame {
                width: 1,
                height: 1,
                rgba: Arc::from([0xd3, 0x71, 0x19, 0xff]),
            };
            let bitmap = nwg::Bitmap::from_bin(&frame_png(&frame).unwrap()).unwrap();
            unsafe {
                let dc = CreateCompatibleDC(std::ptr::null_mut());
                assert!(!dc.is_null());
                let previous = SelectObject(dc, bitmap.handle.cast());
                assert!(!previous.is_null());
                let pixel = GetPixel(dc, 0, 0);
                SelectObject(dc, previous);
                DeleteDC(dc);
                assert_eq!(pixel, 0x1971d3);
            }
        }

        #[test]
        fn capture_loss_releases_buttons_once_at_last_position() {
            let mask = Cell::new(5);
            let last = Cell::new(Some((23, 41)));
            assert!(matches!(
                lost_capture(&mask, &last),
                Some(Input::Mouse {
                    x: 23,
                    y: 41,
                    mask: 0
                })
            ));
            assert_eq!(mask.get(), 0);
            assert!(lost_capture(&mask, &last).is_none());
        }
    }
}
