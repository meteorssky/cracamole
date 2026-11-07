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
    let registry = guacd::Registry::builtin();
    let Some(plugin) = registry.get(&draft.protocol) else {
        return Err(invalid(
            "protocol",
            "不支援這個協定，請改選 SSH、Telnet 或 RDP。",
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

pub struct Store {
    path: PathBuf,
    entries: Mutex<BTreeMap<String, Connection>>,
}

impl Store {
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
            if old.protocol == draft.protocol {
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
