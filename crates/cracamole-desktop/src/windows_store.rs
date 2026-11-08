//! 僅在私有 ACL 與單一桌面程序鎖成立時允許寫入連線設定。
use crate::windows_private::{ensure_private_directory, verify_private_file};
use cracamole_store::{Connection, Draft, Invalid, PublicConnection, Store};
use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
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
        let store = Store::open(directory.clone()).await?;
        Ok(Self {
            directory,
            store,
            _lock: lock,
            _directory_guard: directory_guard,
        })
    }

    fn verify(&self) -> io::Result<()> {
        ensure_private_directory(&self.directory)?;
        verify_private_file(&self.directory.join("desktop.lock"))?;
        verify_if_present(&self.directory.join("connections.json"))
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

    pub async fn delete(&self, id: &str) -> io::Result<bool> {
        self.verify()?;
        let result = self.store.delete(id).await?;
        self.verify()?;
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
