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
    pub cancel: nwg::Button,
    base: RefCell<Option<PublicConnection>>,
    visible: Cell<bool>,
    duplicate: Cell<bool>,
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
            .position((24, 402))
            .size((400, 26))
            .parent(window)
            .build(&mut clear)?;
        nwg::Button::builder()
            .text("儲存")
            .position((24, 445))
            .size((100, 32))
            .parent(window)
            .build(&mut save)?;
        nwg::Button::builder()
            .text("取消")
            .position((145, 445))
            .size((100, 32))
            .parent(window)
            .build(&mut cancel)?;
        let editor = Self {
            labels,
            name,
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
            cancel,
            base: RefCell::new(None),
            visible: Cell::new(false),
            duplicate: Cell::new(false),
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
        self.protocol.set_visible(visible);
        self.hostname.set_visible(visible);
        self.port.set_visible(visible);
        self.username.set_visible(visible);
        self.password.set_visible(visible);
        self.show_protocol_fields(visible);
        self.read_only.set_visible(visible);
        self.clear.set_visible(visible);
        self.save.set_visible(visible);
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

    fn show_protocol_fields(&self, visible: bool) {
        let protocol = self.protocol.selection_string().unwrap_or_default();
        let ssh = visible && protocol == "ssh";
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
            self.show_protocol_fields(true);
            if self.protocol.selection_string().as_deref() != Some("ssh") {
                self.key_path.set_text("");
                self.passphrase.set_text("");
            }
        }
    }

    pub fn is_text_field(&self, handle: nwg::ControlHandle) -> bool {
        self.text_fields().contains(&handle)
    }

    pub fn set_key_path(&self, path: &str) {
        self.key_path.set_text(path);
    }

    pub fn text_fields(&self) -> [nwg::ControlHandle; 10] {
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
        ]
    }

    pub fn begin(&self, base: Option<PublicConnection>) {
        self.duplicate.set(false);
        let value = |key: &str| {
            base.as_ref()
                .and_then(|c| c.params.get(key))
                .map(String::as_str)
                .unwrap_or("")
        };
        self.name
            .set_text(base.as_ref().map(|c| c.name.as_str()).unwrap_or(""));
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
        *self.base.borrow_mut() = base;
        self.show(true);
        self.name.set_focus();
    }

    pub fn begin_duplicate(&self, base: PublicConnection) {
        let name = duplicate_name(&base.name);
        self.begin(Some(base));
        self.duplicate.set(true);
        self.name.set_text(&name);
        self.hostname.set_focus();
        self.hostname.set_selection(0..u32::MAX);
    }

    pub fn draft(&self) -> Result<(Option<String>, Draft), &'static str> {
        let protocol = self.protocol.selection_string().unwrap_or_default();
        let hostname = self.hostname.text();
        let port = self.port.text();
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
        params.insert("hostname".into(), hostname);
        params.insert("port".into(), port);
        params.insert("username".into(), self.username.text());
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
            if !password.is_empty() {
                params.insert("password".into(), password.to_string());
            }
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
                name: self.name.text(),
                protocol,
                params,
            },
        ))
    }
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
    use super::duplicate_name;

    #[test]
    fn duplicate_names_keep_numeric_padding() {
        assert_eq!(duplicate_name("prod-web-01"), "prod-web-02");
        assert_eq!(duplicate_name("主機-9"), "主機-10");
        assert_eq!(duplicate_name("主機"), "主機 - 複製");
        assert!(duplicate_name(&"主".repeat(120)).chars().count() <= 120);
    }
}
