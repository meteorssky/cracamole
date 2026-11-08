//! 有界讀寫 Windows Unicode 剪貼簿，並處理全域記憶體的所有權。
use std::{ptr, slice};
use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL, HWND};
use windows_sys::Win32::System::{
    DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
    },
    Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
};
use zeroize::{Zeroize, Zeroizing};

const CF_UNICODETEXT: u32 = 13;
const MAX_UTF8: usize = 256 * 1024;
const MAX_UTF16_BYTES: usize = MAX_UTF8 * 2 + 2;

struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

struct GlobalMemory(HGLOBAL);
impl Drop for GlobalMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let size = GlobalSize(self.0);
                let pointer = GlobalLock(self.0).cast::<u8>();
                if !pointer.is_null() {
                    ptr::write_bytes(pointer, 0, size);
                    GlobalUnlock(self.0);
                }
                GlobalFree(self.0);
            }
        }
    }
}

/// 將文字寫入系統剪貼簿。記憶體在轉交給 Windows 前由本程序持有。
pub fn write_text(owner: HWND, text: &str) -> Result<(), &'static str> {
    if owner.is_null() {
        return Err("遠端剪貼簿文字過大或格式無效。");
    }
    let units = encode(text)?;
    let bytes = units.len() * std::mem::size_of::<u16>();
    let mut memory = GlobalMemory(unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) });
    if memory.0.is_null() {
        return Err("無法配置剪貼簿記憶體。");
    }
    let pointer = unsafe { GlobalLock(memory.0) }.cast::<u16>();
    if pointer.is_null() {
        return Err("無法存取剪貼簿記憶體。");
    }
    unsafe {
        ptr::copy_nonoverlapping(units.as_ptr(), pointer, units.len());
        GlobalUnlock(memory.0);
    }
    if unsafe { OpenClipboard(owner) } == 0 {
        return Err("無法開啟剪貼簿。");
    }
    let _clipboard = Clipboard;
    if unsafe { EmptyClipboard() } == 0 {
        return Err("無法清除剪貼簿。");
    }
    if unsafe { SetClipboardData(CF_UNICODETEXT, memory.0) }.is_null() {
        return Err("無法寫入剪貼簿。");
    }
    // Windows owns the handle after successful SetClipboardData.
    memory.0 = ptr::null_mut();
    Ok(())
}

fn encode(text: &str) -> Result<Zeroizing<Vec<u16>>, &'static str> {
    if text.len() > MAX_UTF8 || text.contains('\0') {
        return Err("遠端剪貼簿文字過大或格式無效。");
    }
    let mut units = Zeroizing::new(Vec::with_capacity(text.len() + 1));
    let mut chars = text.chars().peekable();
    let mut utf8_bytes = 0;
    while let Some(character) = chars.next() {
        if character == '\r' || character == '\n' {
            if character == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            utf8_bytes += 2;
            if utf8_bytes > MAX_UTF8 {
                return Err("遠端剪貼簿文字過大或格式無效。");
            }
            units.extend_from_slice(&[b'\r' as u16, b'\n' as u16]);
        } else {
            utf8_bytes += character.len_utf8();
            if utf8_bytes > MAX_UTF8 {
                return Err("遠端剪貼簿文字過大或格式無效。");
            }
            let mut encoded = [0u16; 2];
            units.extend_from_slice(character.encode_utf16(&mut encoded));
        }
    }
    units.push(0);
    Ok(units)
}

pub fn read_text() -> Result<String, &'static str> {
    // The clipboard remains open while the HGLOBAL is inspected and locked.
    if unsafe { OpenClipboard(ptr::null_mut()) } == 0 {
        return Err("無法開啟剪貼簿。");
    }
    let _clipboard = Clipboard;
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
    if handle.is_null() {
        return Err("剪貼簿沒有 Unicode 文字。");
    }
    let size = unsafe { GlobalSize(handle) };
    if size == 0 || size > MAX_UTF16_BYTES || size % 2 != 0 {
        return Err("剪貼簿文字過大或格式無效。");
    }
    let pointer = unsafe { GlobalLock(handle) };
    if pointer.is_null() {
        return Err("無法讀取剪貼簿。");
    }
    struct Locked(windows_sys::Win32::Foundation::HGLOBAL);
    impl Drop for Locked {
        fn drop(&mut self) {
            unsafe {
                GlobalUnlock(self.0);
            }
        }
    }
    let _lock = Locked(handle);
    // GlobalLock returns suitably aligned, stable memory until GlobalUnlock.
    let units = unsafe { slice::from_raw_parts(pointer.cast::<u16>(), size / 2) };
    decode(units)
}

