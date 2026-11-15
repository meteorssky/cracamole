//! 原生連線編輯表單；既有機密不會載入控制項。
use cracamole_store::{Draft, PublicConnection};
use native_windows_gui as nwg;
use std::{
    cell::{Cell, RefCell},
    io::Read,
};
use zeroize::Zeroizing;

const ENCODINGS: &[&str] = &[
    "UTF-8",
    "Big5",
    "GBK",
    "GB18030",
    "Shift_JIS",
    "EUC-JP",
    "EUC-KR",
    "windows-1251",
    "windows-1252",
];

pub struct Editor {
    labels: Vec<nwg::Label>,
    name: nwg::TextInput,
    group: nwg::TextInput,
    group_label: nwg::Label,
    pub choose_group: nwg::Button,
    protocol: nwg::ComboBox<String>,
    hostname: nwg::TextInput,
    port: nwg::TextInput,
    username: nwg::TextInput,
    password: nwg::TextInput,
    key_path: nwg::TextInput,
    pub browse_key: nwg::Button,
    passphrase: nwg::TextInput,
    domain: nwg::TextInput,
    security: nwg::ComboBox<String>,
    domain_label: nwg::Label,
    security_label: nwg::Label,
    disable_copy: nwg::CheckBox,
    disable_paste: nwg::CheckBox,
    read_only: nwg::CheckBox,
    encoding: nwg::ComboBox<String>,
    encoding_label: nwg::Label,
    success: nwg::TextInput,
    failure: nwg::TextInput,
    success_label: nwg::Label,
    failure_label: nwg::Label,
    clear: nwg::CheckBox,
    pub save: nwg::Button,
    pub save_connect: nwg::Button,
    pub connect_once: nwg::Button,
    pub remove_once_trust: nwg::Button,
    pub cancel: nwg::Button,
    base: RefCell<Option<PublicConnection>>,
    visible: Cell<bool>,
    duplicate: Cell<bool>,
    applied_protocol: RefCell<String>,
    generation: Cell<u64>,
}

impl Editor {
    pub fn new(window: &nwg::Window) -> Result<Self, nwg::NwgError> {
        let mut labels = Vec::new();
        for (text, x, y) in [
            ("名稱", 24, 130),
            ("協定", 320, 130),
            ("主機", 24, 180),
            ("連接埠", 320, 180),
            ("帳號", 24, 230),
            ("密碼", 320, 230),
            ("私鑰檔案", 24, 280),
            ("私鑰密語", 24, 320),
        ] {
            let mut label = nwg::Label::default();
            nwg::Label::builder()
                .text(text)
                .position((x, y))
                .size((70, 24))
                .parent(window)
                .build(&mut label)?;
            labels.push(label);
        }
        let mut name = nwg::TextInput::default();
        let mut group = nwg::TextInput::default();
        let mut group_label = nwg::Label::default();
        let mut choose_group = nwg::Button::default();
        let mut protocol = nwg::ComboBox::default();
        let mut hostname = nwg::TextInput::default();
        let mut port = nwg::TextInput::default();
        let mut username = nwg::TextInput::default();
        let mut password = nwg::TextInput::default();
        let mut key_path = nwg::TextInput::default();
        let mut browse_key = nwg::Button::default();
        let mut passphrase = nwg::TextInput::default();
        let mut domain = nwg::TextInput::default();
        let mut security = nwg::ComboBox::default();
        let mut domain_label = nwg::Label::default();
        let mut security_label = nwg::Label::default();
        let mut disable_copy = nwg::CheckBox::default();
        let mut disable_paste = nwg::CheckBox::default();
        let mut read_only = nwg::CheckBox::default();
        let mut encoding = nwg::ComboBox::default();
        let mut encoding_label = nwg::Label::default();
        let mut success = nwg::TextInput::default();
        let mut failure = nwg::TextInput::default();
        let mut success_label = nwg::Label::default();
        let mut failure_label = nwg::Label::default();
        let mut clear = nwg::CheckBox::default();
        let mut save = nwg::Button::default();
        let mut save_connect = nwg::Button::default();
        let mut connect_once = nwg::Button::default();
        let mut remove_once_trust = nwg::Button::default();
        let mut cancel = nwg::Button::default();
        nwg::TextInput::builder()
            .position((95, 126))
            .size((200, 28))
            .parent(window)
            .build(&mut name)?;
        nwg::ComboBox::builder()
            .collection(vec![
                "ssh".into(),
                "telnet".into(),
                "rdp".into(),
                "vnc".into(),
            ])
            .selected_index(Some(0))
            .position((390, 126))
            .size((200, 150))
            .parent(window)
            .build(&mut protocol)?;
        nwg::TextInput::builder()
            .position((95, 176))
            .size((200, 28))
            .parent(window)
            .build(&mut hostname)?;
        nwg::TextInput::builder()
            .position((390, 176))
            .size((200, 28))
            .parent(window)
            .build(&mut port)?;
        nwg::TextInput::builder()
            .position((95, 226))
            .size((200, 28))
            .parent(window)
            .build(&mut username)?;
        nwg::TextInput::builder()
            .password(Some('●'))
            .position((390, 226))
            .size((200, 28))
            .parent(window)
            .build(&mut password)?;
        nwg::TextInput::builder()
            .position((95, 276))
            .size((395, 28))
            .parent(window)
            .build(&mut key_path)?;
        nwg::Button::builder()
            .text("瀏覽…")
            .position((500, 274))
            .size((90, 32))
            .parent(window)
            .build(&mut browse_key)?;
        nwg::TextInput::builder()
            .password(Some('●'))
            .position((95, 316))
            .size((200, 28))
            .parent(window)
            .build(&mut passphrase)?;
        nwg::Label::builder()
            .text("網域")
            .position((24, 280))
            .size((70, 24))
            .parent(window)
            .build(&mut domain_label)?;
        nwg::TextInput::builder()
            .position((95, 276))
            .size((200, 28))
            .parent(window)
            .build(&mut domain)?;
        nwg::Label::builder()
            .text("安全模式")
            .position((320, 280))
            .size((70, 24))
            .parent(window)
            .build(&mut security_label)?;
        nwg::ComboBox::builder()
            .collection(vec![
                "NLA（預設）".into(),
                "TLS".into(),
                "自動協商（NLA／TLS）".into(),
            ])
            .selected_index(Some(0))
            .position((390, 276))
            .size((200, 150))
            .parent(window)
            .build(&mut security)?;
        for (checkbox, text, x) in [
            (&mut disable_copy, "禁止從遠端複製", 24),
            (&mut disable_paste, "禁止貼到遠端", 250),
        ] {
            nwg::CheckBox::builder()
                .text(text)
                .position((x, 320))
                .size((210, 26))
                .parent(window)
                .build(checkbox)?;
        }
        for (label, text, y) in [
            (&mut success_label, "成功表示式", 280),
            (&mut failure_label, "失敗表示式", 320),
            (&mut encoding_label, "文字編碼", 360),
        ] {
            nwg::Label::builder()
                .text(text)
                .position((24, y))
                .size((90, 24))
                .parent(window)
                .build(label)?;
        }
        for (input, y) in [(&mut success, 276), (&mut failure, 316)] {
            nwg::TextInput::builder()
                .position((115, y))
                .size((475, 28))
                .limit(guac_protocol::MAX_ELEMENT_LEN)
                .parent(window)
                .build(input)?;
        }
        nwg::ComboBox::builder()
            .collection(ENCODINGS.iter().map(|name| (*name).to_string()).collect())
            .selected_index(Some(0))
            .position((115, 356))
            .size((200, 180))
            .parent(window)
            .build(&mut encoding)?;
        nwg::CheckBox::builder()
            .text("唯讀（禁止遠端輸入）")
            .position((340, 356))
            .size((245, 26))
            .parent(window)
            .build(&mut read_only)?;
        nwg::CheckBox::builder()
            .text("清除已保存的密碼、私鑰和密語")
            .position((315, 402))
            .size((275, 26))
            .parent(window)
            .build(&mut clear)?;
        nwg::Button::builder()
            .text("儲存")
            .position((24, 445))
            .size((100, 32))
            .parent(window)
            .build(&mut save)?;
        nwg::Button::builder()
            .text("儲存並連線")
            .position((266, 445))
            .size((120, 32))
            .parent(window)
            .build(&mut save_connect)?;
        nwg::Button::builder()
            .text("取消")
            .position((145, 445))
            .size((100, 32))
            .parent(window)
            .build(&mut cancel)?;
        nwg::Button::builder()
            .text("免保存連線")
            .position((410, 445))
            .size((120, 32))
            .parent(window)
            .build(&mut connect_once)?;
        nwg::Button::builder()
            .text("移除此主機的一次性信任")
            .position((320, 316))
            .size((270, 32))
            .parent(window)
            .build(&mut remove_once_trust)?;
        nwg::Label::builder()
            .text("群組")
            .position((24, 402))
            .size((70, 24))
            .parent(window)
            .build(&mut group_label)?;
        nwg::TextInput::builder()
            .position((95, 398))
            .size((130, 28))
            .parent(window)
            .build(&mut group)?;
        nwg::Button::builder()
            .text("選群組")
            .position((230, 398))
            .size((65, 28))
            .parent(window)
            .build(&mut choose_group)?;
        let editor = Self {
            labels,
            name,
            group,
            group_label,
            choose_group,
            protocol,
            hostname,
            port,
            username,
            password,
            key_path,
            browse_key,
            passphrase,
            domain,
            security,
            domain_label,
            security_label,
            disable_copy,
            disable_paste,
            read_only,
            encoding,
            encoding_label,
            success,
            failure,
            success_label,
            failure_label,
            clear,
            save,
            save_connect,
            connect_once,
            remove_once_trust,
            cancel,
            base: RefCell::new(None),
            visible: Cell::new(false),
            duplicate: Cell::new(false),
            applied_protocol: RefCell::new(String::new()),
            generation: Cell::new(0),
        };
        editor.show(false);
        Ok(editor)
    }

