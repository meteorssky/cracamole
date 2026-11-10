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
    use cracamole_desktop::driver::{self, DriverError, Frame, Input};
    use cracamole_desktop::geometry::centered_image_point;
    use cracamole_desktop::trust::{ProbeError, Probes};
    use cracamole_store::{credentials::Credentials, Draft, PublicConnection};
    use native_windows_gui as nwg;
    use std::{
        cell::{Cell, RefCell},
        collections::{BTreeMap, BTreeSet},
        rc::Rc,
        sync::Arc,
        time::{Duration, Instant},
    };
    use tokio::sync::{mpsc, watch};
    use tokio_util::sync::CancellationToken;
    use zeroize::Zeroize;

    #[derive(Clone, Copy)]
    struct WindowRestore {
        style: isize,
        placement: windows_sys::Win32::UI::WindowsAndMessaging::WINDOWPLACEMENT,
        visible: bool,
    }

    fn enter_fullscreen(window: &nwg::Window) -> Option<WindowRestore> {
        use windows_sys::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = window.handle.hwnd()?.cast();
        unsafe {
            let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
            placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
            let mut monitor: MONITORINFO = std::mem::zeroed();
            monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetWindowPlacement(hwnd, &mut placement) == 0
                || GetMonitorInfoW(
                    MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                    &mut monitor,
                ) == 0
            {
                return None;
            }
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_OVERLAPPEDWINDOW as isize));
            let rect = monitor.rcMonitor;
            if SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            ) == 0
            {
                SetWindowLongPtrW(hwnd, GWL_STYLE, style);
                return None;
            }
            Some(WindowRestore {
                style,
                placement,
                visible: IsWindowVisible(hwnd) != 0,
            })
        }
    }

    fn leave_fullscreen(window: &nwg::Window, state: &Cell<Option<WindowRestore>>) {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        if let (Some(mut saved), Some(hwnd)) = (state.take(), window.handle.hwnd()) {
            unsafe {
                let hwnd = hwnd.cast();
                SetWindowLongPtrW(hwnd, GWL_STYLE, saved.style);
                if !saved.visible {
                    saved.placement.showCmd = SW_HIDE as u32;
                }
                SetWindowPlacement(hwnd, &saved.placement);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
        }
    }

    struct Active {
        connection_id: String,
        cancel: CancellationToken,
        input: mpsc::Sender<Input>,
        frames: watch::Receiver<Option<Arc<Frame>>>,
        clipboard: watch::Receiver<Option<Arc<str>>>,
        input_policy: watch::Receiver<Option<(bool, bool, bool)>>,
        metadata: watch::Receiver<Option<(String, String, Option<String>)>>,
        shown_frame: bool,
        transient: bool,
        displayed_size: Option<(u32, u32)>,
        paste_limit: Arc<tokio::sync::Semaphore>,
        task: tokio::task::JoinHandle<Result<(), DriverError>>,
    }

    struct TransientConnection(cracamole_store::Connection);

    impl Drop for TransientConnection {
        fn drop(&mut self) {
            for key in ["password", "private-key", "passphrase"] {
                if let Some(secret) = self.0.params.get_mut(key) {
                    secret.zeroize();
                }
            }
        }
    }

    fn clear_draft_secrets(draft: &mut Draft) {
        for key in ["password", "private-key", "passphrase"] {
            if let Some(secret) = draft.params.get_mut(key) {
                secret.zeroize();
            }
        }
    }

    fn saved_ssh_identity(
        draft: &Draft,
        entries: &[PublicConnection],
        independent: Option<&str>,
    ) -> Result<Option<String>, &'static str> {
        fn port(params: &BTreeMap<String, String>) -> Option<u16> {
            match params.get("port").map(String::as_str) {
                None | Some("") => Some(22),
                Some(value) => value.parse::<u16>().ok().filter(|value| *value != 0),
            }
        }
        if draft.protocol != "ssh" {
            return Ok(None);
        }
        let host = draft
            .params
            .get("hostname")
            .map_or("", |value| value.trim());
        let Some(target_port) = port(&draft.params) else {
            return Ok(None);
        };
        let mut trusted = independent;
        for entry in entries {
            if entry.protocol != "ssh"
                || !entry
                    .params
                    .get("hostname")
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case(host))
                || port(&entry.params) != Some(target_port)
            {
                continue;
            }
            let Some(key) = entry
                .params
                .get("host-key")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if trusted.is_some_and(|existing| existing != key) {
                return Err("同一 SSH 目的地有互相衝突的已保存主機金鑰，請先核對設定。");
            }
            trusted = Some(key);
        }
        Ok(trusted.map(str::to_string))
    }

    enum Identity {
        Ssh(guacd::HostKeyInfo),
        Rdp(guacd::CertInfo),
    }

    struct IdentityProbe {
        cancel: CancellationToken,
        target: PublicConnection,
        task: tokio::task::JoinHandle<Result<Identity, ProbeError>>,
        temporary: bool,
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

    fn control_text_for_endpoint(handle: nwg::ControlHandle) -> Option<String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};
        let hwnd = handle.hwnd()?.cast();
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if !(0..=4096).contains(&length) {
            return None;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16(&buffer[..copied as usize]).ok()
    }

    fn control_has_visible_style(handle: nwg::ControlHandle) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, GWL_STYLE, WS_VISIBLE,
        };
        handle.hwnd().is_some_and(|hwnd| unsafe {
            GetWindowLongPtrW(hwnd.cast(), GWL_STYLE) & WS_VISIBLE as isize != 0
        })
    }

    fn layout_library(
        window: &nwg::Window,
        list: &nwg::ListBox<String>,
        (search, group_picker, group_buttons): (
            &nwg::TextInput,
            &nwg::ComboBox<String>,
            [&nwg::Button; 3],
        ),
        buttons: [&nwg::Button; 8],
    ) {
        let (width, height) = window.size();
        let list_width = width.saturating_sub(48).max(96);
        let columns = ((list_width + 8) / 108).clamp(1, 5);
        let top_rows = 3u32.div_ceil(columns);
        let action_rows = 5u32.div_ceil(columns);
        let reserved = 272 + 16 + top_rows * 40 + 16 + action_rows * 40 + 24;
        let list_height = height.saturating_sub(reserved).max(64);
        search.set_position(24, 128);
        search.set_size(list_width, 28);
        group_picker.set_position(24, 164);
        group_picker.set_size(list_width.min(300), 150);
        group_buttons[0].set_position(24, 200);
        group_buttons[0].set_size(100, 28);
        group_buttons[1].set_position(132, 200);
        group_buttons[1].set_size(100, 28);
        group_buttons[2].set_position(24, 236);
        group_buttons[2].set_size(120, 28);
        list.set_position(24, 272);
        list.set_size(list_width, list_height);
        let row = 272 + list_height as i32 + 16;
        for (index, button) in [buttons[0], buttons[2], buttons[1]].iter().enumerate() {
            button.set_position(
                24 + (index as u32 % columns * 108) as i32,
                row + (index as u32 / columns * 40) as i32,
            );
        }
        let action_y = row + (top_rows * 40) as i32 + 16;
        for (index, button) in buttons[3..].iter().enumerate() {
            button.set_position(
                24 + (index as u32 % columns * 108) as i32,
                action_y + (index as u32 / columns * 40) as i32,
            );
        }
    }

    fn layout_session(
        window: &nwg::Window,
        image: &nwg::ImageFrame,
        (input, footer, status): (&nwg::TextInput, &nwg::Label, &nwg::Label),
        list: &nwg::ListBox<String>,
        (search, group_picker, group_buttons): (
            &nwg::TextInput,
            &nwg::ComboBox<String>,
            [&nwg::Button; 3],
        ),
        sidebar_visible: bool,
        [logout, disconnect, paste, secure_attention, fullscreen, sidebar_toggle, recent_button, recent_menu]: [&nwg::Button;
            8],
    ) -> (u32, u32) {
        let (window_width, window_height) = window.size();
        let sidebar_width = if sidebar_visible {
            window_width.saturating_sub(168).min(200)
        } else {
            0
        };
        let left = if sidebar_visible {
            45 + sidebar_width
        } else {
            24
        };
        let width = window_width.saturating_sub(left + 24).clamp(96, 4096);
        let available = window_width.saturating_sub(48).max(125);
        let buttons = [
            sidebar_toggle,
            recent_button,
            recent_menu,
            paste,
            secure_attention,
            fullscreen,
            disconnect,
            logout,
        ];
        let widths = [110u32, 100, 60, 86, 125, 92, 100, 100];
        let mut positions = [(0u32, 0u32); 8];
        let (mut x, mut row) = (0u32, 0u32);
        for (index, button_width) in widths.iter().enumerate() {
            if x > 0 && x + button_width > available {
                x = 0;
                row += 1;
            }
            positions[index] = (x, row);
            x += button_width + 8;
        }
        let logout_right = available.saturating_sub(widths[7]);
        if positions[7].1 != positions[6].1 || logout_right >= positions[6].0 + widths[6] + 8 {
            positions[7].0 = logout_right;
        }
        let toolbar_height = (row + 1) * 32 + row * 8;
        let stage_y = 44 + toolbar_height + 8;
        let height = window_height.saturating_sub(stage_y + 32).clamp(64, 4096);
        for (button, (x, row)) in buttons.into_iter().zip(positions) {
            button.set_position(24 + x as i32, 44 + (row * 40) as i32);
        }
        status.set_position(24, 8);
        status.set_size(window_width.saturating_sub(48).max(96), 28);
        search.set_position(24, stage_y as i32);
        search.set_size(sidebar_width, 28);
        let group_width = sidebar_width.saturating_sub(8) / 2;
        group_picker.set_position(24, (stage_y + 36) as i32);
        group_picker.set_size(sidebar_width, 150);
        group_buttons[0].set_position(24, (stage_y + 72) as i32);
        group_buttons[0].set_size(group_width, 28);
        group_buttons[1].set_position((32 + group_width) as i32, (stage_y + 72) as i32);
        group_buttons[1].set_size(group_width, 28);
        group_buttons[2].set_position(24, (stage_y + 108) as i32);
        group_buttons[2].set_size(sidebar_width.min(120), 28);
        list.set_position(24, (stage_y + 144) as i32);
        list.set_size(sidebar_width, height.saturating_sub(144));
        image.set_position(left as i32, stage_y as i32);
        image.set_size(width, height);
        let footer_y = (stage_y + height + 4) as i32;
        footer.set_position(24, footer_y);
        footer.set_size(window_width.saturating_sub(48).max(96), 20);
        // Keep the edit control focusable for keyboard and IME input without displaying typed text.
        input.set_position(24, footer_y + 1);
        input.set_size(1, 1);
        (width, height)
    }

    fn restore_library_header(heading: &nwg::Label, status: &nwg::Label) {
        heading.set_visible(true);
        status.set_position(24, 70);
        status.set_size(570, 45);
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

    fn connection_group(name: &str) -> Option<(&str, &str)> {
        let (group, member) = name.split_once('/')?;
        let group = group.trim();
        if group.is_empty() {
            None
        } else {
            Some((group, member.trim()))
        }
    }

    fn connection_group_key(name: &str) -> Option<String> {
        connection_group(name).map(|(group, _)| group.to_string())
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
        let (group, name) = connection_group(&connection.name).map_or(
            (String::new(), connection.name.as_str()),
            |(group, name)| (format!("[{group}] "), name),
        );
        format!(
            "[{label}] {group}{name} — {endpoint}{}{}",
            if trusted { " · 已信任" } else { "" },
            if readonly { " · 唯讀" } else { "" }
        )
    }

    struct LibraryGroup {
        key: Option<String>,
        visible: Vec<PublicConnection>,
    }

    fn library_groups(
        connections: Vec<PublicConnection>,
        query: &str,
        collapsed: &BTreeSet<Option<String>>,
    ) -> Vec<LibraryGroup> {
        let query = query.trim().to_lowercase();
        let mut grouped = BTreeMap::<Option<String>, Vec<PublicConnection>>::new();
        for connection in connections {
            let key = connection_group_key(&connection.name);
            let matches = if query.is_empty() {
                !collapsed.contains(&key)
            } else {
                connection.name.to_lowercase().contains(&query)
                    || key
                        .as_deref()
                        .unwrap_or("未分類")
                        .to_lowercase()
                        .contains(&query)
                    || connection_label(&connection)
                        .to_lowercase()
                        .contains(&query)
            };
            let members = grouped.entry(key).or_default();
            if matches {
                members.push(connection);
            }
        }
        let mut groups: Vec<_> = grouped
            .into_iter()
            .map(|(key, mut visible)| {
                visible.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.id.clone()));
                LibraryGroup { key, visible }
            })
            .collect();
        groups.sort_by_cached_key(|group| {
            (
                group.key.is_none(),
                group.key.as_deref().unwrap_or("").to_lowercase(),
                group.key.clone(),
            )
        });
        groups
    }

    fn fill_group_tree(
        tree: &nwg::TreeView,
        groups: Vec<LibraryGroup>,
        collapsed: &BTreeSet<Option<String>>,
        searching: bool,
        statuses: &BTreeMap<String, ConnectionStatus>,
        selected: Option<&str>,
    ) -> (Vec<Option<String>>, Vec<String>) {
        let mut keys = Vec::new();
        let mut ids = Vec::new();
        tree.clear();
        for group in groups {
            if searching && group.visible.is_empty() {
                continue;
            }
            let title = group.key.as_ref().map_or_else(
                || "未分類（無群組）".to_string(),
                |key| format!("群組：{key}"),
            );
            keys.push(group.key.clone());
            let root = tree.insert_item_with_param(
                &title,
                None,
                nwg::TreeInsert::Last,
                -(keys.len() as isize),
            );
            for entry in group.visible {
                ids.push(entry.id.clone());
                let label = format_connection_status(
                    &connection_label(&entry),
                    statuses.get(&entry.id).copied(),
                );
                let child = tree.insert_item_with_param(
                    &label,
                    Some(&root),
                    nwg::TreeInsert::Last,
                    ids.len() as isize,
                );
                if selected == Some(entry.id.as_str()) {
                    if let Some(hwnd) = tree.handle.hwnd() {
                        unsafe {
                            use windows_sys::Win32::UI::Controls::{TVGN_CARET, TVM_SELECTITEM};
                            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                                hwnd.cast(),
                                TVM_SELECTITEM,
                                TVGN_CARET as usize,
                                child.handle as isize,
                            );
                        }
                    }
                }
            }
            if searching || !collapsed.contains(&group.key) {
                tree.set_expand_state(&root, nwg::ExpandState::Expand);
            }
        }
        (keys, ids)
    }

    fn tree_selected_item(tree: &nwg::TreeView) -> Option<nwg::TreeItem> {
        let hwnd = tree.handle.hwnd()?;
        let item = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(hwnd.cast(), 0x110a, 9, 0)
        };
        (item != 0).then_some(nwg::TreeItem { handle: item as _ })
    }

    fn refresh_connections(
        runtime: &tokio::runtime::Runtime,
        store: &WindowsStore,
        list: &nwg::ListBox<String>,
        (status, update_status): (&nwg::Label, bool),
        (ids, statuses): (
            &RefCell<Vec<String>>,
            &RefCell<BTreeMap<String, ConnectionStatus>>,
        ),
        (query, collapsed): (&str, &BTreeSet<Option<String>>),
        (group_picker, group_ids): (&nwg::ComboBox<String>, &RefCell<Vec<Option<String>>>),
    ) {
        let selected = list
            .selection()
            .and_then(|index| ids.borrow().get(index).cloned());
        match runtime.block_on(store.list()) {
            Ok(connections) => {
                let previous_group = group_picker
                    .selection()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| group_ids.borrow().get(index).cloned());
                let model = library_groups(connections, query, collapsed);
                let groups: Vec<_> = model.iter().map(|group| group.key.clone()).collect();
                let mut labels = vec!["所有群組".to_string()];
                labels.extend(groups.iter().map(|group| {
                    group
                        .as_ref()
                        .map_or_else(|| "未分類（無群組）".into(), |name| format!("群組：{name}"))
                }));
                group_picker.set_collection(labels);
                let selected_group = previous_group
                    .and_then(|previous| groups.iter().position(|group| *group == previous))
                    .map_or(0, |index| index + 1);
                *group_ids.borrow_mut() = groups;
                group_picker.set_selection(Some(selected_group));
                let connections: Vec<_> =
                    model.into_iter().flat_map(|group| group.visible).collect();
                let count = connections.len();
                *ids.borrow_mut() = connections.iter().map(|c| c.id.clone()).collect();
                list.set_collection(
                    connections
                        .into_iter()
                        .map(|c| {
                            format_connection_status(
                                &connection_label(&c),
                                statuses.borrow().get(&c.id).copied(),
                            )
                        })
                        .collect(),
                );
                if count > 0 {
                    let selected_index = selected
                        .as_ref()
                        .and_then(|id| ids.borrow().iter().position(|item| item == id))
                        .unwrap_or(0);
                    list.set_selection(Some(selected_index));
                }
                if update_status {
                    status.set_text(&format!("已載入 {count} 筆桌面連線。"));
                }
            }
            Err(error) => {
                ids.borrow_mut().clear();
                list.set_collection(Vec::new());
                group_ids.borrow_mut().clear();
                group_picker.set_collection(vec!["所有群組".into()]);
                group_picker.set_selection(Some(0));
                let _ = error;
                status.set_text("無法讀取桌面連線設定，請檢查資料目錄權限。");
            }
        }
    }

    fn update_library_actions(
        list: &nwg::ListBox<String>,
        ids: &RefCell<Vec<String>>,
        buttons: [&nwg::Button; 5],
    ) {
        let selected = list
            .selection()
            .is_some_and(|index| index < ids.borrow().len());
        for button in buttons {
            button.set_enabled(selected);
        }
    }

    fn list_item_under_cursor(list: &nwg::ListBox<String>) -> Option<usize> {
        let (x, y) = nwg::GlobalCursor::local_position(list.handle, None);
        list_item_at(list, x, y)
    }

    fn tree_item_at(tree: &nwg::TreeView, x: i32, y: i32) -> Option<nwg::TreeItem> {
        use windows_sys::Win32::UI::Controls::{TVHITTESTINFO, TVHT_ONITEM, TVM_HITTEST};
        use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;
        let (width, height) = tree.size();
        if x < 0 || y < 0 || x as u32 >= width || y as u32 >= height {
            return None;
        }
        let mut hit: TVHITTESTINFO = unsafe { std::mem::zeroed() };
        hit.pt.x = x;
        hit.pt.y = y;
        let hwnd = tree.handle.hwnd()?;
        let item =
            unsafe { SendMessageW(hwnd.cast(), TVM_HITTEST, 0, (&mut hit as *mut _) as isize) };
        (item != 0 && hit.flags & TVHT_ONITEM != 0).then_some(nwg::TreeItem { handle: item as _ })
    }

    fn list_item_at(list: &nwg::ListBox<String>, x: i32, y: i32) -> Option<usize> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageW, LB_GETITEMRECT, LB_ITEMFROMPOINT,
        };
        let (width, height) = list.size();
        if x < 0
            || y < 0
            || x as u32 >= width
            || y as u32 >= height
            || x > u16::MAX as i32
            || y > u16::MAX as i32
        {
            return None;
        }
        let point = ((y as u32) << 16) | x as u32;
        let hwnd = list.handle.hwnd()?.cast();
        let result = unsafe { SendMessageW(hwnd, LB_ITEMFROMPOINT, 0, point as isize) } as u32;
        if result >> 16 != 0 {
            return None;
        }
        let index = (result & 0xffff) as usize;
        let mut rect: windows_sys::Win32::Foundation::RECT = unsafe { std::mem::zeroed() };
        let status =
            unsafe { SendMessageW(hwnd, LB_GETITEMRECT, index, (&mut rect as *mut _) as isize) };
        (status != -1 && x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
            .then_some(index)
    }

    fn list_context_anchor(list: &nwg::ListBox<String>, index: usize) -> Option<(i32, i32)> {
        use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
        use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, LB_GETITEMRECT};
        let hwnd = list.handle.hwnd()?.cast();
        let mut rect: windows_sys::Win32::Foundation::RECT = unsafe { std::mem::zeroed() };
        if unsafe { SendMessageW(hwnd, LB_GETITEMRECT, index, (&mut rect as *mut _) as isize) }
            == -1
        {
            return None;
        }
        let mut point = windows_sys::Win32::Foundation::POINT {
            x: rect.left,
            y: rect.bottom,
        };
        if unsafe { ClientToScreen(hwnd, &mut point) } == 0 {
            return None;
        }
        Some((point.x, point.y))
    }

    fn build_connection_context_menu() -> Option<windows_sys::Win32::UI::WindowsAndMessaging::HMENU>
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return None;
            }
            for (index, title) in ["連線", "編輯", "複製", "核對身分", "刪除"]
                .into_iter()
                .enumerate()
            {
                let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
                if AppendMenuW(menu, MF_STRING, index + 1, title.as_ptr()) == 0 {
                    DestroyMenu(menu);
                    return None;
                }
            }
            Some(menu)
        }
    }

    fn build_recent_menu(
        labels: &[String],
    ) -> Option<windows_sys::Win32::UI::WindowsAndMessaging::HMENU> {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        if labels.is_empty() || labels.len() > 20 {
            return None;
        }
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return None;
            }
            for (index, label) in labels.iter().enumerate() {
                let clean: String = label
                    .chars()
                    .take(160)
                    .map(|ch| if ch.is_control() { ' ' } else { ch })
                    .collect();
                let clean = clean.replace('&', "&&");
                let title: Vec<u16> = clean.encode_utf16().chain(Some(0)).collect();
                if AppendMenuW(menu, MF_STRING, index + 1, title.as_ptr()) == 0 {
                    DestroyMenu(menu);
                    return None;
                }
            }
            Some(menu)
        }
    }

    fn show_recent_menu(
        window: &nwg::Window,
        button: &nwg::Button,
        labels: &[String],
    ) -> Option<usize> {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = window.handle.hwnd()?.cast();
        let button_hwnd = button.handle.hwnd()?.cast();
        let menu = build_recent_menu(labels)?;
        unsafe {
            let mut rect: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
            let selected = if GetWindowRect(button_hwnd, &mut rect) != 0 {
                TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                    rect.left,
                    rect.bottom,
                    0,
                    hwnd,
                    std::ptr::null(),
                ) as usize
            } else {
                0
            };
            DestroyMenu(menu);
            (1..=labels.len()).contains(&selected).then(|| selected - 1)
        }
    }

    fn connection_context_menu(
        window: &nwg::Window,
        position: Option<(i32, i32)>,
    ) -> Option<usize> {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = window.handle.hwnd()?.cast();
        unsafe {
            let menu = build_connection_context_menu()?;
            let mut point: windows_sys::Win32::Foundation::POINT = std::mem::zeroed();
            let has_position = if let Some((x, y)) = position {
                point.x = x;
                point.y = y;
                true
            } else {
                GetCursorPos(&mut point) != 0
            };
            let selected = if has_position {
                TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    0,
                    hwnd,
                    std::ptr::null(),
                ) as usize
            } else {
                0
            };
            DestroyMenu(menu);
            (1..=5).contains(&selected).then(|| selected - 1)
        }
    }

    #[derive(Clone, Copy)]
    enum ConnectionStatus {
        Connecting,
        Connected,
        Ended,
        Failed,
        AuthenticationFailed,
        Timeout,
        HostNotFound,
        Disconnected,
    }

    struct QuickRequest {
        address: String,
        selected: Option<String>,
        source: Option<String>,
    }

    fn format_connection_status(label: &str, status: Option<ConnectionStatus>) -> String {
        let label = [
            "○ 未連線 · ",
            "◌ 目前工作階段（正在連線） · ",
            "● 目前工作階段（已連線） · ",
            "○ 連線已結束 · ",
            "● 連線失敗或異常中斷 · ",
            "● 驗證失敗 · ",
            "● 連線逾時 · ",
            "● 找不到主機 · ",
            "○ 已中斷 · ",
        ]
        .iter()
        .find_map(|prefix| label.strip_prefix(prefix))
        .unwrap_or(label);
        let prefix = match status {
            None => "○ 未連線 · ",
            Some(ConnectionStatus::Connecting) => "◌ 目前工作階段（正在連線） · ",
            Some(ConnectionStatus::Connected) => "● 目前工作階段（已連線） · ",
            Some(ConnectionStatus::Ended) => "○ 連線已結束 · ",
            Some(ConnectionStatus::Failed) => "● 連線失敗或異常中斷 · ",
            Some(ConnectionStatus::AuthenticationFailed) => "● 驗證失敗 · ",
            Some(ConnectionStatus::Timeout) => "● 連線逾時 · ",
            Some(ConnectionStatus::HostNotFound) => "● 找不到主機 · ",
            Some(ConnectionStatus::Disconnected) => "○ 已中斷 · ",
        };
        format!("{prefix}{label}")
    }

    fn update_connection_statuses(
        list: &nwg::ListBox<String>,
        ids: &RefCell<Vec<String>>,
        statuses: &RefCell<BTreeMap<String, ConnectionStatus>>,
    ) {
        let selected = list.selection();
        let current = list.collection().clone();
        let ids = ids.borrow();
        let statuses = statuses.borrow();
        let updated: Vec<String> = current
            .iter()
            .enumerate()
            .map(|(index, text)| {
                format_connection_status(
                    text,
                    ids.get(index).and_then(|id| statuses.get(id)).copied(),
                )
            })
            .collect();
        if current != updated {
            list.set_collection(updated);
            list.set_selection(selected);
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
        let mut secure_attention = nwg::Button::default();
        let mut fullscreen = nwg::Button::default();
        let mut sidebar_toggle = nwg::Button::default();
        let mut recent_button = nwg::Button::default();
        let mut recent_menu = nwg::Button::default();
        let mut session_footer = nwg::Label::default();
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
        let mut group_tree = nwg::TreeView::default();
        nwg::TreeView::builder()
            .position((24, 272))
            .size((570, 190))
            .parent(&window)
            .build(&mut group_tree)?;
        if let Some(hwnd) = group_tree.handle.hwnd() {
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    GetWindowLongPtrW, SetWindowLongPtrW, GWL_STYLE,
                };
                let style = GetWindowLongPtrW(hwnd.cast(), GWL_STYLE);
                SetWindowLongPtrW(hwnd.cast(), GWL_STYLE, style & !0x0008);
                windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                    hwnd.cast(),
                    0x2005,
                    1,
                    0,
                );
            }
        }
        group_tree.set_visible(false);
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
        nwg::Button::builder()
            .text("Ctrl+Alt+Del")
            .size((125, 32))
            .parent(&window)
            .build(&mut secure_attention)?;
        secure_attention.set_visible(false);
        nwg::Button::builder()
            .text("全螢幕")
            .size((92, 32))
            .parent(&window)
            .build(&mut fullscreen)?;
        fullscreen.set_visible(false);
        nwg::Button::builder()
            .text("收折側欄")
            .position((480, 88))
            .size((110, 32))
            .parent(&window)
            .build(&mut sidebar_toggle)?;
        sidebar_toggle.set_visible(false);
        nwg::Button::builder()
            .text("上一個連線")
            .size((100, 32))
            .parent(&window)
            .build(&mut recent_button)?;
        recent_button.set_visible(false);
        nwg::Button::builder()
            .text("最近")
            .size((60, 32))
            .parent(&window)
            .build(&mut recent_menu)?;
        recent_menu.set_visible(false);
        nwg::Label::builder()
            .text("等待連線")
            .parent(&window)
            .build(&mut session_footer)?;
        session_footer.set_visible(false);
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
        let mut search = nwg::TextInput::default();
        nwg::TextInput::builder()
            .position((24, 128))
            .size((200, 28))
            .placeholder_text(Some("搜尋連線或主機"))
            .parent(&window)
            .build(&mut search)?;
        search.set_visible(false);
        let mut collapse_group = nwg::Button::default();
        let mut expand_groups = nwg::Button::default();
        let mut tree_toggle = nwg::Button::default();
        for (button, label) in [
            (&mut collapse_group, "收折群組"),
            (&mut expand_groups, "展開群組"),
            (&mut tree_toggle, "樹狀檢視"),
        ] {
            nwg::Button::builder()
                .text(label)
                .size((100, 28))
                .parent(&window)
                .build(button)?;
            button.set_visible(false);
        }
        let mut group_picker = nwg::ComboBox::<String>::default();
        nwg::ComboBox::builder()
            .size((200, 150))
            .parent(&window)
            .collection(vec!["所有群組".into()])
            .build(&mut group_picker)?;
        group_picker.set_selection(Some(0));
        group_picker.set_visible(false);
        let editor_text_handles = editor.text_fields();
        let editor_protocol_handle = editor.protocol_handle();
        let endpoint_blurred = Rc::new(RefCell::new(None::<(String, String, String, String)>));
        let raw_endpoint_blurred = Rc::clone(&endpoint_blurred);
        let endpoint_pasted = Rc::new(RefCell::new(None::<(String, String, String, String)>));
        let raw_endpoint_pasted = Rc::clone(&endpoint_pasted);
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
        let fullscreen_restore = Cell::new(None::<WindowRestore>);
        let sidebar_visible = Cell::new(true);
        let tree_mode = Rc::new(Cell::new(false));
        let raw_tree_mode = Rc::clone(&tree_mode);
        let tree_rebuilding = Cell::new(false);
        let tree_groups = RefCell::new(Vec::<Option<String>>::new());
        let tree_ids = Rc::new(RefCell::new(Vec::<String>::new()));
        let raw_tree_nodes = Rc::clone(&tree_ids);
        let tree_signature = RefCell::new(
            None::<(
                Vec<String>,
                Vec<Option<String>>,
                String,
                BTreeSet<Option<String>>,
            )>,
        );
        let recent = RefCell::new(Vec::<String>::new());
        let statuses = RefCell::new(BTreeMap::<String, ConnectionStatus>::new());
        let pending_recent = RefCell::new(None::<(String, String)>);
        let collapsed_groups = RefCell::new(BTreeSet::<Option<String>>::new());
        let group_ids = RefCell::new(Vec::<Option<String>>::new());
        let ids = Rc::new(RefCell::new(Vec::<String>::new()));
        let pending_connect = RefCell::new(None::<String>);
        let pending_once = RefCell::new(None::<TransientConnection>);
        let pending_context = RefCell::new(None::<(nwg::ControlHandle, String)>);
        let pending_tree = Rc::new(RefCell::new(None::<(String, Option<String>, u32)>));
        let raw_pending_tree = Rc::clone(&pending_tree);
        let raw_tree_ids = Rc::clone(&ids);
        let context_clicked = RefCell::new(None::<(usize, String, bool)>);
        let active = Rc::new(RefCell::new(None::<Active>));
        let raw_tree_active = Rc::clone(&active);
        let paste_reader = PasteReader::new();
        let paste_job = RefCell::new(None::<PasteJob>);
        let paste_id = Cell::new(0u64);
        let raw_active = Rc::clone(&active);
        let capture_active = Rc::clone(&active);
        let input_handle = remote_input.handle;
        let search_handle = search.handle;
        let tree_handle = group_tree.handle;
        let tree_window_handle = window.handle;
        let search_list_handle = connections.handle;
        let add_handle = add.handle;
        let quick_request = Rc::new(RefCell::new(None::<QuickRequest>));
        let raw_quick_request = Rc::clone(&quick_request);
        let raw_quick_ids = Rc::clone(&ids);
        let raw_quick_active = Rc::clone(&active);
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
            let tree_key =
                if handle == connections.handle && event == nwg::Event::OnListBoxDoubleClick {
                    Some(0x0d)
                } else if handle == connections.handle && event == nwg::Event::OnKeyPress {
                    if let nwg::EventData::OnKey(key @ (0x2e | 0x71)) = data {
                        Some(key)
                    } else {
                        None
                    }
                } else {
                    None
                };
            if let Some(key) = tree_key {
                let pending = {
                    let mut state = pending_tree.borrow_mut();
                    if state
                        .as_ref()
                        .is_some_and(|(_, _, expected)| *expected == key)
                    {
                        state.take()
                    } else {
                        None
                    }
                };
                if let Some((id, source, _)) = pending {
                    let selected = connections
                        .selection()
                        .and_then(|index| ids.borrow().get(index).cloned());
                    let current = active
                        .borrow()
                        .as_ref()
                        .map(|session| session.connection_id.clone());
                    if selected.as_deref() != Some(id.as_str()) || current != source {
                        explanation.set_text("選取的連線或工作階段已變更，請重新操作。");
                        return;
                    }
                }
            }
            if event == nwg::Event::OnKeyPress && handle == connections.handle {
                let target = if matches!(data, nwg::EventData::OnKey(0x5d)) {
                    connections.selection().and_then(|index| {
                        ids.borrow().get(index).cloned().map(|id| (index, id, true))
                    })
                } else {
                    None
                };
                *context_clicked.borrow_mut() = target;
            }
            if event == nwg::Event::OnButtonClick {
                let pending = {
                    let mut pending = pending_context.borrow_mut();
                    if pending
                        .as_ref()
                        .is_some_and(|(expected, _)| *expected == handle)
                    {
                        pending.take()
                    } else {
                        None
                    }
                };
                if let Some((expected, id)) = pending {
                    if expected == handle
                        && connections
                            .selection()
                            .and_then(|index| ids.borrow().get(index).cloned())
                            .as_deref()
                            != Some(id.as_str())
                    {
                        explanation.set_text("選取的連線已變更，請重新操作。");
                        return;
                    }
                }
            }
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
                        search.set_visible(true);
                        group_picker.set_visible(true);
                        collapse_group.set_visible(true);
                        expand_groups.set_visible(true);
                        tree_toggle.set_visible(true);
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
                            (
                                &search,
                                &group_picker,
                                [&collapse_group, &expand_groups, &tree_toggle],
                            ),
                            [
                                &refresh, &logout, &connect, &add, &edit, &delete, &verify,
                                &duplicate,
                            ],
                        );
                        refresh_connections(
                            &runtime,
                            &opened,
                            &connections,
                            (&explanation, true),
                            (&ids, &statuses),
                            (&search.text(), &collapsed_groups.borrow()),
                            (&group_picker, &group_ids),
                        );
                        update_library_actions(
                            &connections,
                            &ids,
                            [&connect, &edit, &delete, &verify, &duplicate],
                        );
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
                        refresh_connections(
                            &runtime,
                            store,
                            &connections,
                            (&explanation, true),
                            (&ids, &statuses),
                            (&search.text(), &collapsed_groups.borrow()),
                            (&group_picker, &group_ids),
                        );
                        update_library_actions(
                            &connections,
                            &ids,
                            [&connect, &edit, &delete, &verify, &duplicate],
                        );
                    }
                } else {
                    expires.set(None);
                    pending_connect.borrow_mut().take();
                    pending_once.borrow_mut().take();
                    quick_request.borrow_mut().take();
                    active.borrow_mut().take();
                    leave_fullscreen(&event_window, &fullscreen_restore);
                    fullscreen.set_text("全螢幕");
                    fullscreen.set_visible(false);
                    sidebar_toggle.set_visible(false);
                    recent_button.set_visible(false);
                    recent_menu.set_visible(false);
                    recent.borrow_mut().clear();
                    statuses.borrow_mut().clear();
                    tree_mode.set(false);
                    tree_toggle.set_text("樹狀檢視");
                    tree_signature.borrow_mut().take();
                    pending_recent.borrow_mut().take();
                    session_footer.set_visible(false);
                    restore_library_header(&heading, &explanation);
                    paste_job.borrow_mut().take();
                    pending_resize.set(None);
                    logout.set_position(145, 335);
                    probing.borrow_mut().take();
                    store.borrow_mut().take();
                    collapsed_groups.borrow_mut().clear();
                    group_ids.borrow_mut().clear();
                    group_picker.set_collection(vec!["所有群組".into()]);
                    group_picker.set_selection(Some(0));
                    search.set_visible(false);
                    group_picker.set_visible(false);
                    collapse_group.set_visible(false);
                    expand_groups.set_visible(false);
                    tree_toggle.set_visible(false);
                    group_tree.set_visible(false);
                    search.set_text("");
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
                    secure_attention.set_visible(false);
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
                && !editor.is_visible()
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
                    temporary: false,
                });
                explanation.set_text("正在取得主機身分；此探測不傳送帳密。");
            } else if event == nwg::Event::OnMousePress(nwg::MousePressEvent::MousePressRightDown)
                && handle == connections.handle
            {
                let target = list_item_under_cursor(&connections).and_then(|index| {
                    ids.borrow()
                        .get(index)
                        .cloned()
                        .map(|id| (index, id, false))
                });
                *context_clicked.borrow_mut() = target;
            } else if event == nwg::Event::OnContextMenu && handle == connections.handle {
                let Some((index, id, keyboard)) = context_clicked.borrow_mut().take() else {
                    return;
                };
                if !expires.get().is_some_and(|end| Instant::now() < end)
                    || active.borrow().is_some()
                    || editor.is_visible()
                {
                    return;
                }
                if ids.borrow().get(index) != Some(&id) {
                    return;
                }
                connections.set_selection(Some(index));
                update_library_actions(
                    &connections,
                    &ids,
                    [&connect, &edit, &delete, &verify, &duplicate],
                );
                let anchor = if keyboard {
                    let Some(point) = list_context_anchor(&connections, index) else {
                        return;
                    };
                    Some(point)
                } else {
                    None
                };
                let Some(choice) = connection_context_menu(&event_window, anchor) else {
                    return;
                };
                if !expires.get().is_some_and(|end| Instant::now() < end)
                    || active.borrow().is_some()
                    || editor.is_visible()
                {
                    return;
                }
                let button = [&connect, &edit, &duplicate, &verify, &delete][choice];
                if let Some(hwnd) = button.handle.hwnd() {
                    *pending_context.borrow_mut() = Some((button.handle, id));
                    if unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                            hwnd.cast(),
                            0x00f5,
                            0,
                            0,
                        )
                    } == 0
                    {
                        pending_context.borrow_mut().take();
                        explanation.set_text("無法執行選單操作，請重試。");
                    }
                }
            } else if event == nwg::Event::OnButtonClick
                && handle == tree_toggle.handle
                && expires.get().is_some_and(|end| Instant::now() < end)
                && !editor.is_visible()
            {
                let enabled = !tree_mode.get();
                tree_mode.set(enabled);
                tree_toggle.set_text(if enabled {
                    "清單檢視"
                } else {
                    "樹狀檢視"
                });
                tree_signature.borrow_mut().take();
                group_tree.set_visible(false);
                if !enabled {
                    connections.set_focus();
                }
            } else if handle == group_tree.handle
                && tree_mode.get()
                && !tree_rebuilding.get()
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                match event {
                    nwg::Event::OnTreeItemSelectionChanged => {
                        if let nwg::EventData::OnTreeItemSelectionChanged { new, .. } = data {
                            let param = group_tree.item_param(&new).unwrap_or(0);
                            if param > 0 {
                                let id = tree_ids.borrow().get((param - 1) as usize).cloned();
                                let index = id.and_then(|id| {
                                    ids.borrow().iter().position(|item| *item == id)
                                });
                                connections.set_selection(index);
                            } else {
                                connections.set_selection(None);
                                if param < 0 {
                                    let group =
                                        tree_groups.borrow().get((-param - 1) as usize).cloned();
                                    let index = group
                                        .and_then(|key| {
                                            group_ids.borrow().iter().position(|item| *item == key)
                                        })
                                        .map(|index| index + 1);
                                    group_picker.set_selection(index.or(Some(0)));
                                }
                            }
                            update_library_actions(
                                &connections,
                                &ids,
                                [&connect, &edit, &delete, &verify, &duplicate],
                            );
                        }
                    }
                    nwg::Event::OnTreeItemExpanded if search.text().trim().is_empty() => {
                        if let nwg::EventData::OnTreeItemUpdate { item, action } = data {
                            let param = group_tree.item_param(&item).unwrap_or(0);
                            if param < 0 {
                                if let Some(key) =
                                    tree_groups.borrow().get((-param - 1) as usize).cloned()
                                {
                                    match action {
                                        nwg::TreeItemAction::Expand(nwg::ExpandState::Expand) => {
                                            collapsed_groups.borrow_mut().remove(&key);
                                        }
                                        nwg::TreeItemAction::Expand(nwg::ExpandState::Collapse) => {
                                            collapsed_groups.borrow_mut().insert(key);
                                        }
                                        _ => return,
                                    }
                                    if let Some(opened) = store.borrow().as_ref() {
                                        refresh_connections(
                                            &runtime,
                                            opened,
                                            &connections,
                                            (&explanation, false),
                                            (&ids, &statuses),
                                            (&search.text(), &collapsed_groups.borrow()),
                                            (&group_picker, &group_ids),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    nwg::Event::OnTreeViewRightClick
                        if active.borrow().is_none() && !editor.is_visible() =>
                    {
                        let (x, y) = nwg::GlobalCursor::local_position(group_tree.handle, None);
                        let Some(item) = tree_item_at(&group_tree, x, y) else {
                            return;
                        };
                        let Some(param) = group_tree.item_param(&item).filter(|value| *value > 0)
                        else {
                            return;
                        };
                        let Some(id) = tree_ids.borrow().get((param - 1) as usize).cloned() else {
                            return;
                        };
                        let Some(index) = ids.borrow().iter().position(|entry| *entry == id) else {
                            return;
                        };
                        if let Some(hwnd) = group_tree.handle.hwnd() {
                            unsafe {
                                use windows_sys::Win32::UI::Controls::{
                                    TVGN_CARET, TVM_SELECTITEM,
                                };
                                windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                                    hwnd.cast(),
                                    TVM_SELECTITEM,
                                    TVGN_CARET as usize,
                                    item.handle as isize,
                                );
                            }
                        }
                        connections.set_selection(Some(index));
                        update_library_actions(
                            &connections,
                            &ids,
                            [&connect, &edit, &delete, &verify, &duplicate],
                        );
                        let Some(choice) = connection_context_menu(&event_window, None) else {
                            return;
                        };
                        if !expires.get().is_some_and(|end| Instant::now() < end)
                            || active.borrow().is_some()
                            || editor.is_visible()
                            || connections
                                .selection()
                                .and_then(|i| ids.borrow().get(i).cloned())
                                .as_deref()
                                != Some(id.as_str())
                        {
                            return;
                        }
                        let button = [&connect, &edit, &duplicate, &verify, &delete][choice];
                        if let Some(hwnd) = button.handle.hwnd() {
                            *pending_context.borrow_mut() = Some((button.handle, id));
                            if unsafe {
                                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                                    hwnd.cast(),
                                    0x00f5,
                                    0,
                                    0,
                                )
                            } == 0
                            {
                                pending_context.borrow_mut().take();
                                explanation.set_text("無法執行選單操作，請重試。");
                            }
                        }
                    }
                    nwg::Event::OnTreeViewDoubleClick => {
                        let leaf = tree_selected_item(&group_tree)
                            .and_then(|item| group_tree.item_param(&item))
                            .filter(|param| *param > 0)
                            .and_then(|param| tree_ids.borrow().get((param - 1) as usize).cloned());
                        if let Some(id) = leaf {
                            let selected = connections
                                .selection()
                                .and_then(|index| ids.borrow().get(index).cloned());
                            if selected.as_deref() != Some(id.as_str())
                                || pending_tree.borrow().is_some()
                            {
                                return;
                            }
                            let source = active
                                .borrow()
                                .as_ref()
                                .map(|session| session.connection_id.clone());
                            *pending_tree.borrow_mut() = Some((id, source, 0x0d));
                            if let (Some(window), Some(list)) =
                                (event_window.handle.hwnd(), connections.handle.hwnd())
                            {
                                unsafe {
                                    use windows_sys::Win32::UI::WindowsAndMessaging::{
                                        GetDlgCtrlID, PostMessageW, WM_COMMAND,
                                    };
                                    let command =
                                        GetDlgCtrlID(list.cast()) as usize | (2usize << 16);
                                    if PostMessageW(
                                        window.cast(),
                                        WM_COMMAND,
                                        command,
                                        list as isize,
                                    ) == 0
                                    {
                                        pending_tree.borrow_mut().take();
                                    }
                                }
                            } else {
                                pending_tree.borrow_mut().take();
                            }
                        }
                    }
                    _ => {}
                }
            } else if event == nwg::Event::OnListBoxSelect && handle == connections.handle {
                update_library_actions(
                    &connections,
                    &ids,
                    [&connect, &edit, &delete, &verify, &duplicate],
                );
            } else if ((event == nwg::Event::OnButtonClick
                && (handle == connect.handle || handle == recent_button.handle))
                || (event == nwg::Event::OnListBoxDoubleClick && handle == connections.handle)
                || (event == nwg::Event::OnKeyPress
                    && handle == connections.handle
                    && matches!(data, nwg::EventData::OnKey(0x0d))))
                && expires.get().is_some_and(|end| Instant::now() < end)
                && (active.borrow().is_none()
                    || handle == connections.handle
                    || handle == recent_button.handle)
                && (handle != recent_button.handle || active.borrow().is_some())
                && !editor.is_visible()
            {
                let transient_connection = if handle == connect.handle && active.borrow().is_none()
                {
                    pending_once.borrow_mut().take()
                } else {
                    None
                };
                let is_transient = transient_connection.is_some();
                let requested_id = if is_transient {
                    Some("__temporary_session__".to_string())
                } else if handle == recent_button.handle {
                    let current = active
                        .borrow()
                        .as_ref()
                        .filter(|session| session.shown_frame)
                        .map(|session| session.connection_id.clone());
                    if current.is_none() {
                        return;
                    }
                    if let Some((target, source)) = pending_recent.borrow_mut().take() {
                        if current.as_deref() != Some(source.as_str()) {
                            return;
                        }
                        Some(target)
                    } else {
                        recent
                            .borrow()
                            .iter()
                            .rev()
                            .find(|id| Some(id.as_str()) != current.as_deref())
                            .cloned()
                    }
                } else if handle == connect.handle {
                    pending_connect.borrow_mut().take()
                } else {
                    None
                };
                if handle == recent_button.handle && requested_id.is_none() {
                    return;
                }
                let Some(id) = requested_id.or_else(|| {
                    connections
                        .selection()
                        .and_then(|index| ids.borrow().get(index).cloned())
                }) else {
                    explanation.set_text("請先選擇一筆連線。");
                    return;
                };
                let Some(store) = store.borrow().as_ref().cloned() else {
                    return;
                };
                if handle == recent_button.handle
                    && !runtime
                        .block_on(store.list())
                        .is_ok_and(|items| items.iter().any(|item| item.id == id))
                {
                    explanation.set_text("最近的連線已不存在，請從清單選擇其他連線。");
                    return;
                }
                let previous_id = active
                    .borrow()
                    .as_ref()
                    .map(|session| session.connection_id.clone());
                if let Some(previous_id) = previous_id {
                    if previous_id == id {
                        return;
                    }
                    let choice = nwg::modal_message(
                        &event_window.handle,
                        &nwg::MessageParams {
                            title: "切換連線",
                            content: "切換連線會中斷目前的工作階段。要繼續嗎？",
                            buttons: nwg::MessageButtons::YesNo,
                            icons: nwg::MessageIcons::Question,
                        },
                    );
                    if choice != nwg::MessageChoice::Yes
                        || !expires.get().is_some_and(|end| Instant::now() < end)
                        || active
                            .borrow()
                            .as_ref()
                            .is_none_or(|session| session.connection_id != previous_id)
                    {
                        return;
                    }
                    release_keys(&active, &modifiers, &altgr, &control_keys);
                    if active
                        .borrow_mut()
                        .take()
                        .is_some_and(|session| !session.transient)
                    {
                        statuses
                            .borrow_mut()
                            .insert(previous_id, ConnectionStatus::Disconnected);
                    }
                    paste_job.borrow_mut().take();
                    pending_resize.set(None);
                    pending_key_release.set(false);
                    release_mouse(&mouse_mask);
                    image.set_bitmap(None);
                    remote_input.set_text("");
                }
                probing.borrow_mut().take();
                sidebar_visible.set(true);
                heading.set_visible(false);
                let (width, height) = layout_session(
                    &event_window,
                    &image,
                    (&remote_input, &session_footer, &explanation),
                    &connections,
                    (
                        &search,
                        &group_picker,
                        [&collapse_group, &expand_groups, &tree_toggle],
                    ),
                    true,
                    [
                        &logout,
                        &disconnect,
                        &paste,
                        &secure_attention,
                        &fullscreen,
                        &sidebar_toggle,
                        &recent_button,
                        &recent_menu,
                    ],
                );
                let (input, receiver) = mpsc::channel(64);
                let (frame_tx, frames) = watch::channel(None);
                let (clipboard_tx, clipboard) = watch::channel(None);
                let (input_policy_tx, input_policy) = watch::channel(None);
                let (metadata_tx, metadata) = watch::channel(None);
                let cancel = CancellationToken::new();
                let task_cancel = cancel.clone();
                let connection_id = id.clone();
                let marker_id = connection_id.clone();
                let task = runtime.spawn(async move {
                    let connection = match transient_connection {
                        Some(connection) => connection,
                        None => TransientConnection(
                            store
                                .get(&id)
                                .await
                                .map_err(|e| e.to_string())?
                                .ok_or("找不到連線")?,
                        ),
                    };
                    let connection = &connection.0;
                    let read_only = connection
                        .params
                        .get("read-only")
                        .is_some_and(|value| value == "true");
                    let encoding =
                        if connection.protocol == "ssh" || connection.protocol == "telnet" {
                            Some(
                                connection
                                    .params
                                    .get("character-encoding")
                                    .filter(|value| !value.is_empty())
                                    .cloned()
                                    .unwrap_or_else(|| "UTF-8".into()),
                            )
                        } else {
                            None
                        };
                    metadata_tx.send_replace(Some((
                        connection.name.clone(),
                        connection.protocol.clone(),
                        encoding,
                    )));
                    let paste_allowed = !read_only
                        && !connection
                            .params
                            .get("disable-paste")
                            .is_some_and(|value| value == "true");
                    input_policy_tx.send_replace(Some((
                        read_only,
                        paste_allowed,
                        connection.protocol == "rdp",
                    )));
                    driver::run(
                        connection,
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
                    connection_id,
                    cancel,
                    input,
                    frames,
                    clipboard,
                    input_policy,
                    metadata,
                    shown_frame: false,
                    transient: is_transient,
                    displayed_size: None,
                    paste_limit: Arc::new(tokio::sync::Semaphore::new(1)),
                    task,
                });
                pending_recent.borrow_mut().take();
                if !is_transient {
                    statuses
                        .borrow_mut()
                        .insert(marker_id, ConnectionStatus::Connecting);
                }
                update_connection_statuses(&connections, &ids, &statuses);
                connections.set_visible(true);
                search.set_visible(true);
                group_picker.set_visible(true);
                collapse_group.set_visible(true);
                expand_groups.set_visible(true);
                tree_toggle.set_visible(true);
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
                secure_attention.set_visible(false);
                secure_attention.set_enabled(false);
                fullscreen.set_visible(true);
                sidebar_toggle.set_text("收折側欄");
                sidebar_toggle.set_visible(true);
                recent_button.set_visible(true);
                recent_button.set_enabled(false);
                recent_menu.set_visible(true);
                recent_menu.set_enabled(false);
                session_footer.set_text("等待連線");
                session_footer.set_visible(true);
                explanation.set_text("正在連線；點選遠端畫面後可輸入。");
            } else if event == nwg::Event::OnButtonClick && handle == disconnect.handle {
                if let Some(session) = active.borrow_mut().take() {
                    if !session.transient {
                        statuses.borrow_mut().insert(
                            session.connection_id.clone(),
                            ConnectionStatus::Disconnected,
                        );
                    }
                }
                update_connection_statuses(&connections, &ids, &statuses);
                leave_fullscreen(&event_window, &fullscreen_restore);
                fullscreen.set_text("全螢幕");
                fullscreen.set_visible(false);
                sidebar_toggle.set_visible(false);
                recent_button.set_visible(false);
                recent_menu.set_visible(false);
                pending_recent.borrow_mut().take();
                session_footer.set_visible(false);
                restore_library_header(&heading, &explanation);
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
                secure_attention.set_visible(false);
                connections.set_visible(true);
                search.set_visible(true);
                group_picker.set_visible(true);
                collapse_group.set_visible(true);
                expand_groups.set_visible(true);
                tree_toggle.set_visible(true);
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
                    (
                        &search,
                        &group_picker,
                        [&collapse_group, &expand_groups, &tree_toggle],
                    ),
                    [
                        &refresh, &logout, &connect, &add, &edit, &delete, &verify, &duplicate,
                    ],
                );
                explanation.set_text("連線已結束。");
            } else if ((event == nwg::Event::OnButtonClick
                && (handle == add.handle || handle == edit.handle || handle == duplicate.handle))
                || (event == nwg::Event::OnKeyPress
                    && handle == connections.handle
                    && matches!(data, nwg::EventData::OnKey(0x71))))
                && expires.get().is_some_and(|end| Instant::now() < end)
                && (active.borrow().is_none()
                    || (handle == add.handle && quick_request.borrow().is_some()))
                && !editor.is_visible()
            {
                let Some(opened) = store.borrow().as_ref().cloned() else {
                    return;
                };
                let quick = if handle == add.handle {
                    quick_request.borrow_mut().take()
                } else {
                    None
                };
                if let Some(request) = quick.as_ref() {
                    let address = &request.address;
                    if search.text() != *address
                        || active
                            .borrow()
                            .as_ref()
                            .map(|session| &session.connection_id)
                            != request.source.as_ref()
                    {
                        explanation.set_text("搜尋內容或工作階段已變更，請重新操作。");
                        return;
                    }
                    let is_endpoint = match Editor::is_quick_endpoint(address) {
                        Ok(value) => value,
                        Err(message) => {
                            explanation.set_text(message);
                            return;
                        }
                    };
                    let entries = match runtime.block_on(opened.list()) {
                        Ok(entries) => entries,
                        Err(_) => {
                            explanation.set_text("無法讀取連線設定。");
                            return;
                        }
                    };
                    let existing = if is_endpoint {
                        match Editor::existing_quick(address, &entries) {
                            Ok(existing) => existing,
                            Err(message) => {
                                explanation.set_text(message);
                                return;
                            }
                        }
                    } else {
                        let selected = connections
                            .selection()
                            .and_then(|index| ids.borrow().get(index).cloned());
                        if selected != request.selected {
                            explanation.set_text("搜尋選取項目已變更，請重新操作。");
                            return;
                        }
                        if selected.is_none() {
                            explanation.set_text("沒有符合的連線，請調整搜尋條件。");
                            return;
                        }
                        selected
                    };
                    if let Some(id) = existing {
                        if let Some(source) = active
                            .borrow()
                            .as_ref()
                            .filter(|session| session.shown_frame)
                            .map(|session| session.connection_id.clone())
                        {
                            if source == id {
                                explanation.set_text("已連線至此主機。");
                                return;
                            }
                            *pending_recent.borrow_mut() = Some((id, source));
                            let posted = recent_button.handle.hwnd().is_some_and(|button| unsafe {
                                use windows_sys::Win32::UI::WindowsAndMessaging::{
                                    GetDlgCtrlID, PostMessageW, WM_COMMAND,
                                };
                                event_window.handle.hwnd().is_some_and(|window| {
                                    PostMessageW(
                                        window.cast(),
                                        WM_COMMAND,
                                        GetDlgCtrlID(button.cast()) as usize,
                                        button as isize,
                                    ) != 0
                                })
                            });
                            if !posted {
                                pending_recent.borrow_mut().take();
                                explanation.set_text("無法啟動切換，請重試。");
                            }
                            return;
                        }
                        if active.borrow().is_some() {
                            explanation.set_text("目前連線尚未準備完成，請稍候。");
                            return;
                        }
                        if let Some(entry) = entries.iter().find(|entry| entry.id == id) {
                            collapsed_groups
                                .borrow_mut()
                                .remove(&connection_group_key(&entry.name));
                        }
                        search.set_text("");
                        let index = ids.borrow().iter().position(|item| item == &id);
                        let Some(index) = index else {
                            explanation.set_text("無法重新載入連線，請重試。");
                            return;
                        };
                        connections.set_selection(Some(index));
                        *pending_connect.borrow_mut() = Some(id);
                        let posted = connect.handle.hwnd().is_some_and(|hwnd| unsafe {
                            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                                hwnd.cast(),
                                0x00f5,
                                0,
                                0,
                            ) != 0
                        });
                        if !posted {
                            pending_connect.borrow_mut().take();
                            explanation.set_text("無法啟動連線，請重試。");
                        }
                        return;
                    }
                    if active.borrow().is_some() {
                        explanation.set_text("此目的地尚未保存；請先中斷目前連線再新增。");
                        return;
                    }
                }
                let editing = handle == edit.handle
                    || (handle == connections.handle
                        && matches!(data, nwg::EventData::OnKey(0x71)));
                let base = if editing || handle == duplicate.handle {
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
                if (editing || handle == duplicate.handle) && base.is_none() {
                    explanation.set_text("找不到連線。");
                    return;
                }
                probing.borrow_mut().take();
                connections.set_visible(false);
                search.set_visible(false);
                group_picker.set_visible(false);
                collapse_group.set_visible(false);
                expand_groups.set_visible(false);
                tree_toggle.set_visible(false);
                group_tree.set_visible(false);
                refresh.set_visible(false);
                logout.set_visible(false);
                connect.set_visible(false);
                add.set_visible(false);
                edit.set_visible(false);
                delete.set_visible(false);
                verify.set_visible(false);
                duplicate.set_visible(false);
                endpoint_pasted.borrow_mut().take();
                endpoint_blurred.borrow_mut().take();
                if let Some(request) = quick {
                    if let Err(message) = editor.begin_quick(&request.address) {
                        explanation.set_text(message);
                        return;
                    }
                    explanation.set_text("請確認主機與登入設定；SSH 須先核對主機身分。");
                } else if handle == duplicate.handle {
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
                && (handle == editor.save.handle
                    || handle == editor.save_connect.handle
                    || handle == editor.connect_once.handle
                    || handle == editor.cancel.handle))
                || (event == nwg::Event::OnKeyPress
                    && editor.is_text_field(handle)
                    && matches!(data, nwg::EventData::OnKey(0x0d | 0x1b))))
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                let Some(opened) = store.borrow().as_ref().cloned() else {
                    return;
                };
                let connect_after_save = handle == editor.save_connect.handle;
                let connect_once = handle == editor.connect_once.handle;
                let mut saved_id = None;
                if connect_once {
                    let (id, mut draft) = match editor.draft() {
                        Ok(draft) => draft,
                        Err(message) => {
                            explanation.set_text(message);
                            return;
                        }
                    };
                    if id.is_some() {
                        clear_draft_secrets(&mut draft);
                        explanation.set_text("免保存連線只適用新增設定。");
                        return;
                    }
                    if let Err(error) = cracamole_store::validate(&draft) {
                        clear_draft_secrets(&mut draft);
                        explanation.set_text(error.message);
                        return;
                    }
                    if draft.protocol == "ssh" {
                        let host = draft.params.get("hostname").cloned().unwrap_or_default();
                        let port = draft
                            .params
                            .get("port")
                            .and_then(|value| value.parse::<u16>().ok())
                            .unwrap_or(22);
                        let entries = match runtime.block_on(opened.list()) {
                            Ok(entries) => entries,
                            Err(_) => {
                                clear_draft_secrets(&mut draft);
                                explanation.set_text("無法讀取已保存的 SSH 主機身分。");
                                return;
                            }
                        };
                        let independent = match opened.ssh_identity(&host, port) {
                            Ok(identity) => identity,
                            Err(_) => {
                                clear_draft_secrets(&mut draft);
                                explanation.set_text("無法讀取已保存的 SSH 主機身分。");
                                return;
                            }
                        };
                        match saved_ssh_identity(&draft, &entries, independent.as_deref()) {
                            Ok(Some(key)) => {
                                draft.params.insert("host-key".into(), key);
                            }
                            Ok(None) => {
                                clear_draft_secrets(&mut draft);
                                if probing.borrow().is_some() {
                                    explanation.set_text("主機身分探測進行中，請稍候。");
                                    return;
                                }
                                let cancel = CancellationToken::new();
                                let task_cancel = cancel.clone();
                                let probes = probes.clone();
                                let probe_host = host.clone();
                                let task = runtime.spawn(async move {
                                    probes
                                        .ssh(&probe_host, port, None, &task_cancel)
                                        .await
                                        .map(Identity::Ssh)
                                });
                                *probing.borrow_mut() = Some(IdentityProbe {
                                    cancel,
                                    task,
                                    temporary: true,
                                    target: PublicConnection {
                                        id: String::new(),
                                        name: String::new(),
                                        protocol: "ssh".into(),
                                        params: BTreeMap::from([
                                            ("hostname".into(), host),
                                            ("port".into(), port.to_string()),
                                        ]),
                                        configured_secrets: Vec::new(),
                                    },
                                });
                                explanation.set_text("正在取得 SSH 主機身分；此探測不傳送帳密。");
                                return;
                            }
                            Err(message) => {
                                clear_draft_secrets(&mut draft);
                                explanation.set_text(message);
                                return;
                            }
                        }
                    }
                    *pending_once.borrow_mut() =
                        Some(TransientConnection(cracamole_store::Connection {
                            id: "__temporary_session__".into(),
                            name: draft.name,
                            protocol: draft.protocol,
                            params: draft.params,
                        }));
                } else if handle == editor.save.handle
                    || connect_after_save
                    || matches!(data, nwg::EventData::OnKey(0x0d))
                {
                    let (id, draft) = match editor.draft() {
                        Ok(draft) => draft,
                        Err(message) => {
                            explanation.set_text(message);
                            return;
                        }
                    };
                    match runtime.block_on(opened.save(id.as_deref(), draft)) {
                        Ok(Some(saved)) => saved_id = Some(saved.id),
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
                if handle == editor.cancel.handle {
                    probing.borrow_mut().take();
                }
                if connect_after_save {
                    if let Some(id) = saved_id.as_deref() {
                        if let Ok(entries) = runtime.block_on(opened.list()) {
                            if let Some(entry) = entries.iter().find(|entry| entry.id == id) {
                                collapsed_groups
                                    .borrow_mut()
                                    .remove(&connection_group_key(&entry.name));
                            }
                        }
                    }
                    search.set_text("");
                }
                editor.show(false);
                connections.set_visible(true);
                search.set_visible(true);
                group_picker.set_visible(true);
                collapse_group.set_visible(true);
                expand_groups.set_visible(true);
                tree_toggle.set_visible(true);
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
                    (
                        &search,
                        &group_picker,
                        [&collapse_group, &expand_groups, &tree_toggle],
                    ),
                    [
                        &refresh, &logout, &connect, &add, &edit, &delete, &verify, &duplicate,
                    ],
                );
                refresh_connections(
                    &runtime,
                    &opened,
                    &connections,
                    (&explanation, true),
                    (&ids, &statuses),
                    (&search.text(), &collapsed_groups.borrow()),
                    (&group_picker, &group_ids),
                );
                update_library_actions(
                    &connections,
                    &ids,
                    [&connect, &edit, &delete, &verify, &duplicate],
                );
                if connect_after_save {
                    let index = saved_id
                        .as_ref()
                        .and_then(|id| ids.borrow().iter().position(|item| item == id));
                    if let Some(index) = index {
                        connections.set_selection(Some(index));
                        *pending_connect.borrow_mut() = saved_id;
                        if let Some(hwnd) = connect.handle.hwnd() {
                            let posted = unsafe {
                                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                                    hwnd.cast(),
                                    0x00f5,
                                    0,
                                    0,
                                )
                            };
                            if posted == 0 {
                                pending_connect.borrow_mut().take();
                                explanation.set_text("已儲存，但無法啟動連線；請手動連線。");
                            }
                        } else {
                            pending_connect.borrow_mut().take();
                            explanation.set_text("已儲存，但無法啟動連線；請手動連線。");
                        }
                    } else {
                        explanation.set_text("已儲存，但無法重新載入連線；請重新載入後再連線。");
                    }
                } else if connect_once {
                    let posted = if let (Some(window), Some(button)) =
                        (event_window.handle.hwnd(), connect.handle.hwnd())
                    {
                        unsafe {
                            use windows_sys::Win32::UI::WindowsAndMessaging::{
                                GetDlgCtrlID, PostMessageW, WM_COMMAND,
                            };
                            PostMessageW(
                                window.cast(),
                                WM_COMMAND,
                                GetDlgCtrlID(button.cast()) as usize,
                                button as isize,
                            ) != 0
                        }
                    } else {
                        false
                    };
                    if !posted {
                        pending_once.borrow_mut().take();
                        explanation.set_text("無法啟動免保存連線，請重試。");
                    }
                }
            } else if ((event == nwg::Event::OnButtonClick && handle == delete.handle)
                || (event == nwg::Event::OnKeyPress
                    && handle == connections.handle
                    && matches!(data, nwg::EventData::OnKey(0x2e))))
                && expires.get().is_some_and(|end| Instant::now() < end)
                && active.borrow().is_none()
                && !editor.is_visible()
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
                            recent.borrow_mut().retain(|entry| entry != &id);
                            statuses.borrow_mut().remove(&id);
                            refresh_connections(
                                &runtime,
                                opened,
                                &connections,
                                (&explanation, true),
                                (&ids, &statuses),
                                (&search.text(), &collapsed_groups.borrow()),
                                (&group_picker, &group_ids),
                            );
                            update_library_actions(
                                &connections,
                                &ids,
                                [&connect, &edit, &delete, &verify, &duplicate],
                            );
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
                        .is_some_and(|(_, allowed, _)| allowed)
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
            } else if event == nwg::Event::OnButtonClick
                && handle == secure_attention.handle
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                let allowed = active.borrow().as_ref().is_some_and(|session| {
                    session.shown_frame
                        && session
                            .input_policy
                            .borrow()
                            .is_some_and(|(read_only, _, rdp)| rdp && !read_only)
                });
                if allowed {
                    for (key, pressed) in [
                        (0xffe3, true),
                        (0xffe9, true),
                        (0xffff, true),
                        (0xffff, false),
                        (0xffe9, false),
                        (0xffe3, false),
                    ] {
                        send_key(&active, key, pressed);
                    }
                }
            } else if event == nwg::Event::OnButtonClick
                && (handle == collapse_group.handle || handle == expand_groups.handle)
                && expires.get().is_some_and(|end| Instant::now() < end)
                && !editor.is_visible()
            {
                let Some(opened) = store.borrow().as_ref().cloned() else {
                    return;
                };
                let chosen = group_picker
                    .selection()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| group_ids.borrow().get(index).cloned());
                let message = if handle == collapse_group.handle {
                    let group = if let Some(group) = chosen {
                        group
                    } else {
                        let Some(id) = connections
                            .selection()
                            .and_then(|index| ids.borrow().get(index).cloned())
                        else {
                            explanation.set_text("請先選擇群組或群組內的連線。");
                            return;
                        };
                        let entry = runtime
                            .block_on(opened.list())
                            .ok()
                            .and_then(|entries| entries.into_iter().find(|entry| entry.id == id));
                        let Some(entry) = entry else {
                            explanation.set_text("找不到連線，請重新載入。");
                            return;
                        };
                        connection_group_key(&entry.name)
                    };
                    let label = group.as_deref().unwrap_or("未分類").to_string();
                    collapsed_groups.borrow_mut().insert(group);
                    if search.text().trim().is_empty() {
                        format!("已收折群組：{label}。")
                    } else {
                        format!("已收折群組：{label}；搜尋期間仍顯示符合項目。")
                    }
                } else {
                    if let Some(group) = chosen {
                        let label = group.as_deref().unwrap_or("未分類").to_string();
                        collapsed_groups.borrow_mut().remove(&group);
                        format!("已展開群組：{label}。")
                    } else {
                        collapsed_groups.borrow_mut().clear();
                        "已展開所有群組。".to_string()
                    }
                };
                refresh_connections(
                    &runtime,
                    &opened,
                    &connections,
                    (&explanation, false),
                    (&ids, &statuses),
                    (&search.text(), &collapsed_groups.borrow()),
                    (&group_picker, &group_ids),
                );
                update_library_actions(
                    &connections,
                    &ids,
                    [&connect, &edit, &delete, &verify, &duplicate],
                );
                explanation.set_text(&message);
            } else if event == nwg::Event::OnButtonClick
                && handle == recent_menu.handle
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                let Some(source) = active
                    .borrow()
                    .as_ref()
                    .filter(|session| session.shown_frame)
                    .map(|session| session.connection_id.clone())
                else {
                    return;
                };
                let Some(opened) = store.borrow().as_ref().cloned() else {
                    return;
                };
                let Ok(entries) = runtime.block_on(opened.list()) else {
                    explanation.set_text("無法讀取最近連線。");
                    return;
                };
                let options: Vec<(String, String)> = recent
                    .borrow()
                    .iter()
                    .rev()
                    .filter(|id| **id != source)
                    .filter_map(|id| {
                        entries
                            .iter()
                            .find(|entry| entry.id == *id)
                            .map(|entry| (id.clone(), connection_label(entry)))
                    })
                    .collect();
                if options.is_empty() {
                    explanation.set_text("沒有可使用的其他最近連線。");
                    return;
                }
                let labels: Vec<String> = options.iter().map(|(_, label)| label.clone()).collect();
                let Some(index) = show_recent_menu(&event_window, &recent_menu, &labels) else {
                    return;
                };
                if !expires.get().is_some_and(|end| Instant::now() < end)
                    || active
                        .borrow()
                        .as_ref()
                        .is_none_or(|session| session.connection_id != source)
                {
                    return;
                }
                let Some((target, _)) = options.get(index) else {
                    return;
                };
                *pending_recent.borrow_mut() = Some((target.clone(), source));
                if !recent_button.handle.hwnd().is_some_and(|hwnd| unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                        hwnd.cast(),
                        0x00f5,
                        0,
                        0,
                    ) != 0
                }) {
                    pending_recent.borrow_mut().take();
                    explanation.set_text("無法切換最近連線，請重試。");
                }
            } else if event == nwg::Event::OnButtonClick
                && handle == sidebar_toggle.handle
                && active.borrow().is_some()
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                let visible = !sidebar_visible.get();
                sidebar_visible.set(visible);
                connections.set_visible(visible);
                search.set_visible(visible);
                group_picker.set_visible(visible);
                collapse_group.set_visible(visible);
                expand_groups.set_visible(visible);
                tree_toggle.set_visible(visible);
                group_tree.set_visible(visible && tree_mode.get());
                sidebar_toggle.set_text(if visible {
                    "收折側欄"
                } else {
                    "展開側欄"
                });
                let (width, height) = layout_session(
                    &event_window,
                    &image,
                    (&remote_input, &session_footer, &explanation),
                    &connections,
                    (
                        &search,
                        &group_picker,
                        [&collapse_group, &expand_groups, &tree_toggle],
                    ),
                    visible,
                    [
                        &logout,
                        &disconnect,
                        &paste,
                        &secure_attention,
                        &fullscreen,
                        &sidebar_toggle,
                        &recent_button,
                        &recent_menu,
                    ],
                );
                pending_resize.set(Some((width, height, Instant::now())));
            } else if event == nwg::Event::OnButtonClick
                && handle == fullscreen.handle
                && active.borrow().is_some()
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                if fullscreen_restore.get().is_some() {
                    leave_fullscreen(&event_window, &fullscreen_restore);
                    fullscreen.set_text("全螢幕");
                } else if let Some(saved) = enter_fullscreen(&event_window) {
                    fullscreen_restore.set(Some(saved));
                    fullscreen.set_text("離開全螢幕");
                } else {
                    explanation.set_text("無法切換全螢幕。");
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
                    (
                        &search,
                        &group_picker,
                        [&collapse_group, &expand_groups, &tree_toggle],
                    ),
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
                    (&remote_input, &session_footer, &explanation),
                    &connections,
                    (
                        &search,
                        &group_picker,
                        [&collapse_group, &expand_groups, &tree_toggle],
                    ),
                    sidebar_visible.get(),
                    [
                        &logout,
                        &disconnect,
                        &paste,
                        &secure_attention,
                        &fullscreen,
                        &sidebar_toggle,
                        &recent_button,
                        &recent_menu,
                    ],
                );
                pending_resize.set(Some((width, height, Instant::now())));
            } else if event == nwg::Event::OnTimerTick && handle == timer.handle {
                if tree_mode.get()
                    && control_has_visible_style(connections.handle)
                    && !editor.is_visible()
                {
                    let signature = (
                        connections.collection().clone(),
                        group_ids.borrow().clone(),
                        search.text(),
                        collapsed_groups.borrow().clone(),
                    );
                    if tree_signature.borrow().as_ref() != Some(&signature) {
                        if let Some(opened) = store.borrow().as_ref() {
                            if let Ok(entries) = runtime.block_on(opened.list()) {
                                let selected = connections
                                    .selection()
                                    .and_then(|index| ids.borrow().get(index).cloned());
                                let model =
                                    library_groups(entries, &search.text(), &BTreeSet::new());
                                tree_rebuilding.set(true);
                                let (groups, entries) = fill_group_tree(
                                    &group_tree,
                                    model,
                                    &collapsed_groups.borrow(),
                                    !search.text().trim().is_empty(),
                                    &statuses.borrow(),
                                    selected.as_deref(),
                                );
                                *tree_groups.borrow_mut() = groups;
                                *tree_ids.borrow_mut() = entries;
                                tree_rebuilding.set(false);
                                *tree_signature.borrow_mut() = Some(signature);
                            }
                        }
                    }
                    let (x, y) = connections.position();
                    let (width, height) = connections.size();
                    group_tree.set_position(x, y);
                    group_tree.set_size(width, height);
                    if !control_has_visible_style(group_tree.handle) {
                        group_tree.set_visible(true);
                        group_tree.set_focus();
                    }
                } else if control_has_visible_style(group_tree.handle) {
                    group_tree.set_visible(false);
                }
                let blurred = endpoint_blurred.borrow_mut().take();
                if let Some(blurred) = blurred.filter(|_| editor.is_visible()) {
                    if let Err(message) = editor.normalize_blurred_endpoint(&blurred) {
                        explanation.set_text(message);
                    }
                }
                let pasted = endpoint_pasted.borrow_mut().take();
                if let Some(pasted) = pasted.filter(|_| editor.is_visible()) {
                    if let Err(message) = editor.normalize_pasted_endpoint(&pasted) {
                        explanation.set_text(message);
                    }
                }
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
                    if job.temporary {
                        let host = job.target.params.get("hostname").map_or("", String::as_str);
                        let port = job
                            .target
                            .params
                            .get("port")
                            .and_then(|value| value.parse::<u16>().ok())
                            .unwrap_or(22);
                        if !editor.matches_once_ssh(host, port)
                            || !expires.get().is_some_and(|end| Instant::now() < end)
                        {
                            explanation.set_text("連線設定已變更，請重新核對。");
                        } else if let Ok(Ok(Identity::Ssh(info))) = result {
                            if info.trusted && !info.matches {
                                explanation.set_text("SSH 主機金鑰與現有信任不符，已拒絕取代。");
                            } else {
                                let content = format!("請透過其他可信管道核對 SSH 指紋：\n\n{}\n\n確認後只保存此主機的身分，不保存連線或帳密。要繼續嗎？", info.fingerprint);
                                let choice = nwg::modal_message(
                                    &event_window.handle,
                                    &nwg::MessageParams {
                                        title: "核對一次性 SSH 主機身分",
                                        content: &content,
                                        buttons: nwg::MessageButtons::YesNo,
                                        icons: nwg::MessageIcons::Question,
                                    },
                                );
                                if choice == nwg::MessageChoice::Yes
                                    && expires.get().is_some_and(|end| Instant::now() < end)
                                    && active.borrow().is_none()
                                    && editor.matches_once_ssh(host, port)
                                {
                                    if let Some(opened) = store.borrow().as_ref().cloned() {
                                        let existing = runtime.block_on(opened.list());
                                        let current = opened.ssh_identity(host, port);
                                        let draft = Draft {
                                            name: "identity".into(),
                                            protocol: "ssh".into(),
                                            params: job.target.params.clone(),
                                        };
                                        let compatible = match (existing, current) {
                                            (Ok(entries), Ok(identity)) => {
                                                match saved_ssh_identity(
                                                    &draft,
                                                    &entries,
                                                    identity.as_deref(),
                                                ) {
                                                    Ok(Some(key)) => key == info.public_key,
                                                    Ok(None) => true,
                                                    Err(_) => false,
                                                }
                                            }
                                            _ => false,
                                        };
                                        if compatible
                                            && opened
                                                .save_ssh_identity(host, port, &info.public_key)
                                                .is_ok()
                                        {
                                            let posted = editor.connect_once.handle.hwnd().is_some_and(|button| unsafe {
                                                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(button.cast(), 0x00f5, 0, 0) != 0
                                            });
                                            if !posted {
                                                explanation.set_text(
                                                    "已保存主機身分，請再次按免保存連線。",
                                                );
                                            }
                                        } else {
                                            explanation
                                                .set_text("主機信任衝突或無法保存，請重新核對。");
                                        }
                                    }
                                } else if choice == nwg::MessageChoice::Yes
                                    && expires.get().is_some_and(|end| Instant::now() < end)
                                {
                                    explanation.set_text("連線設定已變更，請重新核對。");
                                } else if expires.get().is_some_and(|end| Instant::now() < end)
                                    && editor.matches_once_ssh(host, port)
                                {
                                    explanation.set_text("未保存 SSH 主機身分，請核對後再連線。");
                                }
                            }
                        } else {
                            explanation.set_text("無法取得主機身分，請確認服務與設定。");
                        }
                    } else {
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
                                        explanation
                                            .set_text("RDP 憑證與現有指紋不符，已拒絕取代。");
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
                                            entries
                                                .into_iter()
                                                .find(|entry| entry.id == job.target.id)
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
                                            refresh_connections(
                                                &runtime,
                                                &opened,
                                                &connections,
                                                (&explanation, false),
                                                (&ids, &statuses),
                                                (&search.text(), &collapsed_groups.borrow()),
                                                (&group_picker, &group_ids),
                                            );
                                            update_library_actions(
                                                &connections,
                                                &ids,
                                                [&connect, &edit, &delete, &verify, &duplicate],
                                            );
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
                    let caption = session.metadata.borrow().clone();
                    let (name, protocol, encoding) =
                        caption.unwrap_or_else(|| ("連線中".into(), String::new(), None));
                    let dimensions = session.displayed_size.map_or_else(
                        || "等待畫面".into(),
                        |(width, height)| format!("{width}×{height}"),
                    );
                    let remaining = expires.get().map_or(0, |end| {
                        end.saturating_duration_since(Instant::now()).as_secs()
                    });
                    let encoding = encoding.map_or_else(String::new, |value| format!(" · {value}"));
                    let text = format!(
                        "{name} · {}{encoding} · 畫面 {dimensions} · 剩餘 {:02}:{:02}:{:02}",
                        protocol.to_uppercase(),
                        remaining / 3600,
                        remaining / 60 % 60,
                        remaining % 60
                    );
                    if session_footer.text() != text {
                        session_footer.set_text(&text);
                    }
                    let policy = *session.input_policy.borrow();
                    paste.set_enabled(policy.is_some_and(|(_, allowed, _)| allowed));
                    remote_input.set_enabled(policy.is_some_and(|(read_only, _, _)| !read_only));
                    secure_attention.set_visible(policy.is_some_and(|(_, _, rdp)| rdp));
                    secure_attention.set_enabled(
                        session.shown_frame
                            && policy.is_some_and(|(read_only, _, rdp)| rdp && !read_only),
                    );
                    recent_button.set_enabled(
                        session.shown_frame
                            && recent
                                .borrow()
                                .iter()
                                .any(|id| id != &session.connection_id),
                    );
                    recent_menu.set_enabled(recent_button.enabled());
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
                                    (&remote_input, &session_footer, &explanation),
                                    &connections,
                                    (
                                        &search,
                                        &group_picker,
                                        [&collapse_group, &expand_groups, &tree_toggle],
                                    ),
                                    sidebar_visible.get(),
                                    [
                                        &logout,
                                        &disconnect,
                                        &paste,
                                        &secure_attention,
                                        &fullscreen,
                                        &sidebar_toggle,
                                        &recent_button,
                                        &recent_menu,
                                    ],
                                );
                                if !session.shown_frame {
                                    session.shown_frame = true;
                                    if !session.transient {
                                        statuses.borrow_mut().insert(
                                            session.connection_id.clone(),
                                            ConnectionStatus::Connected,
                                        );
                                        update_connection_statuses(&connections, &ids, &statuses);
                                        let mut history = recent.borrow_mut();
                                        history.retain(|id| id != &session.connection_id);
                                        history.push(session.connection_id.clone());
                                        if history.len() > 20 {
                                            history.remove(0);
                                        }
                                    }
                                    if policy.is_some_and(|(read_only, _, _)| read_only) {
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
                        if let Some(mut session) = state.take() {
                            let status = match runtime.block_on(&mut session.task) {
                                Ok(Ok(())) => ConnectionStatus::Ended,
                                Ok(Err(DriverError::Remote(status))) => match status {
                                    guac_protocol::Status::ClientUnauthorized
                                    | guac_protocol::Status::ClientForbidden => {
                                        ConnectionStatus::AuthenticationFailed
                                    }
                                    guac_protocol::Status::UpstreamTimeout
                                    | guac_protocol::Status::SessionTimeout
                                    | guac_protocol::Status::ClientTimeout => {
                                        ConnectionStatus::Timeout
                                    }
                                    guac_protocol::Status::ResourceNotFound
                                    | guac_protocol::Status::UpstreamNotFound => {
                                        ConnectionStatus::HostNotFound
                                    }
                                    _ => ConnectionStatus::Failed,
                                },
                                Ok(Err(_)) | Err(_) => ConnectionStatus::Failed,
                            };
                            if !session.transient {
                                statuses
                                    .borrow_mut()
                                    .insert(session.connection_id.clone(), status);
                            }
                        }
                        drop(state);
                        update_connection_statuses(&connections, &ids, &statuses);
                        leave_fullscreen(&event_window, &fullscreen_restore);
                        fullscreen.set_text("全螢幕");
                        fullscreen.set_visible(false);
                        sidebar_toggle.set_visible(false);
                        recent_button.set_visible(false);
                        recent_menu.set_visible(false);
                        pending_recent.borrow_mut().take();
                        session_footer.set_visible(false);
                        restore_library_header(&heading, &explanation);
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
                        secure_attention.set_visible(false);
                        logout.set_position(145, 335);
                        connections.set_visible(true);
                        search.set_visible(true);
                        group_picker.set_visible(true);
                        collapse_group.set_visible(true);
                        expand_groups.set_visible(true);
                        tree_toggle.set_visible(true);
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
                            (
                                &search,
                                &group_picker,
                                [&collapse_group, &expand_groups, &tree_toggle],
                            ),
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
                        .is_none_or(|(read_only, _, _)| read_only)
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
                        .is_none_or(|(read_only, _, _)| read_only)
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
            } else if handle == search.handle
                && event == nwg::Event::OnTextInput
                && !editor.is_visible()
                && expires.get().is_some_and(|end| Instant::now() < end)
            {
                if let Some(opened) = store.borrow().as_ref() {
                    refresh_connections(
                        &runtime,
                        opened,
                        &connections,
                        (&explanation, active.borrow().is_none()),
                        (&ids, &statuses),
                        (&search.text(), &collapsed_groups.borrow()),
                        (&group_picker, &group_ids),
                    );
                    update_library_actions(
                        &connections,
                        &ids,
                        [&connect, &edit, &delete, &verify, &duplicate],
                    );
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
        let add_hwnd = add_handle.hwnd().ok_or("找不到新增按鈕")? as isize;
        let search_key_handler =
            nwg::bind_raw_event_handler(&search_handle, 0x12000, move |hwnd, message, key, _| {
                if message == 0x0100 && matches!(key, 0x26 | 0x28) {
                    let destination = if raw_tree_mode.get() {
                        tree_handle
                    } else {
                        search_list_handle
                    };
                    if let Some(list) = destination.hwnd() {
                        unsafe {
                            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                                list.cast(),
                                message,
                                key,
                                0,
                            );
                        }
                    }
                    return Some(0);
                }
                if message == 0x0100
                    && key == 0x0d
                    && unsafe {
                        use windows_sys::Win32::UI::WindowsAndMessaging::{
                            GetWindowLongPtrW, GWL_STYLE, WS_VISIBLE,
                        };
                        GetWindowLongPtrW(hwnd.cast(), GWL_STYLE) & WS_VISIBLE as isize != 0
                    }
                {
                    if let Some(address) = control_text_for_endpoint(search_handle) {
                        let Ok(mut pending) = raw_quick_request.try_borrow_mut() else {
                            return Some(0);
                        };
                        if pending.is_some() {
                            return Some(0);
                        }
                        let selected = search_list_handle.hwnd().and_then(|list| {
                            let index = unsafe {
                                windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                                    list.cast(),
                                    windows_sys::Win32::UI::WindowsAndMessaging::LB_GETCURSEL,
                                    0,
                                    0,
                                )
                            };
                            usize::try_from(index).ok().and_then(|index| {
                                raw_quick_ids.try_borrow().ok()?.get(index).cloned()
                            })
                        });
                        let Ok(active) = raw_quick_active.try_borrow() else {
                            return Some(0);
                        };
                        let source = active.as_ref().map(|session| session.connection_id.clone());
                        drop(active);
                        *pending = Some(QuickRequest {
                            address,
                            selected,
                            source,
                        });
                        if unsafe {
                            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                                add_hwnd as _,
                                0x00f5,
                                0,
                                0,
                            )
                        } == 0
                        {
                            pending.take();
                        }
                    }
                    return Some(0);
                }
                None
            })?;
        let tree_key_handler = nwg::bind_raw_event_handler(
            &tree_handle,
            0x12001,
            move |tree_hwnd, message, key, _| {
                if message == 0x0100 && matches!(key, 0x0d | 0x2e | 0x71) {
                    let selected = unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                            tree_hwnd.cast(),
                            0x110a,
                            9,
                            0,
                        )
                    };
                    let child = unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                            tree_hwnd.cast(),
                            0x110a,
                            4,
                            selected,
                        )
                    };
                    if selected == 0 || child != 0 {
                        return Some(0);
                    }
                    if let Some(list) = search_list_handle.hwnd() {
                        unsafe {
                            use windows_sys::Win32::UI::Controls::{
                                TVIF_PARAM, TVITEMW, TVM_GETITEMW,
                            };
                            use windows_sys::Win32::UI::WindowsAndMessaging::{
                                GetDlgCtrlID, PostMessageW, SendMessageW, LB_GETCURSEL, WM_COMMAND,
                            };
                            let mut item: TVITEMW = std::mem::zeroed();
                            item.mask = TVIF_PARAM;
                            item.hItem = selected as _;
                            if SendMessageW(
                                tree_hwnd.cast(),
                                TVM_GETITEMW,
                                0,
                                (&mut item as *mut TVITEMW) as isize,
                            ) == 0
                                || item.lParam <= 0
                            {
                                return Some(0);
                            }
                            let Some(id) = raw_tree_nodes
                                .try_borrow()
                                .ok()
                                .and_then(|nodes| nodes.get((item.lParam - 1) as usize).cloned())
                            else {
                                return Some(0);
                            };
                            let index = SendMessageW(list.cast(), LB_GETCURSEL, 0, 0);
                            let list_id = usize::try_from(index).ok().and_then(|index| {
                                raw_tree_ids.try_borrow().ok()?.get(index).cloned()
                            });
                            if list_id.as_deref() != Some(id.as_str()) {
                                return Some(0);
                            }
                            let Ok(active) = raw_tree_active.try_borrow() else {
                                return Some(0);
                            };
                            let source =
                                active.as_ref().map(|session| session.connection_id.clone());
                            drop(active);
                            let Ok(mut pending) = raw_pending_tree.try_borrow_mut() else {
                                return Some(0);
                            };
                            if pending.is_some() {
                                return Some(0);
                            }
                            *pending = Some((id, source, key as u32));
                            let posted = if key == 0x0d {
                                if let Some(window) = tree_window_handle.hwnd() {
                                    let command =
                                        GetDlgCtrlID(list.cast()) as usize | (2usize << 16);
                                    PostMessageW(window.cast(), WM_COMMAND, command, list as isize)
                                } else {
                                    0
                                }
                            } else {
                                PostMessageW(list.cast(), message, key, 0)
                            };
                            if posted == 0 {
                                pending.take();
                            }
                        }
                    }
                    return Some(0);
                }
                None
            },
        )?;
        for (index, handle) in editor_text_handles.iter().enumerate() {
            let cancel = editor_cancel_handle.hwnd().ok_or("找不到取消按鈕")? as isize;
            let endpoint_blurred = Rc::clone(&raw_endpoint_blurred);
            let endpoint_pasted = Rc::clone(&raw_endpoint_pasted);
            let endpoint_handles = [
                editor_text_handles[1],
                editor_text_handles[2],
                editor_text_handles[3],
                editor_protocol_handle,
            ];
            let handler = nwg::bind_raw_event_handler(
                handle,
                0x11000 + index,
                move |hwnd, message, key, param| {
                    if index == 1 && message == 0x0302 {
                        // Let the native edit control perform the paste first, then capture
                        // its result. The timer applies it only if no field has changed.
                        let before = control_text_for_endpoint(endpoint_handles[0]);
                        #[link(name = "comctl32")]
                        unsafe extern "system" {
                            fn DefSubclassProc(
                                hwnd: *mut std::ffi::c_void,
                                message: u32,
                                key: usize,
                                param: isize,
                            ) -> isize;
                        }
                        let result = unsafe { DefSubclassProc(hwnd.cast(), message, key, param) };
                        let [host, port, user, protocol] =
                            endpoint_handles.map(control_text_for_endpoint);
                        *endpoint_pasted.borrow_mut() = host
                            .filter(|host| before.as_ref() != Some(host))
                            .zip(port)
                            .zip(user)
                            .zip(protocol)
                            .map(|(((host, port), user), protocol)| (host, port, user, protocol));
                        return Some(result);
                    }
                    if index == 1 && message == 0x0008 {
                        let [host, port, user, protocol] =
                            endpoint_handles.map(control_text_for_endpoint);
                        *endpoint_blurred.borrow_mut() =
                            host.zip(port).zip(user).zip(protocol).map(
                                |(((host, port), user), protocol)| (host, port, user, protocol),
                            );
                    }
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
                },
            )?;
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
        nwg::unbind_raw_event_handler(&search_key_handler)?;
        nwg::unbind_raw_event_handler(&tree_key_handler)?;
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
                "[SSH] [機房] 伺服器 — alice@[2001:db8::1]:22 · 已信任 · 唯讀"
            );
        }

        #[test]
        fn named_default_group_does_not_share_ungrouped_fold_state() {
            let mut folded = BTreeSet::new();
            folded.insert(connection_group_key("未分類 / server"));
            assert!(folded.contains(&connection_group_key("未分類 / other")));
            assert!(!folded.contains(&connection_group_key("ordinary server")));
        }

        #[test]
        fn transient_ssh_uses_only_unambiguous_identity_for_same_endpoint() {
            let draft = Draft {
                name: "temporary".into(),
                protocol: "ssh".into(),
                params: BTreeMap::from([("hostname".into(), "HOST.example".into())]),
            };
            let entry = |id: &str, host: &str, port: &str, key: &str| PublicConnection {
                id: id.into(),
                name: id.into(),
                protocol: "ssh".into(),
                params: BTreeMap::from([
                    ("hostname".into(), host.into()),
                    ("port".into(), port.into()),
                    ("host-key".into(), key.into()),
                ]),
                configured_secrets: vec!["password".into()],
            };
            let matching = entry("1", "host.example", "22", "ssh-ed25519 accepted");
            let other_port = entry("2", "host.example", "2222", "other");
            assert_eq!(
                saved_ssh_identity(&draft, &[matching, other_port], None)
                    .unwrap()
                    .as_deref(),
                Some("ssh-ed25519 accepted")
            );
            let conflicting = [
                entry("1", "host.example", "", "ssh-ed25519 accepted"),
                entry("2", "host.example", "22", "@revoked * ssh-ed25519 revoked"),
            ];
            assert!(saved_ssh_identity(&draft, &conflicting, None).is_err());
            assert!(saved_ssh_identity(
                &draft,
                &[entry("4", "host.example", "invalid", "key")],
                None
            )
            .unwrap()
            .is_none());
            assert!(
                saved_ssh_identity(&draft, &[entry("3", "elsewhere", "22", "key")], None)
                    .unwrap()
                    .is_none()
            );
        }

        #[test]
        fn group_model_preserves_hidden_groups_and_searches_folded_members() {
            fn entry(id: &str, name: &str, host: &str) -> PublicConnection {
                PublicConnection {
                    id: id.into(),
                    name: name.into(),
                    protocol: "ssh".into(),
                    params: BTreeMap::from([("hostname".into(), host.into())]),
                    configured_secrets: Vec::new(),
                }
            }
            let entries = || {
                vec![
                    entry("1", "機房 / beta", "beta.example"),
                    entry("2", "未分類 / alpha", "alpha.example"),
                    entry("3", "standalone", "other.example"),
                    entry("4", "機房 / alpha", "alpha.example"),
                ]
            };
            let folded = BTreeSet::from([Some("機房".to_string())]);
            let groups = library_groups(entries(), "", &folded);
            let room = groups
                .iter()
                .find(|group| group.key.as_deref() == Some("機房"))
                .unwrap();
            assert!(room.visible.is_empty());
            assert!(groups
                .iter()
                .any(|group| group.key.is_none() && group.visible.len() == 1));
            assert!(groups
                .iter()
                .any(|group| group.key.as_deref() == Some("未分類") && group.visible.len() == 1));
            let groups = library_groups(entries(), "alpha", &folded);
            let room = groups
                .iter()
                .find(|group| group.key.as_deref() == Some("機房"))
                .unwrap();
            assert_eq!(
                room.visible
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>(),
                ["4"]
            );
            let groups = library_groups(entries(), "機房", &folded);
            let room = groups
                .iter()
                .find(|group| group.key.as_deref() == Some("機房"))
                .unwrap();
            assert_eq!(
                room.visible
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>(),
                ["4", "1"]
            );
        }

        #[test]
        fn recent_menu_bounds_and_sanitizes_labels() {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                DestroyMenu, GetMenuItemCount, GetMenuItemID, GetMenuStringW, MF_BYPOSITION,
            };
            assert!(build_recent_menu(&[]).is_none());
            assert!(build_recent_menu(&vec!["x".into(); 21]).is_none());
            let menu = build_recent_menu(&["first".into(), "R&D\nnode".into()]).unwrap();
            unsafe {
                assert_eq!(GetMenuItemCount(menu), 2);
                assert_eq!(GetMenuItemID(menu, 0), 1);
                assert_eq!(GetMenuItemID(menu, 1), 2);
                let mut text = [0u16; 64];
                let len =
                    GetMenuStringW(menu, 1, text.as_mut_ptr(), text.len() as i32, MF_BYPOSITION);
                assert_eq!(String::from_utf16_lossy(&text[..len as usize]), "R&&D node");
                assert_ne!(DestroyMenu(menu), 0);
            }
        }

        #[test]
        fn context_menu_hit_test_rejects_empty_list_area() {
            use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, LB_GETITEMRECT};
            nwg::init().unwrap();
            let mut window = nwg::Window::default();
            nwg::Window::builder()
                .size((320, 240))
                .flags(nwg::WindowFlags::WINDOW)
                .build(&mut window)
                .unwrap();
            let mut list = nwg::ListBox::<String>::default();
            nwg::ListBox::builder()
                .size((200, 150))
                .parent(&window)
                .collection(vec!["first".into(), "second".into()])
                .build(&mut list)
                .unwrap();
            let mut rect: windows_sys::Win32::Foundation::RECT = unsafe { std::mem::zeroed() };
            let hwnd = list.handle.hwnd().unwrap().cast();
            assert_ne!(
                unsafe { SendMessageW(hwnd, LB_GETITEMRECT, 1, (&mut rect as *mut _) as isize) },
                -1
            );
            assert_eq!(list_item_at(&list, 2, 2), Some(0));
            assert_eq!(
                list_item_at(&list, 2, (rect.top + rect.bottom) / 2),
                Some(1)
            );
            assert_eq!(list_item_at(&list, 2, 140), None);
            assert_eq!(list_item_at(&list, -1, 2), None);
            let (x, y) = list_context_anchor(&list, 1).unwrap();
            let mut point = windows_sys::Win32::Foundation::POINT { x, y };
            assert_ne!(
                unsafe { windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point) },
                0
            );
            assert_eq!((point.x, point.y), (rect.left, rect.bottom));
            assert!(list_context_anchor(&list, 99).is_none());
        }

        #[test]
        fn tree_hit_test_distinguishes_nodes_from_empty_space() {
            use windows_sys::Win32::UI::Controls::TVM_GETITEMRECT;
            use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;
            nwg::init().unwrap();
            let mut window = nwg::Window::default();
            nwg::Window::builder()
                .size((320, 240))
                .flags(nwg::WindowFlags::WINDOW)
                .build(&mut window)
                .unwrap();
            let mut tree = nwg::TreeView::default();
            nwg::TreeView::builder()
                .size((220, 180))
                .parent(&window)
                .build(&mut tree)
                .unwrap();
            let root = tree.insert_item_with_param("group", None, nwg::TreeInsert::Last, -1);
            let leaf = tree.insert_item_with_param("host", Some(&root), nwg::TreeInsert::Last, 1);
            tree.set_expand_state(&root, nwg::ExpandState::Expand);
            let point = |item: &nwg::TreeItem| {
                let mut rect: windows_sys::Win32::Foundation::RECT = unsafe { std::mem::zeroed() };
                unsafe {
                    std::ptr::write_unaligned(
                        (&mut rect as *mut windows_sys::Win32::Foundation::RECT).cast::<isize>(),
                        item.handle as isize,
                    );
                    assert_ne!(
                        SendMessageW(
                            tree.handle.hwnd().unwrap().cast(),
                            TVM_GETITEMRECT,
                            1,
                            (&mut rect as *mut _) as isize
                        ),
                        0
                    );
                }
                ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
            };
            let (x, y) = point(&root);
            assert_eq!(
                tree.item_param(&tree_item_at(&tree, x, y).unwrap()),
                Some(-1)
            );
            let (x, y) = point(&leaf);
            assert_eq!(
                tree.item_param(&tree_item_at(&tree, x, y).unwrap()),
                Some(1)
            );
            assert!(tree_item_at(&tree, 5, 175).is_none());
            assert!(tree_item_at(&tree, -1, 5).is_none());
        }

        #[test]
        fn isolated_context_menu_open_and_cancel() {
            const CHILD: &str = "CRACAMOLE_CONTEXT_MENU_ISOLATED_TEST";
            if std::env::var_os(CHILD).is_none() {
                use std::os::windows::process::CommandExt;
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "windows_ui::tests::isolated_context_menu_open_and_cancel",
                        "--nocapture",
                    ])
                    .env(CHILD, "1")
                    .creation_flags(0x08000000)
                    .spawn()
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(12);
                loop {
                    if let Some(status) = child.try_wait().unwrap() {
                        assert!(status.success(), "isolated menu test failed: {status}");
                        return;
                    }
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("isolated menu test timed out");
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
            use windows_sys::Win32::System::StationsAndDesktops::{
                CreateDesktopW, SetThreadDesktop,
            };
            use windows_sys::Win32::System::Threading::GetCurrentThreadId;
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                DestroyMenu, GetGUIThreadInfo, GetMenuItemCount, GetMenuItemID, GetMenuStringW,
                PostMessageW, GUITHREADINFO, GUI_INMENUMODE, GUI_POPUPMENUMODE, MF_BYPOSITION,
                WM_CANCELMODE,
            };
            unsafe {
                let name: Vec<u16> = format!("cracamole-menu-{}", std::process::id())
                    .encode_utf16()
                    .chain([0])
                    .collect();
                let desktop = CreateDesktopW(
                    name.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    0x01ff,
                    std::ptr::null(),
                );
                if desktop.is_null() {
                    eprintln!(
                        "SKIP: isolated desktop unavailable: {}",
                        std::io::Error::last_os_error()
                    );
                    return;
                }
                assert_ne!(SetThreadDesktop(desktop), 0);
                let menu = build_connection_context_menu().expect("cannot build context menu");
                assert_eq!(GetMenuItemCount(menu), 5);
                for (index, expected) in ["連線", "編輯", "複製", "核對身分", "刪除"]
                    .into_iter()
                    .enumerate()
                {
                    assert_eq!(GetMenuItemID(menu, index as i32), (index + 1) as u32);
                    let mut text = [0u16; 32];
                    let len = GetMenuStringW(
                        menu,
                        index as u32,
                        text.as_mut_ptr(),
                        text.len() as i32,
                        MF_BYPOSITION,
                    );
                    assert_eq!(String::from_utf16_lossy(&text[..len as usize]), expected);
                }
                assert_ne!(DestroyMenu(menu), 0);
                nwg::init().unwrap();
                let mut window = nwg::Window::default();
                nwg::Window::builder()
                    .size((320, 240))
                    .build(&mut window)
                    .unwrap();
                let hwnd = window.handle.hwnd().unwrap() as isize;
                let thread_id = GetCurrentThreadId();
                let watcher = std::thread::spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(1);
                    while Instant::now() < deadline {
                        let mut info: GUITHREADINFO = std::mem::zeroed();
                        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
                        if GetGUIThreadInfo(thread_id, &mut info) != 0
                            && info.flags & (GUI_INMENUMODE | GUI_POPUPMENUMODE) != 0
                        {
                            PostMessageW(hwnd as _, WM_CANCELMODE, 0, 0);
                            return true;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    PostMessageW(hwnd as _, WM_CANCELMODE, 0, 0);
                    false
                });
                let result = connection_context_menu(&window, Some((10, 10)));
                if !watcher.join().unwrap() {
                    eprintln!("SKIP: inactive isolated desktop cannot enter popup menu mode");
                }
                assert_eq!(result, None);
                drop(window);
                // This isolated child exits immediately; Windows releases its desktop handle on exit.
            }
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
            let (_, input_policy) = watch::channel(Some((false, true, false)));
            let (_, metadata) = watch::channel(None);
            let active = RefCell::new(Some(Active {
                connection_id: "test".into(),
                cancel: CancellationToken::new(),
                input,
                frames,
                clipboard,
                input_policy,
                metadata,
                shown_frame: false,
                transient: false,
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
