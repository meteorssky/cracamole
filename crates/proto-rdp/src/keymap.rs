//! X11 keysym -> RDP scancode (PC/AT set 1) for a US layout.
//! Printable characters outside the table fall back to Unicode key events.

use ironrdp_input::Scancode;

pub enum Key {
    Scan(Scancode),
    Unicode(char),
}

const fn s(code: u8) -> Key {
    Key::Scan(Scancode::from_u8(false, code))
}
const fn e(code: u8) -> Key {
    Key::Scan(Scancode::from_u8(true, code))
}

const KEYPAD: [u8; 10] = [0x52, 0x4f, 0x50, 0x51, 0x4b, 0x4c, 0x4d, 0x47, 0x48, 0x49];

/// Shift state is sent as separate Shift key events by the browser, so shifted and
/// unshifted symbols map to the same physical key.
pub fn map(keysym: u32) -> Option<Key> {
    let k = match keysym {
        0xff08 => s(0x0e), // BackSpace
        0xff09 => s(0x0f), // Tab
        0xff0d => s(0x1c), // Return
        0xff1b => s(0x01), // Escape
        0xff14 => s(0x46), // Scroll_Lock
        0xff50 => e(0x47), // Home
        0xff51 => e(0x4b), // Left
        0xff52 => e(0x48), // Up
        0xff53 => e(0x4d), // Right
        0xff54 => e(0x50), // Down
        0xff55 => e(0x49), // Page_Up
        0xff56 => e(0x51), // Page_Down
        0xff57 => e(0x4f), // End
        0xff63 => e(0x52), // Insert
        0xffff => e(0x53), // Delete
        0xff7f => s(0x45), // Num_Lock
        0xff8d => e(0x1c), // KP_Enter
        0xffaa => s(0x37), // KP_Multiply
        0xffab => s(0x4e), // KP_Add
        0xffad => s(0x4a), // KP_Subtract
        0xffae => s(0x53), // KP_Decimal
        0xffaf => e(0x35), // KP_Divide
        0xffb0..=0xffb9 => s(KEYPAD[(keysym - 0xffb0) as usize]),
        0xffbe..=0xffc7 => s(0x3b + (keysym - 0xffbe) as u8), // F1..F10
        0xffc8 => s(0x57),                                    // F11
        0xffc9 => s(0x58),                                    // F12
        0xffe1 => s(0x2a),                                    // Shift_L
        0xffe2 => s(0x36),                                    // Shift_R
        0xffe3 => s(0x1d),                                    // Control_L
        0xffe4 => e(0x1d),                                    // Control_R
        0xffe5 => s(0x3a),                                    // Caps_Lock
        0xffe9 => s(0x38),                                    // Alt_L
        0xffea => e(0x38),                                    // Alt_R
        0xfe03 => e(0x38),                                    // ISO_Level3_Shift (AltGr)
        0xffeb => e(0x5b),                                    // Super_L
        0xffec => e(0x5c),                                    // Super_R
        0xff67 => e(0x5d),                                    // Menu
        _ => return printable(keysym),
    };
    Some(k)
}

/// True for printable ASCII keysyms that need Shift on a US keyboard (A-Z and shifted symbols).
pub fn needs_shift(keysym: u32) -> bool {
    match char::from_u32(keysym) {
        Some(c) if keysym <= 0x7e => c.is_ascii_uppercase() || "!@#$%^&*()_+{}|:\"<>?~".contains(c),
        _ => false,
    }
}

fn printable(keysym: u32) -> Option<Key> {
    let c = match keysym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(keysym)?,
        0x0100_0000..=0x0110_ffff => char::from_u32(keysym - 0x0100_0000)?,
        _ => return None,
    };
    let lower = c.to_ascii_lowercase();
    let code = match lower {
        ' ' => 0x39,
        'a'..='z' => {
            const ROWS: [(&str, u8); 3] =
                [("qwertyuiop", 0x10), ("asdfghjkl", 0x1e), ("zxcvbnm", 0x2c)];
            let (row, base) = ROWS.iter().find(|(r, _)| r.contains(lower))?;
            base + row.find(lower)? as u8
        }
        '1'..='9' => 0x02 + (lower as u8 - b'1'),
        '!' => 0x02,
        '@' => 0x03,
        '#' => 0x04,
        '$' => 0x05,
        '%' => 0x06,
        '^' => 0x07,
        '&' => 0x08,
        '*' => 0x09,
        '(' => 0x0a,
        '0' | ')' => 0x0b,
        '-' | '_' => 0x0c,
        '=' | '+' => 0x0d,
        '[' | '{' => 0x1a,
        ']' | '}' => 0x1b,
        ';' | ':' => 0x27,
        '\'' | '"' => 0x28,
        '`' | '~' => 0x29,
        '\\' | '|' => 0x2b,
        ',' | '<' => 0x33,
        '.' | '>' => 0x34,
        '/' | '?' => 0x35,
        _ => return Some(Key::Unicode(c)),
    };
    Some(s(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(k: Option<Key>) -> (bool, u8) {
        match k {
            Some(Key::Scan(s)) => s.as_u8(),
            _ => panic!("expected scancode"),
        }
    }

    #[test]
    fn letters_and_shifted_letters_share_a_scancode() {
        assert_eq!(scan(map(b'a' as u32)), (false, 0x1e));
        assert_eq!(scan(map(b'A' as u32)), (false, 0x1e));
        assert_eq!(scan(map(b'z' as u32)), (false, 0x2c));
        assert_eq!(scan(map(b'p' as u32)), (false, 0x19));
    }

    #[test]
    fn digits_symbols_and_specials() {
        assert_eq!(scan(map(b'1' as u32)), (false, 0x02));
        assert_eq!(scan(map(b'!' as u32)), (false, 0x02));
        assert_eq!(scan(map(b'0' as u32)), (false, 0x0b));
        assert_eq!(scan(map(0x20)), (false, 0x39));
        assert_eq!(scan(map(0xff0d)), (false, 0x1c));
        assert_eq!(scan(map(0xff51)), (true, 0x4b)); // Left is an extended key
        assert_eq!(scan(map(0xffbe)), (false, 0x3b)); // F1
        assert_eq!(scan(map(0xffc7)), (false, 0x44)); // F10
    }

    #[test]
    fn shifted_symbols_are_detected() {
        for c in "AZ!@#$%^&*()_+{}|:<>?~\"".chars() {
            assert!(needs_shift(c as u32), "{c}");
        }
        for c in r"az019-=[];',./` ".chars() {
            assert!(!needs_shift(c as u32), "{c}");
        }
        assert!(!needs_shift(0xffe1));
    }

    #[test]
    fn non_ascii_falls_back_to_unicode_and_unknown_is_none() {
        assert!(matches!(map(0x0100_4e2d), Some(Key::Unicode('中'))));
        assert!(map(0xfd00).is_none());
    }
}