    pub fn show(&self, visible: bool) {
        self.visible.set(visible);
        for label in self.labels.iter().take(6) {
            label.set_visible(visible);
        }
        self.name.set_visible(visible);
        self.group.set_visible(visible);
        self.group_label.set_visible(visible);
        self.choose_group.set_visible(visible);
        self.protocol.set_visible(visible);
        self.hostname.set_visible(visible);
        self.port.set_visible(visible);
        self.username.set_visible(visible);
        self.password.set_visible(visible);
        self.show_protocol_fields(visible);
        self.read_only.set_visible(visible);
        self.clear.set_visible(visible);
        self.save.set_visible(visible);
        self.save_connect.set_visible(visible);
        self.connect_once
            .set_visible(visible && self.base.borrow().is_none() && !self.duplicate.get());
        self.cancel.set_visible(visible);
        if !visible {
            self.password.set_text("");
            self.key_path.set_text("");
            self.passphrase.set_text("");
            self.base.borrow_mut().take();
            self.duplicate.set(false);
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible.get()
    }

    pub fn matches_once_ssh(&self, host: &str, port: u16) -> bool {
        self.visible.get()
            && self.base.borrow().is_none()
            && !self.duplicate.get()
            && self.protocol.selection_string().as_deref() == Some("ssh")
            && self.hostname.text().trim().eq_ignore_ascii_case(host)
            && match self.port.text().as_str() {
                "" => port == 22,
                value => value.parse::<u16>().ok() == Some(port),
            }
    }

    pub fn once_ssh_endpoint(&self) -> Option<(String, u16)> {
        let host = self.hostname.text().trim().to_string();
        let port_text = self.port.text();
        let port = if port_text.is_empty() {
            22
        } else {
            port_text.parse().ok()?
        };
        if host.is_empty() || port == 0 || !self.matches_once_ssh(&host, port) {
            return None;
        }
        Some((host, port))
    }

    fn show_protocol_fields(&self, visible: bool) {
        let protocol = self.protocol.selection_string().unwrap_or_default();
        let ssh = visible && protocol == "ssh";
        self.remove_once_trust
            .set_visible(ssh && self.base.borrow().is_none() && !self.duplicate.get());
        let rdp = visible && protocol == "rdp";
        let telnet = visible && protocol == "telnet";
        let vnc = visible && protocol == "vnc";
        self.port
            .set_placeholder_text(Some(match protocol.as_str() {
                "ssh" => "22",
                "telnet" => "23",
                "rdp" => "3389",
                "vnc" => "5900",
                _ => "",
            }));
        self.labels[6].set_visible(ssh);
        self.labels[7].set_visible(ssh);
        self.key_path.set_visible(ssh);
        self.browse_key.set_visible(ssh);
        self.passphrase.set_visible(ssh);
        self.domain_label.set_visible(rdp);
        self.domain.set_visible(rdp);
        self.security_label.set_visible(rdp);
        self.security.set_visible(rdp);
        self.disable_copy.set_visible(rdp || vnc);
        self.disable_paste.set_visible(rdp || vnc);
        self.success_label.set_visible(telnet);
        self.failure_label.set_visible(telnet);
        self.success.set_visible(telnet);
        self.failure.set_visible(telnet);
        self.encoding_label.set_visible(ssh || telnet);
        self.encoding.set_visible(ssh || telnet);
    }

    pub fn protocol_changed(&self, handle: nwg::ControlHandle) {
        if handle == self.protocol.handle {
            let protocol = self.protocol.selection_string().unwrap_or_default();
            if *self.applied_protocol.borrow() != protocol {
                self.password.set_text("");
                self.key_path.set_text("");
                self.passphrase.set_text("");
                *self.applied_protocol.borrow_mut() = protocol;
            }
            self.show_protocol_fields(true);
        }
    }

    pub fn is_text_field(&self, handle: nwg::ControlHandle) -> bool {
        self.text_fields().contains(&handle)
    }

    pub fn set_key_path(&self, path: &str) {
        self.key_path.set_text(path);
    }

    pub fn text_fields(&self) -> [nwg::ControlHandle; 11] {
        [
            self.name.handle,
            self.hostname.handle,
            self.port.handle,
            self.username.handle,
            self.password.handle,
            self.key_path.handle,
            self.passphrase.handle,
            self.domain.handle,
            self.success.handle,
            self.failure.handle,
            self.group.handle,
        ]
    }

    pub fn protocol_handle(&self) -> nwg::ControlHandle {
        self.protocol.handle
    }

    pub fn begin(&self, base: Option<PublicConnection>) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.duplicate.set(false);
        let value = |key: &str| {
            base.as_ref()
                .and_then(|c| c.params.get(key))
                .map(String::as_str)
                .unwrap_or("")
        };
        let (group, name) = split_group_name(base.as_ref().map(|c| c.name.as_str()).unwrap_or(""));
        self.name.set_text(name);
        self.group.set_text(group);
        self.hostname.set_text(value("hostname"));
        self.port.set_text(value("port"));
        self.username.set_text(value("username"));
        self.password.set_text("");
        self.key_path.set_text("");
        self.passphrase.set_text("");
        self.domain.set_text(value("domain"));
        self.success.set_text(value("login-success-regex"));
        self.failure.set_text(value("login-failure-regex"));
        let encoding = guacd::TextEncoding::parse(Some(value("character-encoding")))
            .ok()
            .and_then(|encoding| {
                ENCODINGS
                    .iter()
                    .position(|name| name.eq_ignore_ascii_case(encoding.name()))
            });
        self.encoding.set_selection(encoding);
        let security = match value("security") {
            "tls" => 1,
            "any" => 2,
            _ => 0,
        };
        self.security.set_selection(Some(security));
        self.read_only
            .set_check_state(if value("read-only") == "true" {
                nwg::CheckBoxState::Checked
            } else {
                nwg::CheckBoxState::Unchecked
            });
        for (checkbox, key) in [
            (&self.disable_copy, "disable-copy"),
            (&self.disable_paste, "disable-paste"),
        ] {
            checkbox.set_check_state(if value(key) == "true" {
                nwg::CheckBoxState::Checked
            } else {
                nwg::CheckBoxState::Unchecked
            });
        }
        self.clear.set_check_state(nwg::CheckBoxState::Unchecked);
        let protocol = base.as_ref().map(|c| c.protocol.as_str()).unwrap_or("ssh");
        self.protocol.set_selection_string(protocol);
        *self.applied_protocol.borrow_mut() = protocol.to_string();
        *self.base.borrow_mut() = base;
        self.show(true);
        self.name.set_focus();
    }

