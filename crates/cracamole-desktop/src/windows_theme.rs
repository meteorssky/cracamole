//! 跟隨 Windows 深淺色設定的外觀：深色標題列、控制項主題與配色。
//! 色彩取自 NSH／shadcn 的 neutral（zinc）token；`CRACAMOLE_THEME=dark|light` 可強制指定。

use native_windows_gui as nwg;
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    CreateSolidBrush, FillRect, GetSysColorBrush, SetBkColor, SetBkMode, SetTextColor, HBRUSH,
};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::UI::Controls::SetWindowTheme;
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetClassNameW, GetClientRect};

// COLORREF 為 0x00BBGGRR。
const DARK_BG: u32 = 0x001B_1818; // #18181B
const DARK_INPUT: u32 = 0x002A_2727; // #27272A
const DARK_FG: u32 = 0x00FA_FAFA; // #FAFAFA
const DARK_MUTED: u32 = 0x00AA_A1A1; // #A1A1AA
const LIGHT_MUTED: u32 = 0x005B_5252; // #52525B

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// 是否使用深色外觀。
pub fn is_dark() -> bool {
    static DARK: OnceLock<bool> = OnceLock::new();
    *DARK.get_or_init(|| match std::env::var("CRACAMOLE_THEME").as_deref() {
        Ok("dark") => true,
        Ok("light") => false,
        _ => system_prefers_dark(),
    })
}

fn system_prefers_dark() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("AppsUseLightTheme");
    let mut data: u32 = 1;
    let mut size = 4u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        )
    };
    status == 0 && data == 0
}

fn brush(color: u32, cell: &'static OnceLock<isize>) -> HBRUSH {
    *cell.get_or_init(|| unsafe { CreateSolidBrush(color) } as isize) as HBRUSH
}

fn bg_brush() -> HBRUSH {
    static CELL: OnceLock<isize> = OnceLock::new();
    brush(DARK_BG, &CELL)
}

fn input_brush() -> HBRUSH {
    static CELL: OnceLock<isize> = OnceLock::new();
    brush(DARK_INPUT, &CELL)
}

unsafe extern "system" fn theme_child(child: HWND, _: LPARAM) -> i32 {
    let mut class = [0u16; 32];
    let length = unsafe { GetClassNameW(child, class.as_mut_ptr(), 32) } as usize;
    let combo = String::from_utf16_lossy(&class[..length]).eq_ignore_ascii_case("ComboBox");
    let name = wide(if combo {
        "DarkMode_CFD"
    } else {
        "DarkMode_Explorer"
    });
    unsafe { SetWindowTheme(child, name.as_ptr(), std::ptr::null()) };
    1
}

/// 垂直置中標籤需自行填色的背景；淺色時維持 NWG 預設。
pub fn label_background() -> Option<[u8; 3]> {
    is_dark().then_some([0x18, 0x18, 0x1B])
}

/// 為已建立的視窗與所有子控制項套用深色主題；淺色時不做任何事。
pub fn style_window(window: &nwg::Window) {
    if !is_dark() {
        return;
    }
    let Some(hwnd) = window.handle.hwnd() else {
        return;
    };
    unsafe {
        let enabled: i32 = 1;
        // DWMWA_USE_IMMERSIVE_DARK_MODE
        DwmSetWindowAttribute(hwnd as _, 20, &enabled as *const i32 as *const _, 4);
        EnumChildWindows(hwnd as _, Some(theme_child), 0);
    }
}

/// 綁定繪色訊息：深色時繪製背景與各類控制項；淺色時僅將 `muted` 標籤改為次要文字色。
pub fn install(
    window: &nwg::Window,
    muted: nwg::ControlHandle,
) -> Result<nwg::RawEventHandler, nwg::NwgError> {
    nwg::bind_raw_event_handler(
        &window.handle,
        0x10002,
        move |hwnd, message, wparam, lparam| {
            let dark = is_dark();
            let is_muted = muted.hwnd().is_some_and(|handle| handle as isize == lparam);
            unsafe {
                match message {
                    // WM_ERASEBKGND
                    0x0014 if dark => {
                        let mut rect: RECT = std::mem::zeroed();
                        GetClientRect(hwnd as _, &mut rect);
                        FillRect(wparam as _, &rect, bg_brush());
                        Some(1)
                    }
                    // WM_CTLCOLORSTATIC / WM_CTLCOLORBTN
                    0x0138 | 0x0135 if dark => {
                        SetBkMode(wparam as _, 1);
                        SetTextColor(wparam as _, if is_muted { DARK_MUTED } else { DARK_FG });
                        Some(bg_brush() as isize)
                    }
                    // WM_CTLCOLOREDIT / WM_CTLCOLORLISTBOX
                    0x0133 | 0x0134 if dark => {
                        SetBkColor(wparam as _, DARK_INPUT);
                        SetTextColor(wparam as _, DARK_FG);
                        Some(input_brush() as isize)
                    }
                    0x0138 if is_muted => {
                        SetBkMode(wparam as _, 1);
                        SetTextColor(wparam as _, LIGHT_MUTED);
                        Some(GetSysColorBrush(15) as isize)
                    }
                    _ => None,
                }
            }
        },
    )
}