fn decode(units: &[u16]) -> Result<String, &'static str> {
    if units.len() > MAX_UTF16_BYTES / 2 {
        return Err("剪貼簿文字過大或格式無效。");
    }
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .ok_or("剪貼簿文字未正確結束。")?;
    let mut text =
        String::from_utf16(&units[..end]).map_err(|_| "剪貼簿不是有效的 Unicode 文字。")?;
    if text.len() > MAX_UTF8 {
        text.zeroize();
        return Err("剪貼簿超過 256 KiB，請先縮短內容。");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "會覆蓋互動工作階段的剪貼簿，須先明確同意並設定測試環境變數"]
    fn interactive_windows_clipboard_round_trip() {
        assert_eq!(
            std::env::var("CRACAMOLE_ALLOW_INTERACTIVE_CLIPBOARD_TEST").as_deref(),
            Ok("1"),
            "此測試會覆蓋互動剪貼簿；須先設定 CRACAMOLE_ALLOW_INTERACTIVE_CLIPBOARD_TEST=1"
        );
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow};
        struct Owner(HWND);
        impl Drop for Owner {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
        let handle = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                10,
                10,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        assert!(!handle.is_null(), "無法建立隱藏的剪貼簿擁有者視窗");
        let owner = Owner(handle);
        const TEST_TEXT: &str = "cracamole 剪貼簿測試🙂\r\n第二行";
        write_text(owner.0, TEST_TEXT).expect("無法寫入測試文字");
        let read = Zeroizing::new(read_text().expect("無法讀回測試文字"));
        assert!(
            read.as_str() == TEST_TEXT,
            "剪貼簿內容未正確讀回或已被其他程式變更"
        );
    }

    #[test]
    fn unicode_clipboard_is_bounded_and_well_formed() {
        let text: Vec<u16> = "中文🙂".encode_utf16().chain([0]).collect();
        assert_eq!(decode(&text).unwrap(), "中文🙂");
        assert!(decode(&[0xd800, 0]).is_err());
        assert!(decode(&[65]).is_err());
        assert!(decode(&vec![65; MAX_UTF16_BYTES / 2 + 1]).is_err());
        let too_large: Vec<u16> = std::iter::repeat_n(0x0800, 100_000).chain([0]).collect();
        assert!(decode(&too_large).is_err());
        assert_eq!(decode(&encode("中文🙂").unwrap()).unwrap(), "中文🙂");
        assert_eq!(
            decode(&encode("甲\n乙\r丙\r\n丁🙂").unwrap()).unwrap(),
            "甲\r\n乙\r\n丙\r\n丁🙂"
        );
        assert_eq!(decode(&encode("").unwrap()).unwrap(), "");
        assert!(encode("a\0b").is_err());
        assert!(encode(&"x".repeat(MAX_UTF8 + 1)).is_err());
        let maximum = "x".repeat(MAX_UTF8);
        assert_eq!(decode(&encode(&maximum).unwrap()).unwrap(), maximum);
        let expanded = format!("{}\n", "x".repeat(MAX_UTF8 - 1));
        assert!(encode(&expanded).is_err());
        let fitting = format!("{}\n", "x".repeat(MAX_UTF8 - 2));
        assert_eq!(decode(&encode(&fitting).unwrap()).unwrap().len(), MAX_UTF8);
    }

    #[test]
    fn isolated_windows_clipboard_round_trip() {
        const CHILD: &str = "CRACAMOLE_CLIPBOARD_ISOLATED_TEST";
        if std::env::var_os(CHILD).is_none() {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "windows_clipboard::tests::isolated_windows_clipboard_round_trip",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "隔離剪貼簿測試失敗：\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let skipped = String::from_utf8_lossy(&output.stderr).contains("SKIP:");
            if skipped {
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
                assert_ne!(
                    std::env::var("CRACAMOLE_REQUIRE_ISOLATED_CLIPBOARD").as_deref(),
                    Ok("1"),
                    "嚴格驗證要求實際執行隔離剪貼簿讀寫，不能跳過"
                );
            } else {
                assert!(
                    String::from_utf8_lossy(&output.stdout).contains("CLIPBOARD_ROUND_TRIP_OK"),
                    "隔離剪貼簿子程序沒有完成讀寫驗證"
                );
            }
            return;
        }
        use windows_sys::Win32::System::StationsAndDesktops::{
            CloseDesktop, CloseWindowStation, CreateDesktopW, CreateWindowStationW,
            GetProcessWindowStation, GetThreadDesktop, SetProcessWindowStation, SetThreadDesktop,
        };
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, CWF_CREATE_ONLY,
        };

        // A separate process, window station and desktop ensure no operation
        // below can read or overwrite the interactive user's clipboard.
        unsafe {
            let old_station = GetProcessWindowStation();
            let old_desktop = GetThreadDesktop(GetCurrentThreadId());
            assert!(!old_station.is_null() && !old_desktop.is_null());
            // CWF_CREATE_ONLY ensures that neither attempt can attach to an
            // existing station (and therefore another user's clipboard).
            let mut station =
                CreateWindowStationW(ptr::null(), CWF_CREATE_ONLY, 0x0000_000f, ptr::null());
            if station.is_null() && std::io::Error::last_os_error().raw_os_error() == Some(183) {
                // NULL uses the logon-session name, which may already exist.
                // A named station requires an appropriately privileged token;
                // failure is safe and handled below without elevation.
                let suffix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let name: Vec<u16> =
                    format!("CracamoleClipboardTest-{}-{suffix}", std::process::id())
                        .encode_utf16()
                        .chain([0])
                        .collect();
                station =
                    CreateWindowStationW(name.as_ptr(), CWF_CREATE_ONLY, 0x0000_000f, ptr::null());
            }
            if station.is_null() {
                let error = std::io::Error::last_os_error();
                if matches!(error.raw_os_error(), Some(5 | 183)) {
                    eprintln!("SKIP: Windows 無法建立全新的隔離視窗站：{error}");
                    return;
                }
            }
            assert!(
                !station.is_null(),
                "CreateWindowStationW: {}",
                std::io::Error::last_os_error()
            );
            assert_ne!(
                SetProcessWindowStation(station),
                0,
                "SetProcessWindowStation: {}",
                std::io::Error::last_os_error()
            );
            let desktop_name: Vec<u16> = "clipboard-test".encode_utf16().chain([0]).collect();
            let desktop = CreateDesktopW(
                desktop_name.as_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                0x0000_01ff,
                ptr::null(),
            );
            assert!(
                !desktop.is_null(),
                "CreateDesktopW: {}",
                std::io::Error::last_os_error()
            );
            assert_ne!(
                SetThreadDesktop(desktop),
                0,
                "SetThreadDesktop: {}",
                std::io::Error::last_os_error()
            );
            let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
            let window = CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                10,
                10,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(
                !window.is_null(),
                "CreateWindowExW: {}",
                std::io::Error::last_os_error()
            );
            write_text(window, "中文🙂\n測試").unwrap();
            assert_eq!(read_text().unwrap(), "中文🙂\r\n測試");
            assert!(write_text(window, "bad\0text").is_err());
            assert!(write_text(window, &"x".repeat(MAX_UTF8 + 1)).is_err());
            assert_eq!(read_text().unwrap(), "中文🙂\r\n測試");
            write_text(window, "").unwrap();
            assert_eq!(read_text().unwrap(), "");
            assert_ne!(DestroyWindow(window), 0);
            assert_ne!(SetThreadDesktop(old_desktop), 0);
            assert_ne!(CloseDesktop(desktop), 0);
            assert_ne!(SetProcessWindowStation(old_station), 0);
            assert_ne!(CloseWindowStation(station), 0);
            println!("CLIPBOARD_ROUND_TRIP_OK");
        }
    }
}