    /// 從公開設定建立新連線，保留可檢查的設定，但不沿用原 ID 或機密。
    pub fn begin_import(&self, mut base: PublicConnection) {
        base.params
            .retain(|key, _| !["password", "private-key", "passphrase"].contains(&key.as_str()));
        base.configured_secrets.clear();
        self.begin(Some(base));
        self.duplicate.set(true);
    }

    pub fn begin_duplicate(&self, base: PublicConnection) {
        let name = duplicate_name(&base.name);
        self.begin_import(base);
        self.name.set_text(split_group_name(&name).1);
        self.hostname.set_focus();
        self.hostname.set_selection(0..u32::MAX);
    }

    pub fn begin_in_group(&self, group: &str) {
        self.begin(None);
        self.group.set_text(group);
        self.name.set_focus();
    }

    pub fn group_text(&self) -> String {
        self.group.text()
    }

    pub fn set_group(&self, value: &str) {
        self.group.set_text(value);
        self.group.set_focus();
    }

    pub fn generation(&self) -> u64 {
        self.generation.get()
    }

    pub fn is_quick_endpoint(address: &str) -> Result<bool, &'static str> {
        Ok(parse_endpoint(address)?.is_some())
    }

    pub fn existing_quick(
        address: &str,
        entries: &[PublicConnection],
    ) -> Result<Option<String>, &'static str> {
        let target = parse_endpoint(address)?.ok_or("請輸入 user@host 或 host:port。")?;
        let protocol = target.protocol.as_deref().unwrap_or("ssh");
        let default_port = match protocol {
            "ssh" => 22,
            "telnet" => 23,
            "rdp" => 3389,
            "vnc" => 5900,
            _ => return Err("不支援的協定。"),
        };
        let target_port = target
            .port
            .as_deref()
            .map_or(Some(default_port), |port| port.parse::<u16>().ok())
            .ok_or("連接埠無效。")?;
        let mut matches = entries.iter().filter(|entry| {
            entry.protocol == protocol
                && entry
                    .params
                    .get("hostname")
                    .is_some_and(|host| host.eq_ignore_ascii_case(&target.host))
                && entry.params.get("username").map_or("", String::as_str)
                    == target.user.as_deref().unwrap_or("")
                && (protocol != "rdp"
                    || entry.params.get("domain").map_or("", String::as_str)
                        == target.domain.as_deref().unwrap_or(""))
                && entry
                    .params
                    .get("port")
                    .filter(|port| !port.is_empty())
                    .map_or(Some(default_port), |port| port.parse::<u16>().ok())
                    == Some(target_port)
        });
        let first = matches.next().map(|entry| entry.id.clone());
        if matches.next().is_some() {
            return Err("有多筆相同目的地，請從清單選擇。");
        }
        Ok(first)
    }

    pub fn begin_quick(&self, address: &str) -> Result<(), &'static str> {
        let endpoint = parse_endpoint(address)?.ok_or("請輸入 user@host 或 host:port。")?;
        self.begin(None);
        if let Some(protocol) = endpoint.protocol.as_deref() {
            self.protocol.set_selection_string(protocol);
            self.protocol_changed(self.protocol.handle);
        }
        self.domain
            .set_text(endpoint.domain.as_deref().unwrap_or(""));
        self.hostname.set_text(&endpoint.host);
        if let Some(port) = endpoint.port {
            self.port.set_text(&port);
        }
        if let Some(user) = endpoint.user {
            self.username.set_text(&user);
        }
        self.hostname.set_focus();
        Ok(())
    }

    pub fn normalize_endpoint(&self) -> Result<(), &'static str> {
        let value = self.hostname.text();
        let endpoint = if self.protocol.selection_string().as_deref() == Some("rdp")
            && !value.contains("://")
            && value.contains('\\')
        {
            let mut endpoint = parse_endpoint(&format!("rdp://{value}"))?;
            if let Some(endpoint) = endpoint.as_mut() {
                // 此處借用 URI 解析，但使用者並未切換協定，也未指定其他欄位。
                endpoint.protocol = None;
            }
            endpoint
        } else {
            parse_endpoint(&value)?
        };
        if let Some(endpoint) = endpoint {
            if let Some(protocol) = endpoint.protocol.as_deref() {
                if self.protocol.selection_string().as_deref() != Some(protocol) {
                    self.protocol.set_selection_string(protocol);
                    self.protocol_changed(self.protocol.handle);
                }
            }
            self.hostname.set_text(&endpoint.host);
            if endpoint.protocol.is_some() {
                // 明確 URI／命令代表完整目的地；未填埠或帳號時不得沿用舊值。
                self.port.set_text(endpoint.port.as_deref().unwrap_or(""));
                self.username
                    .set_text(endpoint.user.as_deref().unwrap_or(""));
                self.domain
                    .set_text(endpoint.domain.as_deref().unwrap_or(""));
            } else {
                if let Some(value) = endpoint.port {
                    self.port.set_text(&value);
                }
                if let Some(value) = endpoint.user {
                    self.username.set_text(&value);
                    if self.protocol.selection_string().as_deref() == Some("rdp") {
                        self.domain
                            .set_text(endpoint.domain.as_deref().unwrap_or(""));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn normalize_blurred_endpoint(
        &self,
        previous: &(String, String, String, String, String),
    ) -> Result<(), &'static str> {
        if !self.hostname.focus()
            && self.hostname.text() == previous.0
            && self.port.text() == previous.1
            && self.username.text() == previous.2
            && self.protocol.selection_string().as_deref() == Some(previous.3.as_str())
            && self.domain.text() == previous.4
        {
            self.normalize_endpoint()?;
        }
        Ok(())
    }

    pub fn normalize_pasted_endpoint(
        &self,
        previous: &(String, String, String, String, String),
    ) -> Result<(), &'static str> {
        if self.hostname.text() == previous.0
            && self.port.text() == previous.1
            && self.username.text() == previous.2
            && self.protocol.selection_string().as_deref() == Some(previous.3.as_str())
            && self.domain.text() == previous.4
        {
            self.normalize_endpoint()?;
        }
        Ok(())
    }

    pub fn draft(&self) -> Result<(Option<String>, Draft), &'static str> {
        self.normalize_endpoint()?;
        let protocol = self.protocol.selection_string().unwrap_or_default();
        let hostname = self.hostname.text();
        let port = self.port.text();
        let username = self.username.text();
        let base = self.base.borrow();
        let mut params = base
            .as_ref()
            .filter(|old| old.protocol == protocol)
            .map(|old| old.params.clone())
            .unwrap_or_default();
        if self.duplicate.get() {
            for key in ["password", "private-key", "passphrase"] {
                params.remove(key);
            }
        }
        if base.as_ref().is_some_and(|old| {
            old.protocol != protocol
                || old.params.get("hostname").map(String::as_str).unwrap_or("") != hostname
                || old.params.get("port").map(String::as_str).unwrap_or("") != port
        }) {
            params.remove("host-key");
            params.remove("server-cert-sha256");
        }
        let name = if self.name.text().trim().is_empty() {
            let generated = if username.is_empty() {
                hostname.clone()
            } else {
                format!("{username}@{hostname}")
            };
            self.name.set_text(&generated);
            generated
        } else {
            self.name.text().trim().to_string()
        };
        let group = self.group.text();
        let group = group.trim();
        if group.contains('/') || group.chars().any(char::is_control) {
            return Err("群組名稱不可包含斜線或控制字元。");
        }
        let name = if group.is_empty() {
            name
        } else {
            format!("{group} / {name}")
        };
        if name.chars().count() > 120 {
            return Err("連線名稱加上群組不可超過 120 個字元。");
        }
        params.insert("hostname".into(), hostname);
        params.insert("port".into(), port);
        params.insert("username".into(), username);
        params.insert(
            "read-only".into(),
            (self.read_only.check_state() == nwg::CheckBoxState::Checked).to_string(),
        );
        if protocol == "ssh" || protocol == "telnet" {
            let encoding = self
                .encoding
                .selection()
                .and_then(|index| ENCODINGS.get(index))
                .ok_or("請選擇文字編碼。")?;
            params.insert("character-encoding".into(), (*encoding).into());
        }
        if protocol == "telnet" {
            let success = self.success.text();
            let failure = self.failure.text();
            if success.is_empty() != failure.is_empty() {
                return Err("Telnet 登入成功及失敗的正規表示式須同時填寫。");
            }
            if success.chars().count() > guac_protocol::MAX_ELEMENT_LEN
                || failure.chars().count() > guac_protocol::MAX_ELEMENT_LEN
            {
                return Err("Telnet 登入正規表示式過長。");
            }
            if !success.is_empty()
                && (proto_telnet::login::compile(&success).is_err()
                    || proto_telnet::login::compile(&failure).is_err())
            {
                return Err("Telnet 登入正規表示式無效。");
            }
            params.insert("login-success-regex".into(), success);
            params.insert("login-failure-regex".into(), failure);
        }
        if protocol == "rdp" {
            params.insert("domain".into(), self.domain.text());
            let security = match self.security.selection() {
                Some(0) => "nla",
                Some(1) => "tls",
                Some(2) => "any",
                _ => return Err("請選擇 RDP 安全模式。"),
            };
            params.insert("security".into(), security.into());
        }
        if protocol == "rdp" || protocol == "vnc" {
            for (checkbox, key) in [
                (&self.disable_copy, "disable-copy"),
                (&self.disable_paste, "disable-paste"),
            ] {
                params.insert(
                    key.into(),
                    (checkbox.check_state() == nwg::CheckBoxState::Checked).to_string(),
                );
            }
        }
        if self.clear.check_state() == nwg::CheckBoxState::Checked {
            if let Some(plugin) = guacd::Registry::builtin().get(&protocol) {
                for key in ["password", "private-key", "passphrase"] {
                    if plugin.arg_names().contains(&key) {
                        params.insert(key.into(), String::new());
                    }
                }
            }
        } else {
            let password = Zeroizing::new(self.password.text());
            let key_path = self.key_path.text();
            let passphrase = Zeroizing::new(self.passphrase.text());
            if protocol != "ssh" && (!key_path.is_empty() || !passphrase.is_empty()) {
                return Err("私鑰檔案與密語只適用 SSH 連線。");
            }
            if !key_path.is_empty() {
                let file = std::fs::File::open(&key_path).map_err(|_| "無法開啟私鑰檔案。")?;
                if !file
                    .metadata()
                    .map_err(|_| "無法讀取私鑰檔案。")
                    .is_ok_and(|metadata| metadata.is_file())
                {
                    return Err("私鑰路徑必須是一般檔案。");
                }
                let mut bytes = Zeroizing::new(Vec::new());
                file.take(8193)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "無法讀取私鑰檔案。")?;
                if bytes.is_empty() || bytes.len() > 8192 || bytes.contains(&0) {
                    return Err("私鑰檔案必須是 1 至 8192 位元組的文字。");
                }
                let key = std::str::from_utf8(&bytes).map_err(|_| "私鑰檔案必須是 UTF-8 文字。")?;
                params.insert("private-key".into(), key.into());
            }
            if !password.is_empty() {
                params.insert("password".into(), password.to_string());
            }
            if !passphrase.is_empty() {
                params.insert("passphrase".into(), passphrase.to_string());
            }
        }
        Ok((
            if self.duplicate.get() {
                None
            } else {
                base.as_ref().map(|old| old.id.clone())
            },
            Draft {
                name,
                protocol,
                params,
            },
        ))
    }
}

