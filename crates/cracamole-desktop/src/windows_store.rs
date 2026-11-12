//! 僅在私有 ACL 與單一桌面程序鎖成立時允許寫入連線設定。
use crate::windows_private::{ensure_private_directory, verify_private_file};
use cracamole_store::{Connection, Draft, Invalid, PublicConnection, Store};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Mutex,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

#[derive(Debug)]
pub enum SaveError {
    Invalid(Invalid),
    Io(io::Error),
}

pub struct WindowsStore {
    directory: PathBuf,
    store: Store,
    // share_mode(0) keeps other desktop instances from opening this lock file.
    _lock: File,
    // 不允許其他程序在工作階段中刪除或重新命名目錄，避免 ACL 驗證後被替換。
    _directory_guard: File,
    trust_lock: Mutex<()>,
}

type SshIdentities = BTreeMap<String, BTreeMap<String, String>>;
const TRUST_FILE: &str = "ssh-identities.json";
const PINS_FILE: &str = "pinned-connections.json";
const RECENT_FILE: &str = "recent-connections.json";
const MAX_TRUST_BYTES: u64 = 1024 * 1024;

fn identity_host(host: &str) -> io::Result<String> {
    let host = host.trim();
    if host.is_empty()
        || host.len() > 255
        || host.chars().any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid SSH identity host",
        ));
    }
    Ok(host.to_ascii_lowercase())
}

fn valid_public_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 8192 || key.lines().count() != 1 {
        return false;
    }
    let (entries, errors) = proto_ssh::hostkey::parse(key);
    errors.is_empty() && entries.len() == 1 && !entries[0].revoked && entries[0].hosts.is_none()
}

