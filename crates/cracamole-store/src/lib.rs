//! 共用連線設定驗證與保存。
#![forbid(unsafe_code)]

pub mod credentials;

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::PathBuf};
use tokio::sync::Mutex;
use uuid::Uuid;

const SECRETS: &[&str] = &["password", "private-key", "passphrase"];

#[derive(Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub params: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub struct Draft {
    pub name: String,
    pub protocol: String,
    pub params: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub struct PublicConnection {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub params: BTreeMap<String, String>,
    pub configured_secrets: Vec<String>,
}

impl Connection {
    pub fn public(&self) -> PublicConnection {
        PublicConnection {
            id: self.id.clone(),
            name: self.name.clone(),
            protocol: self.protocol.clone(),
            params: self
                .params
                .iter()
                .filter(|(key, _)| !SECRETS.contains(&key.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            configured_secrets: SECRETS
                .iter()
                .filter(|key| self.params.get(**key).is_some_and(|s| !s.is_empty()))
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

/// Numeric advanced settings: (parameter, minimum, maximum, message shown next to the field).
const NUMERIC_OPTIONS: &[(&str, u32, u32, &str)] = &[
    (
        "timeout",
        1,
        120,
        "連線逾時須為 1 到 120 的整數，或留空使用預設值。",
    ),
    (
        "scrollback",
        1,
        100_000,
        "捲動緩衝須為 1 到 100000 的整數，或留空使用預設值。",
    ),
    (
        "server-alive-interval",
        0,
        3600,
        "保持連線間隔須為 0 到 3600 的整數，或留空。",
    ),
    (
        "font-size",
        6,
        72,
        "字型大小須為 6 到 72 的整數，或留空使用預設值。",
    ),
];

/// A rejected draft: which form field is wrong and what the user should do.
#[derive(Debug, PartialEq, Eq)]
pub struct Invalid {
    pub field: &'static str,
    pub message: &'static str,
}

fn invalid(field: &'static str, message: &'static str) -> Invalid {
    Invalid { field, message }
}

pub fn validate(draft: &Draft) -> Result<(), Invalid> {
    if draft.name.trim().is_empty() || draft.name.chars().count() > 120 {
        return Err(invalid("name", "請輸入 1 到 120 個字元的連線名稱。"));
    }
    static REGISTRY: std::sync::OnceLock<guacd::Registry> = std::sync::OnceLock::new();
    let Some(plugin) = REGISTRY
        .get_or_init(guacd::Registry::builtin)
        .get(&draft.protocol)
    else {
        return Err(invalid(
            "protocol",
            "不支援這個協定，請改選 SSH、Telnet、RDP 或 VNC。",
        ));
    };
    if draft.params.len() > 64
        || draft.params.iter().any(|(key, value)| {
            !plugin.arg_names().contains(&key.as_str())
                || value.chars().count() > guac_protocol::MAX_ELEMENT_LEN
                || value.contains('\0')
        })
    {
        return Err(invalid(
            "params",
            "有欄位的內容無效或過長，請檢查後重新儲存。",
        ));
    }
    if draft
        .params
        .get("hostname")
        .is_none_or(|s| s.trim().is_empty() || s.len() > 255)
    {
        return Err(invalid(
            "hostname",
            "請填寫主機名稱或 IP（最長 255 字元）。",
        ));
    }
    for (name, min, max, message) in NUMERIC_OPTIONS {
        if let Some(text) = draft
            .params
            .get(*name)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            if !text
                .parse::<u32>()
                .is_ok_and(|n| (*min..=*max).contains(&n))
            {
                return Err(invalid(name, message));
            }
        }
    }
    if let Some(label) = draft.params.get("character-encoding") {
        if guacd::TextEncoding::parse(Some(label)).is_err() {
            return Err(invalid(
                "character-encoding",
                "不支援這個文字編碼，請選擇 UTF-8 或清單中的舊編碼。",
            ));
        }
    }
    if let Some(pin) = draft
        .params
        .get("server-cert-sha256")
        .filter(|s| !s.trim().is_empty())
    {
        if guacd::normalize_fingerprint(pin).is_none() {
            return Err(invalid(
                "server-cert-sha256",
                "憑證指紋須為 64 個十六進位字元（SHA-256），可含冒號或空白；或留空。",
            ));
        }
    }
    if draft.protocol == "rdp"
        && draft.params.get("ignore-cert").is_some_and(|v| v == "true")
        && draft
            .params
            .get("server-cert-sha256")
            .is_none_or(|s| s.trim().is_empty())
        && !matches!(
            draft.params.get("security").map(String::as_str),
            None | Some("") | Some("nla")
        )
    {
        return Err(invalid(
            "ignore-cert",
            "不驗證憑證時必須選「僅 NLA」，或改為填入憑證指紋。",
        ));
    }
    if let Some(port) = draft.params.get("port").filter(|s| !s.is_empty()) {
        if port.parse::<u16>().ok().filter(|p| *p > 0).is_none() {
            return Err(invalid(
                "port",
                "連接埠須為 1 到 65535 的數字，或留空使用預設值。",
            ));
        }
    }
    Ok(())
}

/// Parameters that decide where, and under what verification, saved secrets are sent.
/// Pins (`host-key`, `server-cert-sha256`) are deliberately excluded: trusting a host
/// updates them on an otherwise unchanged connection.
const DESTINATION: &[&str] = &["hostname", "port", "ignore-cert", "security"];

/// Saved secrets may only be carried over when the destination is unchanged;
/// otherwise a stolen session could redirect them to an attacker's host.
fn same_destination(old: &BTreeMap<String, String>, new: &BTreeMap<String, String>) -> bool {
    let norm = |params: &BTreeMap<String, String>, key: &str| {
        params
            .get(key)
            .map(|v| v.trim().to_ascii_lowercase())
            .unwrap_or_default()
    };
    DESTINATION
        .iter()
        .all(|key| norm(old, key) == norm(new, key))
}

pub struct Store {
    path: PathBuf,
    entries: Mutex<BTreeMap<String, Connection>>,
}

impl Store {
    /// 讀取單筆既有連線供本機協定引擎使用；不存在時不建立檔案。
    pub async fn get_existing(directory: PathBuf, id: String) -> io::Result<Option<Connection>> {
        tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(directory.join("connections.json"))?;
            let mut entries: BTreeMap<String, Connection> =
                serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            Ok(entries.remove(&id))
        })
        .await
        .map_err(io::Error::other)?
    }
    /// 載入既有連線的公開欄位；不建立資料目錄，亦不回傳機密。
    pub async fn list_existing(directory: PathBuf) -> io::Result<Vec<PublicConnection>> {
        tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(directory.join("connections.json"))?;
            let entries: BTreeMap<String, Connection> =
                serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            Ok(entries.values().map(Connection::public).collect())
        })
        .await
        .map_err(io::Error::other)?
    }