fn split_group_name(name: &str) -> (&str, &str) {
    if let Some((group, member)) = name.split_once('/') {
        let (group, member) = (group.trim(), member.trim());
        if !group.is_empty() && !member.is_empty() {
            return (group, member);
        }
    }
    ("", name)
}

struct ParsedEndpoint {
    user: Option<String>,
    domain: Option<String>,
    host: String,
    port: Option<String>,
    protocol: Option<String>,
}

fn parse_endpoint(value: &str) -> Result<Option<ParsedEndpoint>, &'static str> {
    let value = value.trim();
    let invalid = "主機格式無效，請使用 user@host:port 或 [IPv6]:port。";
    if value
        .split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("ssh"))
        && value.chars().any(char::is_whitespace)
    {
        let parts: Vec<_> = value.split_whitespace().collect();
        if parts.len() != 4
            || parts[1] != "-p"
            || value.chars().any(char::is_control)
            || parts[3].starts_with('-')
            || parts[3].contains(['\'', '"', '`', '$', ';', '&', '|', '\\'])
        {
            return Err(invalid);
        }
        let mut endpoint = parse_endpoint(&format!("{}:{}", parts[3], parts[2]))?.ok_or(invalid)?;
        endpoint.protocol = Some("ssh".into());
        return Ok(Some(endpoint));
    }
    if value.contains("://") {
        // Only encode the user portion; the shared parser still validates the host.
        let normalized = if let Some(authority) = value.strip_prefix("rdp://") {
            if let Some((user, host)) = authority.split_once('@') {
                format!("rdp://{}@{host}", user.replace('\\', "%5C"))
            } else {
                value.to_string()
            }
        } else {
            value.to_string()
        };
        let mut entries =
            cracamole_store::portable::import_uris(&normalized).map_err(|_| invalid)?;
        if entries.len() != 1 {
            return Err(invalid);
        }
        let entry = entries.remove(0);
        let mut user = entry.params.get("username").cloned();
        let mut domain = entry.params.get("domain").cloned();
        if entry.protocol == "rdp" {
            if let Some((left, right)) = user.as_deref().and_then(|value| value.split_once('\\')) {
                if left.is_empty() || right.is_empty() || right.contains('\\') {
                    return Err(invalid);
                }
                domain = Some(left.to_string());
                user = Some(right.to_string());
            }
        }
        return Ok(Some(ParsedEndpoint {
            user,
            domain,
            host: entry.params.get("hostname").cloned().ok_or(invalid)?,
            port: entry.params.get("port").cloned(),
            protocol: Some(entry.protocol),
        }));
    }
    if value.parse::<std::net::Ipv6Addr>().is_ok() {
        return Ok(None);
    }
    if !value.contains(['@', ':', '[', ']']) {
        return Ok(None);
    }
    if value.contains(['/', '?', '#', '\\']) || value.chars().any(char::is_whitespace) {
        return Err(invalid);
    }
    let (user, address) = if let Some((user, address)) = value.split_once('@') {
        if user.is_empty() || user.contains([':', '@']) || address.contains('@') {
            return Err(invalid);
        }
        (Some(user.to_string()), address)
    } else {
        (None, value)
    };
    let bracketed = address.starts_with('[');
    let (host, port) = if let Some(rest) = address.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']').ok_or(invalid)?;
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(invalid);
        }
        let port = if suffix.is_empty() {
            None
        } else {
            Some(suffix.strip_prefix(':').ok_or(invalid)?)
        };
        (host, port)
    } else if let Some((host, port)) = address.split_once(':') {
        (host, Some(port))
    } else {
        (address, None)
    };
    if host.is_empty()
        || (!bracketed && host.contains(':'))
        || host.contains(['[', ']', '@'])
        || host.len() > 255
    {
        return Err(invalid);
    }
    if let Some(port) = port {
        if port.is_empty()
            || !port.bytes().all(|byte| byte.is_ascii_digit())
            || port
                .parse::<u16>()
                .ok()
                .filter(|number| *number > 0)
                .is_none()
        {
            return Err(invalid);
        }
    }
    Ok(Some(ParsedEndpoint {
        user,
        domain: None,
        host: host.to_string(),
        port: port.map(str::to_string),
        protocol: None,
    }))
}

