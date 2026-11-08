#![cfg(windows)]

use cracamole_desktop::geometry::centered_image_point;
use std::ptr;
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
    GetPixel, PatBlt, ReleaseDC, RestoreDC, SaveDC, SelectObject, BLACKNESS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SendMessageW, SetWindowPos, IMAGE_BITMAP, PRF_CLIENT,
    PRF_ERASEBKGND, STM_SETIMAGE, SWP_NOMOVE, SWP_NOZORDER, WM_PRINT, WS_CHILD,
};

#[test]
fn centered_coordinates_match_windows_static_control_drawing() {
    unsafe {
        // Hidden test windows and a private memory DC: no desktop capture or visible UI.
        let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
        let parent = CreateWindowExW(
            0,
            class.as_ptr(),
            ptr::null(),
            0,
            0,
            0,
            20,
            20,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        );
        assert!(!parent.is_null());
        // NWG ImageFrame forces SS_BITMAP | SS_CENTERIMAGE.
        let child = CreateWindowExW(
            0,
            class.as_ptr(),
            ptr::null(),
            WS_CHILD | 0x1000_0000 | 0x000e | 0x0200,
            0,
            0,
            5,
            5,
            parent,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        );
        assert!(!child.is_null());
        let pixels: [u32; 4] = [0x000000, 0x0000ff, 0x00ff00, 0xff0000];
        let source = CreateBitmap(2, 2, 1, 32, pixels.as_ptr().cast());
        assert!(!source.is_null());
        SendMessageW(child, STM_SETIMAGE, IMAGE_BITMAP as usize, source as isize);
        let screen = GetDC(ptr::null_mut());
        assert!(!screen.is_null());
        let dest_dc = CreateCompatibleDC(screen);
        let source_dc = CreateCompatibleDC(screen);
        let dest = CreateCompatibleBitmap(screen, 5, 5);
        assert!(!dest_dc.is_null() && !source_dc.is_null() && !dest.is_null());
        let old_dest = SelectObject(dest_dc, dest);
        let old_source = SelectObject(source_dc, source);
        assert!(!old_dest.is_null() && !old_source.is_null());
        let source_colors = [
            GetPixel(source_dc, 0, 0),
            GetPixel(source_dc, 1, 0),
            GetPixel(source_dc, 0, 1),
            GetPixel(source_dc, 1, 1),
        ];
        SelectObject(source_dc, old_source);
        for (width, height) in [(5u32, 5u32), (2, 2), (1, 1), (1, 5), (5, 1), (3, 2), (2, 3)] {
            assert_ne!(
                SetWindowPos(
                    child,
                    ptr::null_mut(),
                    0,
                    0,
                    width as i32,
                    height as i32,
                    SWP_NOMOVE | SWP_NOZORDER
                ),
                0
            );
            assert_ne!(PatBlt(dest_dc, 0, 0, 5, 5, BLACKNESS), 0);
            assert_ne!(
                GetPixel(dest_dc, 1, 1),
                u32::MAX,
                "memory DC not readable before drawing"
            );
            let saved_dc = SaveDC(dest_dc);
            assert_ne!(saved_dc, 0);
            SendMessageW(
                child,
                WM_PRINT,
                dest_dc as usize,
                (PRF_CLIENT | PRF_ERASEBKGND) as isize,
            );
            assert_ne!(RestoreDC(dest_dc, saved_dc), 0);
            for y in 0..height {
                for x in 0..width {
                    if let Some((remote_x, remote_y)) =
                        centered_image_point((width, height), (2, 2), (x as i32, y as i32), false)
                    {
                        assert_eq!(GetPixel(dest_dc, x as i32, y as i32), source_colors[(remote_y * 2 + remote_x) as usize], "viewport={width}x{height}, local={x},{y}, remote={remote_x},{remote_y}");
                    }
                }
            }
        }
        SelectObject(dest_dc, old_dest);
        DestroyWindow(child);
        DestroyWindow(parent);
        DeleteObject(source);
        DeleteObject(dest);
        DeleteDC(source_dc);
        DeleteDC(dest_dc);
        ReleaseDC(ptr::null_mut(), screen);
    }
}
