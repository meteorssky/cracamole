//! Windows 私有目錄的原子建立與嚴格 ACL 驗證。
use std::{
    ffi::OsStr,
    io, mem,
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, ERROR_ALREADY_EXISTS, HANDLE},
    Security::{
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
            SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        EqualSid, GetTokenInformation, TokenUser, DACL_SECURITY_INFORMATION,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER,
    },
    Storage::FileSystem::{CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT},
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain([0]).collect()
}

struct Local(*mut core::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}
struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

struct User {
    token_data: Vec<usize>,
    sid_string: String,
}
impl User {
    fn sid(&self) -> PSID {
        // GetTokenInformation wrote a TOKEN_USER into aligned token_data.
        unsafe { (*(self.token_data.as_ptr().cast::<TOKEN_USER>())).User.Sid }
    }

    fn current() -> io::Result<Self> {
        let mut token = Token(ptr::null_mut());
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token.0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut bytes = 0u32;
        unsafe {
            GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut bytes);
        }
        if bytes == 0 || bytes > 64 * 1024 {
            return Err(io::Error::other("invalid user token size"));
        }
        let mut token_data = vec![0usize; (bytes as usize).div_ceil(mem::size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                token_data.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let sid = unsafe { (*(token_data.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        let mut raw = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid, &mut raw) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let allocation = Local(raw.cast());
        let len = (0..256)
            .find(|&index| unsafe { *raw.add(index) == 0 })
            .ok_or_else(|| io::Error::other("invalid SID"))?;
        let sid_string = String::from_utf16(unsafe { std::slice::from_raw_parts(raw, len) })
            .map_err(io::Error::other)?;
        drop(allocation);
        Ok(Self {
            token_data,
            sid_string,
        })
    }
}

fn descriptor(text: &str) -> io::Result<Local> {
    let mut result: PSECURITY_DESCRIPTOR = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(OsStr::new(text)).as_ptr(),
            SDDL_REVISION_1,
            &mut result,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(Local(result))
}

fn dacl_string(security: PSECURITY_DESCRIPTOR) -> io::Result<String> {
    let mut raw = ptr::null_mut();
    let mut len = 0u32;
    if unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            security,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut raw,
            &mut len,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let allocation = Local(raw.cast());
    if len == 0 || len > 65536 {
        return Err(io::Error::other("invalid DACL string"));
    }
    let text = String::from_utf16(unsafe { std::slice::from_raw_parts(raw, (len - 1) as usize) })
        .map_err(io::Error::other)?;
    drop(allocation);
    Ok(text)
}

/// 建立時指定僅目前使用者與 SYSTEM 可存取的受保護 DACL；既有目錄不會被修改。
pub fn ensure_private_directory(path: &Path) -> io::Result<()> {
    let user = User::current()?;
    let expected = descriptor(&format!(
        "D:P(A;OICI;FA;;;{})(A;OICI;FA;;;SY)",
        user.sid_string
    ))?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: expected.0,
        bInheritHandle: 0,
    };
    let path_wide = wide(path.as_os_str());
    if path_wide[..path_wide.len() - 1].contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "data path contains NUL",
        ));
    }
    if unsafe { CreateDirectoryW(path_wide.as_ptr(), &attributes) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
            return Err(error);
        }
    }
    verify_security(path, &user, &expected, true)
}

fn verify_security(path: &Path, user: &User, expected: &Local, directory: bool) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private data path is not a regular directory",
        ));
    }
    let mut actual: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let mut owner: PSID = ptr::null_mut();
    let path_wide = wide(path.as_os_str());
    if path_wide[..path_wide.len() - 1].contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "data path contains NUL",
        ));
    }
    let error = unsafe {
        GetNamedSecurityInfoW(
            path_wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut actual,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let actual = Local(actual);
    if owner.is_null()
        || unsafe { EqualSid(owner, user.sid()) } == 0
        || dacl_string(actual.0)? != dacl_string(expected.0)?
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private data directory ACL or owner is not trusted",
        ));
    }
    Ok(())
}

/// 驗證既有檔案由目前使用者擁有，且只繼承私有目錄的兩條允許規則。
pub fn verify_private_file(path: &Path) -> io::Result<()> {
    let user = User::current()?;
    let expected = descriptor(&format!("D:(A;ID;FA;;;{})(A;ID;FA;;;SY)", user.sid_string))?;
    verify_security(path, &user, &expected, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;

    #[test]
    fn rejects_insecure_existing_acl_without_modifying_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("private");
        ensure_private_directory(&path).unwrap();
        ensure_private_directory(&path).unwrap();
        let child = path.join("probe");
        std::fs::write(&child, b"probe").unwrap();
        verify_private_file(&child).unwrap();
        let file_output = std::process::Command::new(
            std::env::var_os("SystemRoot")
                .map(std::path::PathBuf::from)
                .unwrap()
                .join("System32/icacls.exe"),
        )
        .arg(&child)
        .args(["/grant", "*S-1-1-0:F"])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
        assert!(file_output.status.success());
        assert_eq!(
            verify_private_file(&child).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        ensure_private_directory(&path).unwrap();
        let output = std::process::Command::new(
            std::env::var_os("SystemRoot")
                .map(std::path::PathBuf::from)
                .unwrap()
                .join("System32/icacls.exe"),
        )
        .arg(&path)
        .args(["/grant", "*S-1-1-0:(OI)(CI)F"])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
        assert!(output.status.success());
        assert_eq!(
            ensure_private_directory(&path).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[tokio::test]
    async fn atomic_store_replacements_inherit_private_acl() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("private");
        ensure_private_directory(&directory).unwrap();
        let store = cracamole_store::Store::open(directory.clone())
            .await
            .unwrap();
        for _ in 0..2 {
            let draft = cracamole_store::Draft {
                name: "fixture".into(),
                protocol: "telnet".into(),
                params: std::collections::BTreeMap::from([
                    ("hostname".into(), "localhost".into()),
                    ("password".into(), "fixture-secret".into()),
                ]),
            };
            store.save(None, draft).await.unwrap().unwrap();
            verify_private_file(&directory.join("connections.json")).unwrap();
        }
    }
}