fn duplicate_name(name: &str) -> String {
    let start = name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (prefix, digits) = name.split_at(start);
    let copy = if !digits.is_empty() {
        digits
            .parse::<u128>()
            .ok()
            .and_then(|number| number.checked_add(1))
            .map(|number| format!("{prefix}{number:0width$}", width = digits.len()))
    } else {
        None
    };
    copy.unwrap_or_else(|| format!("{name} - 複製"))
        .chars()
        .take(120)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{duplicate_name, parse_endpoint, Editor};
    use cracamole_store::PublicConnection;
    use std::collections::BTreeMap;

    #[test]
    fn imported_settings_create_new_connection_and_clear_changed_identity() {
        native_windows_gui::init().unwrap();
        let mut window = native_windows_gui::Window::default();
        native_windows_gui::Window::builder()
            .size((640, 600))
            .flags(native_windows_gui::WindowFlags::WINDOW)
            .build(&mut window)
            .unwrap();
        let editor = Editor::new(&window).unwrap();
        let source = cracamole_store::PublicConnection {
            id: "original".into(),
            name: "匯入主機".into(),
            protocol: "ssh".into(),
            params: std::collections::BTreeMap::from([
                ("hostname".into(), "example.test".into()),
                ("host-key".into(), "ssh-ed25519 public-key".into()),
                ("password".into(), "must-not-copy".into()),
            ]),
            configured_secrets: vec!["password".into()],
        };
        editor.begin_import(source);
        let (id, draft) = editor.draft().unwrap();
        assert!(id.is_none());
        assert_eq!(draft.name, "匯入主機");
        assert_eq!(
            draft.params.get("host-key").unwrap(),
            "ssh-ed25519 public-key"
        );
        assert!(!draft.params.contains_key("password"));
        editor.hostname.set_text("other.example");
        let (_, changed) = editor.draft().unwrap();
        assert!(!changed.params.contains_key("host-key"));
    }

    #[test]
    fn web_exchange_data_opens_as_a_new_desktop_draft() {
        native_windows_gui::init().unwrap();
        let mut window = native_windows_gui::Window::default();
        native_windows_gui::Window::builder()
            .size((640, 600))
            .flags(native_windows_gui::WindowFlags::WINDOW)
            .build(&mut window)
            .unwrap();
        let editor = Editor::new(&window).unwrap();
        let content = r#"{"format":"cracamole-connections","version":1,"connections":[{"name":"共用主機","protocol":"telnet","params":{"hostname":"example.test","port":"23","username":"alice","read-only":"true","character-encoding":"Big5"}}]}"#;
        let mut entries = cracamole_store::portable::import(content).unwrap();
        let entry = entries.remove(0);
        editor.begin_import(cracamole_store::PublicConnection {
            id: String::new(),
            name: entry.name,
            protocol: entry.protocol,
            params: entry.params,
            configured_secrets: Vec::new(),
        });
        let (id, draft) = editor.draft().unwrap();
        assert!(id.is_none());
        assert_eq!(draft.name, "共用主機");
        assert_eq!(draft.protocol, "telnet");
        assert_eq!(draft.params["hostname"], "example.test");
        assert_eq!(draft.params["username"], "alice");
        assert_eq!(draft.params["read-only"], "true");
        assert_eq!(draft.params["character-encoding"], "Big5");
        cracamole_store::validate(&draft).unwrap();
    }

    #[test]
    fn duplicate_names_keep_numeric_padding() {
        assert_eq!(duplicate_name("prod-web-01"), "prod-web-02");
        assert_eq!(duplicate_name("主機-9"), "主機-10");
        assert_eq!(duplicate_name("主機"), "主機 - 複製");
        assert!(duplicate_name(&"主".repeat(120)).chars().count() <= 120);
    }

    #[test]
    fn endpoint_parsing_rejects_ambiguous_or_secret_inputs() {
        let endpoint = parse_endpoint("alice@[2001:db8::1]:3389").unwrap().unwrap();
        assert_eq!(endpoint.user.as_deref(), Some("alice"));
        assert_eq!(endpoint.host, "2001:db8::1");
        assert_eq!(endpoint.port.as_deref(), Some("3389"));
        let endpoint = parse_endpoint("alice@example.test:2222").unwrap().unwrap();
        assert_eq!(endpoint.user.as_deref(), Some("alice"));
        assert_eq!(endpoint.host, "example.test");
        assert_eq!(endpoint.port.as_deref(), Some("2222"));
        let endpoint = parse_endpoint("alice@example.test").unwrap().unwrap();
        assert_eq!(endpoint.user.as_deref(), Some("alice"));
        assert!(endpoint.port.is_none());
        assert!(parse_endpoint("2001:db8::1").unwrap().is_none());
        assert!(parse_endpoint("user:secret@host:22").is_err());
        assert!(parse_endpoint("host:65536").is_err());
        assert!(parse_endpoint("host:0").is_err());
        assert!(parse_endpoint("[2001:db8::1]:bad").is_err());
        assert_eq!(
            parse_endpoint("ssh://host")
                .unwrap()
                .unwrap()
                .protocol
                .as_deref(),
            Some("ssh")
        );
        let uri = parse_endpoint("rdp://alice@[2001:db8::1]:3390")
            .unwrap()
            .unwrap();
        assert_eq!(uri.protocol.as_deref(), Some("rdp"));
        assert_eq!(uri.user.as_deref(), Some("alice"));
        assert_eq!(uri.host, "2001:db8::1");
        assert_eq!(uri.port.as_deref(), Some("3390"));
        assert!(parse_endpoint("rdp://alice:secret@host").is_err());
        assert!(parse_endpoint("rdp://host/path").is_err());
        assert!(parse_endpoint("rdp://host\nssh://other").is_err());
        let command = parse_endpoint("ssh -p 2222 alice@[2001:db8::1]")
            .unwrap()
            .unwrap();
        assert_eq!(command.protocol.as_deref(), Some("ssh"));
        assert_eq!(command.host, "2001:db8::1");
        assert_eq!(command.port.as_deref(), Some("2222"));
        for invalid in [
            "ssh -p 0 alice@host",
            "ssh -p 22 user:secret@host",
            "ssh -p 22 host:23",
            "ssh -p 22 host;cmd",
            "ssh -p 22 host command",
            "ssh -o option host",
        ] {
            assert!(parse_endpoint(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn pasted_endpoint_only_updates_unchanged_fields() {
        native_windows_gui::init().unwrap();
        let mut window = native_windows_gui::Window::default();
        native_windows_gui::Window::builder()
            .size((640, 600))
            .flags(native_windows_gui::WindowFlags::WINDOW)
            .build(&mut window)
            .unwrap();
        let editor = Editor::new(&window).unwrap();
        editor.begin(None);
        editor.hostname.set_text("alice@[2001:db8::1]:2222");
        editor.port.set_text("23");
        editor.username.set_text("old");
        let snapshot = (
            editor.hostname.text(),
            editor.port.text(),
            editor.username.text(),
            editor.protocol.selection_string().unwrap(),
            editor.domain.text(),
        );
        editor.normalize_pasted_endpoint(&snapshot).unwrap();
        assert_eq!(editor.hostname.text(), "2001:db8::1");
        assert_eq!(editor.port.text(), "2222");
        assert_eq!(editor.username.text(), "alice");

        editor.hostname.set_text("bob@host.example:2222");
        let stale = (
            editor.hostname.text(),
            editor.port.text(),
            editor.username.text(),
            editor.protocol.selection_string().unwrap(),
            editor.domain.text(),
        );
        editor.port.set_text("3333");
        editor.normalize_pasted_endpoint(&stale).unwrap();
        assert_eq!(editor.hostname.text(), "bob@host.example:2222");
        assert_eq!(editor.port.text(), "3333");
        assert_eq!(editor.username.text(), "alice");

        editor.hostname.set_text("bob:secret@host:22");
        let invalid = (
            editor.hostname.text(),
            editor.port.text(),
            editor.username.text(),
            editor.protocol.selection_string().unwrap(),
            editor.domain.text(),
        );
        assert!(editor.normalize_pasted_endpoint(&invalid).is_err());
        assert_eq!(editor.hostname.text(), invalid.0);
        assert_eq!(editor.port.text(), invalid.1);
        assert_eq!(editor.username.text(), invalid.2);

        editor.protocol.set_selection(Some(0));
        editor.password.set_text("old-ssh-password");
        editor.username.set_text("old-user");
        editor.port.set_text("2200");
        editor.hostname.set_text("rdp://host.example");
        editor.normalize_endpoint().unwrap();
        assert_eq!(editor.protocol.selection_string().as_deref(), Some("rdp"));
        assert_eq!(editor.hostname.text(), "host.example");
        assert_eq!(editor.port.text(), "");
        assert_eq!(editor.username.text(), "");
        assert_eq!(editor.password.text(), "");
        editor.hostname.set_text(r"CORP\alice@desktop.example:3390");
        editor.normalize_endpoint().unwrap();
        assert_eq!(editor.protocol.selection_string().as_deref(), Some("rdp"));
        assert_eq!(editor.hostname.text(), "desktop.example");
        assert_eq!(editor.username.text(), "alice");
        assert_eq!(editor.domain.text(), "CORP");
        assert_eq!(editor.port.text(), "3390");
        editor.hostname.set_text(r"OTHER\bob@desktop.example");
        let pending = (
            editor.hostname.text(),
            editor.port.text(),
            editor.username.text(),
            editor.protocol.selection_string().unwrap(),
            editor.domain.text(),
        );
        editor.domain.set_text("手動修改");
        editor.normalize_pasted_endpoint(&pending).unwrap();
        assert_eq!(editor.hostname.text(), pending.0);
        assert_eq!(editor.domain.text(), "手動修改");
        editor.protocol.set_selection(Some(2));
        editor.password.set_text("old-rdp-password");
        editor.hostname.set_text("ssh -p 2200 bob@host.example");
        let command = (
            editor.hostname.text(),
            editor.port.text(),
            editor.username.text(),
            editor.protocol.selection_string().unwrap(),
            editor.domain.text(),
        );
        editor.protocol.set_selection(Some(3));
        editor.normalize_pasted_endpoint(&command).unwrap();
        assert_eq!(editor.protocol.selection_string().as_deref(), Some("vnc"));
        assert_eq!(editor.hostname.text(), command.0);
        editor.protocol.set_selection(Some(2));
        editor.normalize_pasted_endpoint(&command).unwrap();
        assert_eq!(editor.protocol.selection_string().as_deref(), Some("ssh"));
        assert_eq!(editor.hostname.text(), "host.example");
        assert_eq!(editor.port.text(), "2200");
        assert_eq!(editor.username.text(), "bob");
        assert_eq!(editor.password.text(), "");
    }

    #[test]
    fn changing_protocol_clears_unsaved_secrets_once() {
        native_windows_gui::init().unwrap();
        let mut window = native_windows_gui::Window::default();
        native_windows_gui::Window::builder()
            .size((640, 600))
            .flags(native_windows_gui::WindowFlags::WINDOW)
            .build(&mut window)
            .unwrap();
        let editor = Editor::new(&window).unwrap();
        editor.begin(None);
        editor.password.set_text("password");
        editor.key_path.set_text("key-file");
        editor.passphrase.set_text("passphrase");
        editor.protocol_changed(editor.protocol.handle);
        assert_eq!(editor.password.text(), "password");
        editor.protocol.set_selection_string("rdp");
        editor.protocol_changed(editor.protocol.handle);
        assert!(editor.password.text().is_empty());
        assert!(editor.key_path.text().is_empty());
        assert!(editor.passphrase.text().is_empty());
        editor.password.set_text("new-password");
        editor.protocol_changed(editor.protocol.handle);
        assert_eq!(editor.password.text(), "new-password");
    }

    #[test]
    fn group_field_preserves_compatible_connection_names() {
        native_windows_gui::init().unwrap();
        let mut window = native_windows_gui::Window::default();
        native_windows_gui::Window::builder()
            .size((640, 600))
            .flags(native_windows_gui::WindowFlags::WINDOW)
            .build(&mut window)
            .unwrap();
        let editor = Editor::new(&window).unwrap();
        editor.begin_in_group("機房");
        assert_eq!(editor.group.text(), "機房");
        assert!(editor.name.text().is_empty());
        let connection = || PublicConnection {
            id: "one".into(),
            name: "舊群組 / 主機-01".into(),
            protocol: "ssh".into(),
            params: BTreeMap::from([("hostname".into(), "example.test".into())]),
            configured_secrets: Vec::new(),
        };
        editor.begin(Some(connection()));
        assert_eq!(editor.group.text(), "舊群組");
        assert_eq!(editor.name.text(), "主機-01");
        editor.group.set_text("新群組");
        assert_eq!(editor.draft().unwrap().1.name, "新群組 / 主機-01");
        editor.begin_duplicate(connection());
        assert_eq!(editor.group.text(), "舊群組");
        assert_eq!(editor.name.text(), "主機-02");
        editor.group.set_text("無效/群組");
        assert!(editor.draft().is_err());
    }

    #[test]
    fn quick_existing_requires_a_unique_matching_ssh_endpoint() {
        let connection = PublicConnection {
            id: "one".into(),
            name: "host".into(),
            protocol: "ssh".into(),
            params: BTreeMap::from([
                ("hostname".into(), "EXAMPLE.test".into()),
                ("username".into(), "alice".into()),
            ]),
            configured_secrets: Vec::new(),
        };
        assert_eq!(
            Editor::existing_quick("alice@example.test:22", std::slice::from_ref(&connection))
                .unwrap()
                .as_deref(),
            Some("one")
        );
        assert!(
            Editor::existing_quick("bob@example.test:22", std::slice::from_ref(&connection))
                .unwrap()
                .is_none()
        );
        assert!(
            Editor::existing_quick("alice@example.test:23", std::slice::from_ref(&connection))
                .unwrap()
                .is_none()
        );
        let rdp = PublicConnection {
            id: "rdp".into(),
            name: "desktop".into(),
            protocol: "rdp".into(),
            params: BTreeMap::from([("hostname".into(), "example.test".into())]),
            configured_secrets: Vec::new(),
        };
        assert_eq!(
            Editor::existing_quick("rdp://example.test:3389", &[rdp])
                .unwrap()
                .as_deref(),
            Some("rdp")
        );
        let duplicate = PublicConnection {
            id: "two".into(),
            name: connection.name.clone(),
            protocol: connection.protocol.clone(),
            params: connection.params.clone(),
            configured_secrets: Vec::new(),
        };
        assert!(Editor::existing_quick("alice@example.test:22", &[connection, duplicate]).is_err());
    }

    #[test]
    fn rdp_quick_endpoint_distinguishes_domains() {
        let endpoint = parse_endpoint(r"rdp://CORP\alice@host.test:3390")
            .unwrap()
            .unwrap();
        assert_eq!(endpoint.domain.as_deref(), Some("CORP"));
        assert_eq!(endpoint.user.as_deref(), Some("alice"));
        assert!(parse_endpoint(r"rdp://CORP\@host.test").is_err());
        assert!(parse_endpoint(r"rdp://CORP\alice\extra@host.test").is_err());
        let entry = PublicConnection {
            id: "one".into(),
            name: "desktop".into(),
            protocol: "rdp".into(),
            params: BTreeMap::from([
                ("hostname".into(), "host.test".into()),
                ("username".into(), "alice".into()),
                ("domain".into(), "CORP".into()),
            ]),
            configured_secrets: Vec::new(),
        };
        assert_eq!(
            Editor::existing_quick(r"rdp://CORP\alice@host.test", std::slice::from_ref(&entry))
                .unwrap()
                .as_deref(),
            Some("one")
        );
        assert!(
            Editor::existing_quick(r"rdp://OTHER\alice@host.test", &[entry])
                .unwrap()
                .is_none()
        );
    }
}