fn valid_pin_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
            }
        })
}
fn verify_if_present(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => verify_private_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn path_identity(path: &Path) -> io::Result<PathBuf> {
    if let Ok(existing) = path.canonicalize() {
        return Ok(existing);
    }
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid data directory"))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Ok(parent.canonicalize()?.join(name))
}

impl WindowsStore {
    pub async fn open(directory: PathBuf) -> io::Result<Self> {
        let web_directory = std::env::var_os("CRACAMOLE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"));
        if let (Ok(desktop), Ok(web)) = (path_identity(&directory), path_identity(&web_directory)) {
            if desktop
                .to_string_lossy()
                .eq_ignore_ascii_case(&web.to_string_lossy())
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "desktop and web data directories must be separate",
                ));
            }
        }
        ensure_private_directory(&directory)?;
        let directory_guard = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&directory)?;
        ensure_private_directory(&directory)?;
        if let (Ok(desktop), Ok(web)) = (directory.canonicalize(), web_directory.canonicalize()) {
            if desktop
                .to_string_lossy()
                .eq_ignore_ascii_case(&web.to_string_lossy())
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "desktop and web data directories must be separate",
                ));
            }
        }
        let lock_path = directory.join("desktop.lock");
        verify_if_present(&lock_path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&lock_path)?;
        verify_private_file(&lock_path)?;
        verify_if_present(&directory.join("connections.json"))?;
        verify_if_present(&directory.join(TRUST_FILE))?;
        verify_if_present(&directory.join(PINS_FILE))?;
        verify_if_present(&directory.join(RECENT_FILE))?;
        let store = Store::open(directory.clone()).await?;
        Ok(Self {
            directory,
            store,
            _lock: lock,
            _directory_guard: directory_guard,
            trust_lock: Mutex::new(()),
        })
    }

    fn verify(&self) -> io::Result<()> {
        ensure_private_directory(&self.directory)?;
        verify_private_file(&self.directory.join("desktop.lock"))?;
        verify_if_present(&self.directory.join("connections.json"))?;
        verify_if_present(&self.directory.join(TRUST_FILE))?;
        verify_if_present(&self.directory.join(PINS_FILE))?;
        verify_if_present(&self.directory.join(RECENT_FILE))
    }

    fn read_ssh_identities(&self) -> io::Result<SshIdentities> {
        self.verify()?;
        let path = self.directory.join(TRUST_FILE);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(error) => return Err(error),
        };
        verify_private_file(&path)?;
        let mut bytes = Vec::new();
        file.take(MAX_TRUST_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_TRUST_BYTES {
            return Err(io::Error::other("SSH identity store too large"));
        }
        let identities: SshIdentities = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let mut count = 0usize;
        for (host, ports) in &identities {
            if !identity_host(host).is_ok_and(|normalized| normalized == *host) {
                return Err(io::Error::other("invalid SSH identity host"));
            }
            for (port, key) in ports {
                count += 1;
                if count > 4096
                    || !port
                        .parse::<u16>()
                        .is_ok_and(|value| value > 0 && value.to_string() == *port)
                    || !valid_public_key(key)
                {
                    return Err(io::Error::other("invalid SSH identity record"));
                }
            }
        }
        Ok(identities)
    }

    pub fn ssh_identity(&self, host: &str, port: u16) -> io::Result<Option<String>> {
        let _guard = self
            .trust_lock
            .lock()
            .map_err(|_| io::Error::other("SSH identity lock poisoned"))?;
        let host = identity_host(host)?;
        if port == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid SSH identity port",
            ));
        }
        Ok(self
            .read_ssh_identities()?
            .get(&host)
            .and_then(|ports| ports.get(&port.to_string()))
            .cloned())
    }

    pub fn save_ssh_identity(&self, host: &str, port: u16, key: &str) -> io::Result<()> {
        let _guard = self
            .trust_lock
            .lock()
            .map_err(|_| io::Error::other("SSH identity lock poisoned"))?;
        let host = identity_host(host)?;
        if port == 0 || !valid_public_key(key) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid SSH identity",
            ));
        }
        let mut identities = self.read_ssh_identities()?;
        let ports = identities.entry(host).or_default();
        if let Some(existing) = ports.get(&port.to_string()) {
            return if existing == key {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "SSH identity conflict",
                ))
            };
        }
        ports.insert(port.to_string(), key.to_string());
        let bytes = serde_json::to_vec(&identities).map_err(io::Error::other)?;
        if bytes.len() as u64 > MAX_TRUST_BYTES
            || identities.values().map(BTreeMap::len).sum::<usize>() > 4096
        {
            return Err(io::Error::other("SSH identity store limit exceeded"));
        }
        let mut temp = tempfile::NamedTempFile::new_in(&self.directory)?;
        verify_private_file(temp.path())?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        self.verify()?;
        temp.persist(self.directory.join(TRUST_FILE))
            .map_err(|error| error.error)?;
        verify_private_file(&self.directory.join(TRUST_FILE))
    }

    /// 只儲存公開連線 ID；損壞或權限不正確時拒絕讀取。
    pub fn pinned(&self) -> io::Result<BTreeSet<String>> {
        let _guard = self
            .trust_lock
            .lock()
            .map_err(|_| io::Error::other("metadata lock poisoned"))?;
        self.read_pinned()
    }

    fn read_pinned(&self) -> io::Result<BTreeSet<String>> {
        self.verify()?;
        let path = self.directory.join(PINS_FILE);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
            Err(error) => return Err(error),
        };
        verify_private_file(&path)?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err(io::Error::other("pin store too large"));
        }
        let pins: BTreeSet<String> = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if pins.len() > 4096 || pins.iter().any(|id| !valid_pin_id(id)) {
            return Err(io::Error::other("invalid pinned connection ID"));
        }
        Ok(pins)
    }

    pub fn set_pinned(&self, id: &str, pinned: bool) -> io::Result<()> {
        if !valid_pin_id(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid pinned connection ID",
            ));
        }
        let _guard = self
            .trust_lock
            .lock()
            .map_err(|_| io::Error::other("metadata lock poisoned"))?;
        let mut pins = self.read_pinned()?;
        if pinned {
            pins.insert(id.to_owned());
        } else {
            pins.remove(id);
        }
        if pins.len() > 4096 {
            return Err(io::Error::other("pin store limit exceeded"));
        }
        let bytes = serde_json::to_vec(&pins).map_err(io::Error::other)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.directory)?;
        verify_private_file(temp.path())?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        self.verify()?;
        temp.persist(self.directory.join(PINS_FILE))
            .map_err(|error| error.error)?;
        verify_private_file(&self.directory.join(PINS_FILE))
    }
    /// 最近成功連線的公開 ID，依使用時間由舊至新排列。
    pub fn recent(&self) -> io::Result<Vec<String>> {
        let _guard = self
            .trust_lock
            .lock()
            .map_err(|_| io::Error::other("metadata lock poisoned"))?;
        self.read_recent()
    }

    fn read_recent(&self) -> io::Result<Vec<String>> {
        self.verify()?;
        let path = self.directory.join(RECENT_FILE);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        verify_private_file(&path)?;
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(io::Error::other("recent store too large"));
        }
        let recent: Vec<String> = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let unique: BTreeSet<_> = recent.iter().collect();
        if recent.len() > 20
            || unique.len() != recent.len()
            || recent.iter().any(|id| !valid_pin_id(id))
        {
            return Err(io::Error::other("invalid recent connection IDs"));
        }
        Ok(recent)
    }

    pub fn record_recent(&self, id: &str) -> io::Result<()> {
        if !valid_pin_id(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid recent connection ID",
            ));
        }
        let _guard = self
            .trust_lock
            .lock()
            .map_err(|_| io::Error::other("metadata lock poisoned"))?;
        let mut recent = self.read_recent()?;
        recent.retain(|entry| entry != id);
        recent.push(id.to_owned());
        if recent.len() > 20 {
            recent.remove(0);
        }
        self.write_recent(&recent)
    }

    fn forget_recent(&self, id: &str) -> io::Result<()> {
        let _guard = self
            .trust_lock
            .lock()
            .map_err(|_| io::Error::other("metadata lock poisoned"))?;
        let mut recent = self.read_recent()?;
        let previous = recent.len();
        recent.retain(|entry| entry != id);
        if recent.len() == previous {
            return Ok(());
        }
        self.write_recent(&recent)
    }

    fn write_recent(&self, recent: &[String]) -> io::Result<()> {
        let bytes = serde_json::to_vec(recent).map_err(io::Error::other)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.directory)?;
        verify_private_file(temp.path())?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        self.verify()?;
        temp.persist(self.directory.join(RECENT_FILE))
            .map_err(|error| error.error)?;
        verify_private_file(&self.directory.join(RECENT_FILE))
    }
    pub async fn list(&self) -> io::Result<Vec<PublicConnection>> {
        self.verify()?;
        Ok(self.store.list().await)
    }

    pub async fn get(&self, id: &str) -> io::Result<Option<Connection>> {
        self.verify()?;
        Ok(self.store.get(id).await)
    }

    pub async fn save(
        &self,
        id: Option<&str>,
        draft: Draft,
    ) -> Result<Option<PublicConnection>, SaveError> {
        cracamole_store::validate(&draft).map_err(SaveError::Invalid)?;
        self.verify().map_err(SaveError::Io)?;
        let result = self.store.save(id, draft).await.map_err(SaveError::Io)?;
        self.verify().map_err(SaveError::Io)?;
        Ok(result)
    }

    pub async fn rename_group(&self, old: &str, new: &str) -> Result<usize, SaveError> {
        self.verify().map_err(SaveError::Io)?;
        let result = self
            .store
            .rename_group(old, new)
            .await
            .map_err(SaveError::Io)?
            .map_err(SaveError::Invalid)?;
        self.verify().map_err(SaveError::Io)?;
        Ok(result)
    }

    pub async fn import_new(&self, drafts: Vec<Draft>) -> Result<usize, SaveError> {
        self.verify().map_err(SaveError::Io)?;
        let created = self
            .store
            .import_new(drafts)
            .await
            .map_err(SaveError::Io)?
            .map_err(SaveError::Invalid)?;
        self.verify().map_err(SaveError::Io)?;
        Ok(created.len())
    }
    pub async fn delete(&self, id: &str) -> io::Result<bool> {
        self.verify()?;
        let result = self.store.delete(id).await?;
        self.verify()?;
        // 清理已刪除 ID；若額外的中繼資料寫入失敗，不回報成連線刪除失敗。
        if result && self.pinned().is_ok_and(|pins| pins.contains(id)) {
            let _ = self.set_pinned(id, false);
        }
        if result {
            let _ = self.forget_recent(id);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::os::windows::process::CommandExt;

    fn draft(password: Option<&str>) -> Draft {
        let mut params = BTreeMap::from([("hostname".into(), "example.test".into())]);
        if let Some(password) = password {
            params.insert("password".into(), password.into());
        }
        Draft {
            name: "example".into(),
            protocol: "ssh".into(),
            params,
        }
    }

    #[test]
    fn missing_directory_aliases_have_the_same_identity_without_creation() {
        let parent = tempfile::tempdir().unwrap();
        let first = parent.path().join("data");
        let alias = parent.path().join(".").join("data");
        assert_eq!(
            path_identity(&first).unwrap(),
            path_identity(&alias).unwrap()
        );
        assert!(!first.exists());
    }

    #[tokio::test]
    async fn private_store_validates_locks_and_preserves_secrets() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("private");
        let store = WindowsStore::open(path.clone()).await.unwrap();
        assert!(WindowsStore::open(path.clone()).await.is_err());
        assert!(std::fs::rename(&path, parent.path().join("moved")).is_err());
        let mut invalid = draft(None);
        invalid.name.clear();
        assert!(matches!(
            store.save(None, invalid).await,
            Err(SaveError::Invalid(_))
        ));
        assert!(!path.join("connections.json").exists());
        let saved = store
            .save(None, draft(Some("secret")))
            .await
            .unwrap()
            .unwrap();
        let saved = store
            .save(Some(&saved.id), draft(None))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.configured_secrets, ["password"]);
        assert!(!saved.params.contains_key("password"));
        let saved = store
            .save(Some(&saved.id), draft(Some("")))
            .await
            .unwrap()
            .unwrap();
        assert!(saved.configured_secrets.is_empty());
        assert!(store.delete(&saved.id).await.unwrap());
        drop(store);
        assert!(WindowsStore::open(path).await.is_ok());
    }

    #[tokio::test]
    async fn bulk_import_is_atomic_and_creates_new_ids() {
        let parent = tempfile::tempdir().unwrap();
        let store = WindowsStore::open(parent.path().join("private"))
            .await
            .unwrap();
        let saved = store
            .save(None, draft(Some("existing-secret")))
            .await
            .unwrap()
            .unwrap();
        let mut invalid = draft(None);
        invalid.name.clear();
        assert!(matches!(
            store.import_new(vec![draft(None), invalid]).await,
            Err(SaveError::Invalid(_))
        ));
        assert_eq!(store.list().await.unwrap().len(), 1);
        assert_eq!(
            store
                .import_new(vec![draft(None), draft(None)])
                .await
                .unwrap(),
            2
        );
        let entries = store.list().await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(
            store.get(&saved.id).await.unwrap().unwrap().params["password"],
            "existing-secret"
        );
        assert_eq!(
            entries.iter().filter(|entry| entry.id == saved.id).count(),
            1
        );
    }

    #[tokio::test]
    async fn independent_ssh_identity_is_private_bounded_and_never_overwritten() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("private");
        let store = WindowsStore::open(path.clone()).await.unwrap();
        let key =
            russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
                .unwrap()
                .public_key()
                .to_openssh()
                .unwrap();
        let other =
            russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
                .unwrap()
                .public_key()
                .to_openssh()
                .unwrap();
        assert_eq!(store.ssh_identity("Host.Example", 22).unwrap(), None);
        store.save_ssh_identity("Host.Example", 22, &key).unwrap();
        assert_eq!(
            store.ssh_identity("host.example", 22).unwrap().as_deref(),
            Some(key.as_str())
        );
        assert_eq!(store.ssh_identity("host.example", 2222).unwrap(), None);
        assert!(!path.join("connections.json").exists());
        assert_eq!(
            store
                .save_ssh_identity("host.example", 22, &other)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(store
            .save_ssh_identity("host.example", 23, "@cert-authority * ssh-ed25519 invalid")
            .is_err());
        verify_private_file(&path.join(TRUST_FILE)).unwrap();
        drop(store);
        let store = WindowsStore::open(path.clone()).await.unwrap();
        assert_eq!(
            store.ssh_identity("HOST.EXAMPLE", 22).unwrap().as_deref(),
            Some(key.as_str())
        );
        std::fs::write(path.join(TRUST_FILE), b"not valid json").unwrap();
        assert!(store.ssh_identity("host.example", 22).is_err());
        assert!(store.save_ssh_identity("host.example", 22, &key).is_err());
    }

    #[tokio::test]
    async fn independent_ssh_identity_rejects_broadened_acl() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("private");
        let store = WindowsStore::open(path.clone()).await.unwrap();
        let key =
            russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
                .unwrap()
                .public_key()
                .to_openssh()
                .unwrap();
        store.save_ssh_identity("host.example", 22, &key).unwrap();
        let file = path.join(TRUST_FILE);
        let before = std::fs::read(&file).unwrap();
        let result = std::process::Command::new(
            std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .unwrap()
                .join("System32/icacls.exe"),
        )
        .arg(&file)
        .args(["/grant", "*S-1-1-0:F"])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
        assert!(result.status.success());
        assert_eq!(
            store.ssh_identity("host.example", 22).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            store
                .save_ssh_identity("other.example", 22, &key)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(std::fs::read(file).unwrap(), before);
    }

    #[tokio::test]
    async fn refuses_to_save_when_existing_file_acl_is_broadened() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("private");
        let store = WindowsStore::open(path.clone()).await.unwrap();
        store
            .save(None, draft(Some("secret")))
            .await
            .unwrap()
            .unwrap();
        let file = path.join("connections.json");
        let before = std::fs::read(&file).unwrap();
        let result = std::process::Command::new(
            std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .unwrap()
                .join("System32/icacls.exe"),
        )
        .arg(&file)
        .args(["/grant", "*S-1-1-0:F"])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
        assert!(result.status.success());
        assert!(matches!(
            store.save(None, draft(None)).await,
            Err(SaveError::Io(_))
        ));
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
}