    pub async fn open(directory: PathBuf) -> io::Result<Self> {
        let path = directory.join("connections.json");
        let entries = tokio::task::spawn_blocking({
            let path = path.clone();
            move || -> io::Result<BTreeMap<String, Connection>> {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    std::fs::DirBuilder::new()
                        .recursive(true)
                        .mode(0o700)
                        .create(path.parent().unwrap())?;
                }
                #[cfg(not(unix))]
                std::fs::create_dir_all(path.parent().unwrap())?;
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            std::fs::set_permissions(
                                &path,
                                std::fs::Permissions::from_mode(0o600),
                            )?;
                        }
                        serde_json::from_slice(&bytes).map_err(io::Error::other)
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
                    Err(e) => Err(e),
                }
            }
        })
        .await
        .map_err(io::Error::other)??;
        Ok(Self {
            path,
            entries: Mutex::new(entries),
        })
    }

    pub async fn list(&self) -> Vec<PublicConnection> {
        self.entries
            .lock()
            .await
            .values()
            .map(Connection::public)
            .collect()
    }
    pub async fn get(&self, id: &str) -> Option<Connection> {
        self.entries.lock().await.get(id).cloned()
    }

    pub async fn save(
        &self,
        id: Option<&str>,
        draft: Draft,
    ) -> io::Result<Option<PublicConnection>> {
        let mut entries = self.entries.lock().await;
        let mut next = entries.clone();
        let id = match id {
            Some(id) if next.contains_key(id) => id.to_string(),
            Some(_) => return Ok(None),
            None => Uuid::new_v4().to_string(),
        };
        let mut params = draft.params;
        if let Some(old) = next.get(&id) {
            if old.protocol == draft.protocol && same_destination(&old.params, &params) {
                for key in SECRETS {
                    if !params.contains_key(*key) {
                        if let Some(value) = old.params.get(*key) {
                            params.insert(key.to_string(), value.clone());
                        }
                    }
                }
            }
        }
        let connection = Connection {
            id: id.clone(),
            name: draft.name.trim().to_string(),
            protocol: draft.protocol,
            params,
        };
        let public = connection.public();
        next.insert(id, connection);
        self.persist(&next).await?;
        *entries = next;
        Ok(Some(public))
    }

    pub async fn delete(&self, id: &str) -> io::Result<bool> {
        let mut entries = self.entries.lock().await;
        let mut next = entries.clone();
        if next.remove(id).is_none() {
            return Ok(false);
        }
        self.persist(&next).await?;
        *entries = next;
        Ok(true)
    }

    async fn persist(&self, entries: &BTreeMap<String, Connection>) -> io::Result<()> {
        let path = self.path.clone();
        let bytes = serde_json::to_vec_pretty(entries).map_err(io::Error::other)?;
        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
            temp.write_all(&bytes)?;
            temp.as_file().sync_all()?;
            temp.persist(path).map_err(|e| e.error)?;
            Ok(())
        })
        .await
        .map_err(io::Error::other)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rdp(params: &[(&str, &str)]) -> Draft {
        let mut map = BTreeMap::from([("hostname".to_string(), "h.example".to_string())]);
        map.extend(params.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        Draft {
            name: "rdp".into(),
            protocol: "rdp".into(),
            params: map,
        }
    }

    #[test]
    fn destination_change_detection_ignores_pins_and_case() {
        let a = BTreeMap::from([("hostname".to_string(), "Host".to_string())]);
        let mut b = a.clone();
        b.insert("host-key".into(), "pin".into());
        b.insert("hostname".into(), " host ".into());
        assert!(same_destination(&a, &b));
        b.insert("ignore-cert".into(), "true".into());
        assert!(!same_destination(&a, &b));
    }

    #[tokio::test]
    async fn changed_destination_does_not_retain_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().into()).await.unwrap();
        let saved = store
            .save(None, rdp(&[("password", "secret")]))
            .await
            .unwrap()
            .unwrap();
        let changed = store
            .save(Some(&saved.id), rdp(&[("security", "tls")]))
            .await
            .unwrap()
            .unwrap();
        assert!(changed.configured_secrets.is_empty());
        assert!(!store
            .get(&saved.id)
            .await
            .unwrap()
            .params
            .contains_key("password"));
    }

    #[test]
    fn ignore_cert_without_pin_requires_nla() {
        for security in ["any", "tls"] {
            let draft = rdp(&[("ignore-cert", "true"), ("security", security)]);
            assert_eq!(validate(&draft).unwrap_err().field, "ignore-cert");
        }
        assert!(validate(&rdp(&[("ignore-cert", "true"), ("security", "nla")])).is_ok());
        assert!(validate(&rdp(&[("ignore-cert", "true")])).is_ok());
        let pin = "ab".repeat(32);
        assert!(validate(&rdp(&[
            ("ignore-cert", "true"),
            ("security", "any"),
            ("server-cert-sha256", &pin)
        ]))
        .is_ok());
    }

    #[tokio::test]
    async fn secrets_are_preserved_cleared_and_never_returned() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().into()).await.unwrap();
        let mut draft = rdp(&[("password", "secret")]);
        let saved = store.save(None, draft).await.unwrap().unwrap();
        assert!(!saved.params.contains_key("password"));
        assert_eq!(saved.configured_secrets, ["password"]);
        draft = rdp(&[]);
        let saved = store.save(Some(&saved.id), draft).await.unwrap().unwrap();
        assert_eq!(saved.configured_secrets, ["password"]);
        let draft = rdp(&[("password", "")]);
        let saved = store.save(Some(&saved.id), draft).await.unwrap().unwrap();
        assert!(saved.configured_secrets.is_empty());
        assert_eq!(store.get(&saved.id).await.unwrap().params["password"], "");
    }

    #[tokio::test]
    async fn list_existing_filters_secrets_without_creating_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert_eq!(
            Store::list_existing(missing.clone())
                .await
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert!(!missing.exists());
        let store = Store::open(dir.path().into()).await.unwrap();
        let saved = store
            .save(None, rdp(&[("password", "test-secret")]))
            .await
            .unwrap()
            .unwrap();
        let entries = Store::list_existing(dir.path().into()).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].params.contains_key("password"));
        assert_eq!(entries[0].configured_secrets, ["password"]);
        let connection = Store::get_existing(dir.path().into(), saved.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(connection.params["password"], "test-secret");
        assert!(Store::get_existing(dir.path().into(), "missing".into())
            .await
            .unwrap()
            .is_none());
    }
}
