//! Client-to-server RFB messages and clipboard text conversion.

use crate::decode::REQUESTED_ENCODINGS;
use crate::format::PixelFormat;
use crate::settings::ClipboardEncoding;

pub fn set_pixel_format(format: &PixelFormat) -> Vec<u8> {
    let mut m = vec![0, 0, 0, 0];
    m.extend_from_slice(&format.wire());
    m
}

pub fn set_encodings() -> Vec<u8> {
    let mut m = vec![2, 0];
    m.extend_from_slice(&(REQUESTED_ENCODINGS.len() as u16).to_be_bytes());
    for e in REQUESTED_ENCODINGS {
        m.extend_from_slice(&e.to_be_bytes());
    }
    m
}

pub fn update_request(incremental: bool, width: u32, height: u32) -> Vec<u8> {
    let mut m = vec![3, u8::from(incremental), 0, 0, 0, 0];
    m.extend_from_slice(&(width.min(65535) as u16).to_be_bytes());
    m.extend_from_slice(&(height.min(65535) as u16).to_be_bytes());
    m
}

pub fn key_event(keysym: u32, pressed: bool) -> Vec<u8> {
    let mut m = vec![4, u8::from(pressed), 0, 0];
    m.extend_from_slice(&keysym.to_be_bytes());
    m
}

/// `mask` uses the Guacamole layout, which matches RFB: left, middle, right, wheel up, wheel down.
pub fn pointer_event(mask: u32, x: i32, y: i32, size: (u32, u32)) -> Vec<u8> {
    let cx = x.clamp(0, size.0.saturating_sub(1).min(65535) as i32) as u16;
    let cy = y.clamp(0, size.1.saturating_sub(1).min(65535) as i32) as u16;
    let mut m = vec![5, (mask & 0xff) as u8];
    m.extend_from_slice(&cx.to_be_bytes());
    m.extend_from_slice(&cy.to_be_bytes());
    m
}

/// Characters a pasted text may not carry into the remote session (other than TAB, LF, CR).
fn is_forbidden(c: char) -> bool {
    (c.is_control() && !matches!(c, '\t' | '\n' | '\r')) || c == '\u{7f}'
}

pub fn cut_text(text: &str, encoding: ClipboardEncoding) -> Vec<u8> {
    let mut data = Vec::with_capacity(text.len());
    for c in text.chars().filter(|c| !is_forbidden(*c)) {
        match encoding {
            ClipboardEncoding::Utf8 => {
                let mut b = [0u8; 4];
                data.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
            ClipboardEncoding::Latin1 => data.push(u8::try_from(u32::from(c)).unwrap_or(b'?')),
        }
    }
    let mut m = vec![6, 0, 0, 0];
    m.extend_from_slice(&(data.len() as u32).to_be_bytes());
    m.extend_from_slice(&data);
    m
}

/// Convert server clipboard bytes to text with `\n` line endings and no control characters.
pub fn clipboard_text(bytes: &[u8], encoding: ClipboardEncoding) -> String {
    let text: String = match encoding {
        ClipboardEncoding::Latin1 => bytes.iter().map(|b| char::from(*b)).collect(),
        ClipboardEncoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
    };
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| !is_forbidden(*c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_messages_have_the_documented_layout() {
        assert_eq!(set_pixel_format(&PixelFormat::rgb32(false)).len(), 20);
        let enc = set_encodings();
        assert_eq!(&enc[..4], &[2, 0, 0, REQUESTED_ENCODINGS.len() as u8]);
        assert_eq!(&enc[4..8], &16i32.to_be_bytes());
        assert_eq!(enc.len(), 4 + 4 * REQUESTED_ENCODINGS.len());
        assert_eq!(
            update_request(true, 1024, 768),
            [3, 1, 0, 0, 0, 0, 4, 0, 3, 0]
        );
        assert_eq!(key_event(0xff0d, true), [4, 1, 0, 0, 0, 0, 0xff, 0x0d]);
    }

    #[test]
    fn pointer_is_clamped_to_the_desktop() {
        assert_eq!(pointer_event(1, -5, 9999, (100, 50)), [5, 1, 0, 0, 0, 49]);
        assert_eq!(
            pointer_event(0x1f, 10, 20, (100, 50)),
            [5, 0x1f, 0, 10, 0, 20]
        );
    }

    #[test]
    fn clipboard_conversion_filters_controls_and_honours_the_encoding() {
        let m = cut_text("a\u{1b}[31mé€\n", ClipboardEncoding::Latin1);
        assert_eq!(&m[..4], &[6, 0, 0, 0]);
        assert_eq!(&m[8..], b"a[31m\xe9?\n");
        let m = cut_text("é€", ClipboardEncoding::Utf8);
        assert_eq!(&m[8..], "é€".as_bytes());
        assert_eq!(
            clipboard_text(b"x\r\ny\rz\x1b\xe9", ClipboardEncoding::Latin1),
            "x\ny\nzé"
        );
        assert_eq!(
            clipboard_text("héllo".as_bytes(), ClipboardEncoding::Utf8),
            "héllo"
        );
    }
}
