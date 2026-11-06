//! Keysym -> terminal input bytes. Behaviour follows the C implementation
//! (`terminal.c`, `__guac_terminal_send_key`), verified by reading the reference source.

#[derive(Debug, Clone, Copy, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Mods {
    fn any(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.meta
    }
    /// xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4) + meta(8).
    fn param(&self) -> u32 {
        1 + self.shift as u32 + 2 * self.alt as u32 + 4 * self.ctrl as u32 + 8 * self.meta as u32
    }
}

/// Terminal-side modes that change key encoding.
#[derive(Debug, Clone, Copy)]
pub struct KeyModes {
    /// DECCKM application cursor keys.
    pub app_cursor: bool,
    /// DECKPAM application keypad.
    pub app_keypad: bool,
    /// `func-keys-and-keypad=vt100`.
    pub vt100_func_keys: bool,
    /// Linux-console style Home/End (C default: true).
    pub linux_console_keys: bool,
    /// Byte sent for Backspace (C default 127).
    pub backspace: u8,
    /// Rows on screen (page size for Shift+PgUp/PgDn).
    pub rows: usize,
    pub max_scrollback: usize,
}

impl Default for KeyModes {
    fn default() -> Self {
        Self {
            app_cursor: false,
            app_keypad: false,
            vt100_func_keys: false,
            linux_console_keys: true,
            backspace: 127,
            rows: 24,
            max_scrollback: 1000,
        }
    }
}

/// What a key event asks the terminal to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct KeyOutcome {
    pub send: Vec<u8>,
    /// Positive = scroll back into history, negative = toward the present.
    pub scroll: i64,
    /// Return to the live screen before applying the key.
    pub reset_scroll: bool,
    /// Ctrl+Shift+V / Cmd+V: paste the local clipboard.
    pub paste: bool,
}

#[derive(Debug, Default)]
pub struct Keyboard {
    pub mods: Mods,
}

mod ks {
    pub const SHIFT_L: u32 = 0xFFE1;
    pub const SHIFT_R: u32 = 0xFFE2;
    pub const CTRL_L: u32 = 0xFFE3;
    pub const CTRL_R: u32 = 0xFFE4;
    pub const CAPS: u32 = 0xFFE5;
    pub const COMMAND_L: u32 = 0xFFE7;
    pub const COMMAND_R: u32 = 0xFFE8;
    pub const ALT_L: u32 = 0xFFE9;
    pub const ALT_R: u32 = 0xFFEA;
    pub const SUPER_L: u32 = 0xFFEB;
    pub const SUPER_R: u32 = 0xFFEC;
    pub const OPTION_R: u32 = 0xFFEE;
    pub const ALTGR: u32 = 0xFE03;
    pub const NUM_LOCK: u32 = 0xFF7F;
    pub const SCROLL_LOCK: u32 = 0xFF14;

    pub const BACKSPACE: u32 = 0xFF08;
    pub const TAB: u32 = 0xFF09;
    pub const LF: u32 = 0xFF0A;
    pub const ENTER: u32 = 0xFF0D;
    pub const ESCAPE: u32 = 0xFF1B;
    pub const HOME: u32 = 0xFF50;
    pub const LEFT: u32 = 0xFF51;
    pub const UP: u32 = 0xFF52;
    pub const RIGHT: u32 = 0xFF53;
    pub const DOWN: u32 = 0xFF54;
    pub const PAGE_UP: u32 = 0xFF55;
    pub const PAGE_DOWN: u32 = 0xFF56;
    pub const END: u32 = 0xFF57;
    pub const INSERT: u32 = 0xFF63;
    pub const KP_TAB: u32 = 0xFF89;
    pub const KP_ENTER: u32 = 0xFF8D;
    pub const KP_F1: u32 = 0xFF91;
    pub const KP_F4: u32 = 0xFF94;
    pub const KP_HOME: u32 = 0xFF95;
    pub const KP_LEFT: u32 = 0xFF96;
    pub const KP_UP: u32 = 0xFF97;
    pub const KP_RIGHT: u32 = 0xFF98;
    pub const KP_DOWN: u32 = 0xFF99;
    pub const KP_PAGE_UP: u32 = 0xFF9A;
    pub const KP_PAGE_DOWN: u32 = 0xFF9B;
    pub const KP_END: u32 = 0xFF9C;
    pub const KP_INSERT: u32 = 0xFF9E;
    pub const KP_DELETE: u32 = 0xFF9F;
    pub const F1: u32 = 0xFFBE;
    pub const F12: u32 = 0xFFC9;
    pub const DELETE: u32 = 0xFFFF;
}

fn is_modifier(k: u32) -> bool {
    (ks::SHIFT_L..=ks::OPTION_R).contains(&k) || k == ks::ALTGR
}

fn is_lock(k: u32) -> bool {
    matches!(k, ks::CAPS | ks::NUM_LOCK | ks::SCROLL_LOCK)
}

fn arrow_suffix(k: u32) -> Option<char> {
    Some(match k {
        ks::LEFT | ks::KP_LEFT => 'D',
        ks::UP | ks::KP_UP => 'A',
        ks::RIGHT | ks::KP_RIGHT => 'C',
        ks::DOWN | ks::KP_DOWN => 'B',
        _ => return None,
    })
}

fn is_function(k: u32) -> bool {
    (ks::F1..=ks::F12).contains(&k) || (ks::KP_F1..=ks::KP_F4).contains(&k)
}

fn is_editing(k: u32) -> bool {
    matches!(
        k,
        ks::HOME
            | ks::KP_HOME
            | ks::END
            | ks::KP_END
            | ks::INSERT
            | ks::KP_INSERT
            | ks::DELETE
            | ks::KP_DELETE
            | ks::PAGE_UP
            | ks::KP_PAGE_UP
            | ks::PAGE_DOWN
            | ks::KP_PAGE_DOWN
    )
}

fn modified_arrow(m: &Mods, k: u32) -> Vec<u8> {
    match arrow_suffix(k) {
        Some(s) => format!("\x1b[1;{}{}", m.param(), s).into_bytes(),
        None => vec![],
    }
}

fn modified_editing(m: &Mods, k: u32, linux: bool) -> Vec<u8> {
    let p = m.param();
    let s = match k {
        ks::HOME | ks::KP_HOME => {
            if linux {
                format!("\x1b[1;{p}~")
            } else {
                format!("\x1b[1;{p}H")
            }
        }
        ks::END | ks::KP_END => {
            if linux {
                format!("\x1b[4;{p}~")
            } else {
                format!("\x1b[1;{p}F")
            }
        }
        ks::INSERT | ks::KP_INSERT => format!("\x1b[2;{p}~"),
        ks::DELETE | ks::KP_DELETE => format!("\x1b[3;{p}~"),
        ks::PAGE_UP | ks::KP_PAGE_UP => format!("\x1b[5;{p}~"),
        ks::PAGE_DOWN | ks::KP_PAGE_DOWN => format!("\x1b[6;{p}~"),
        _ => return vec![],
    };
    s.into_bytes()
}

fn modified_function(m: &Mods, k: u32) -> Vec<u8> {
    let p = m.param();
    let letter = match k {
        ks::F1 | ks::KP_F1 => Some('P'),
        0xFFBF | 0xFF92 => Some('Q'),
        0xFFC0 | 0xFF93 => Some('R'),
        0xFFC1 | ks::KP_F4 => Some('S'),
        _ => None,
    };
    if let Some(l) = letter {
        return format!("\x1b[1;{p}{l}").into_bytes();
    }
    // Note the intentional gap at 22 (as in xterm) and that unmodified F11/F12 differ (see below).
    let n = match k {
        0xFFC2 => 15,
        0xFFC3 => 17,
        0xFFC4 => 18,
        0xFFC5 => 19,
        0xFFC6 => 20,
        0xFFC7 => 21,
        0xFFC8 => 23,
        0xFFC9 => 24,
        _ => return vec![],
    };
    format!("\x1b[{n};{p}~").into_bytes()
}

fn utf8(cp: u32) -> Vec<u8> {
    char::from_u32(cp)
        .map(|c| c.to_string().into_bytes())
        .unwrap_or_default()
}

impl Keyboard {
    pub fn process(&mut self, keysym: u32, pressed: bool, modes: &KeyModes) -> KeyOutcome {
        let mut out = KeyOutcome::default();

        // Modifier bookkeeping. AltGr/Option are intentionally not terminal-visible modifiers.
        match keysym {
            ks::CTRL_L | ks::CTRL_R => {
                self.mods.ctrl = pressed;
                return out;
            }
            ks::COMMAND_L | ks::COMMAND_R | ks::SUPER_L | ks::SUPER_R => {
                self.mods.meta = pressed;
                return out;
            }
            ks::ALT_L | ks::ALT_R => {
                self.mods.alt = pressed;
                return out;
            }
            ks::SHIFT_L | ks::SHIFT_R => {
                self.mods.shift = pressed;
                return out;
            }
            _ => {}
        }
        if !pressed {
            return out;
        }
        let m = self.mods;

        if (keysym == 'V' as u32 && m.ctrl) || (keysym == 'v' as u32 && m.meta) {
            out.paste = true;
            return out;
        }
        if m.shift && keysym == ks::TAB {
            out.send = b"\x1b[Z".to_vec();
            return out;
        }
        // Copy shortcuts are swallowed: selecting already copies.
        if (keysym == 'C' as u32 && m.ctrl) || (keysym == 'c' as u32 && m.meta) {
            return out;
        }

        if m.shift {
            let page = modes.rows as i64;
            let scroll = match keysym {
                ks::HOME | ks::KP_HOME => Some(modes.max_scrollback as i64),
                ks::END | ks::KP_END => Some(-(modes.max_scrollback as i64)),
                ks::PAGE_UP | ks::KP_PAGE_UP => Some(page),
                ks::PAGE_DOWN | ks::KP_PAGE_DOWN => Some(-page),
                ks::UP | ks::KP_UP => Some(1),
                ks::DOWN | ks::KP_DOWN => Some(-1),
                _ => None,
            };
            if let Some(s) = scroll {
                out.scroll = s;
                return out;
            }
        }

        if !is_modifier(keysym) && !is_lock(keysym) {
            out.reset_scroll = true;
        }

        if m.alt {
            out.send.push(0x1b);
        }

        if m.ctrl {
            let data: Option<u8> = match keysym {
                k if (b'@' as u32..=b'_' as u32).contains(&k) => Some((k - b'@' as u32) as u8),
                k if (b'a' as u32..=b'z' as u32).contains(&k) => Some((k - b'a' as u32 + 1) as u8),
                k if k == '?' as u32 => Some(0x7F),
                k if k == '2' as u32 => Some(0),
                k if (b'3' as u32..=b'7' as u32).contains(&k) => {
                    Some((k - b'3' as u32 + 0x1B) as u8)
                }
                ks::LEFT | ks::KP_LEFT => {
                    out.send.extend_from_slice(b"\x1b[1;5D");
                    return out;
                }
                ks::RIGHT | ks::KP_RIGHT => {
                    out.send.extend_from_slice(b"\x1b[1;5C");
                    return out;
                }
                ks::BACKSPACE => Some(23),
                ks::DELETE | ks::KP_DELETE => {
                    out.send.extend_from_slice(b"\x1bD");
                    return out;
                }
                _ => {
                    if arrow_suffix(keysym).is_some() {
                        out.send.extend(modified_arrow(&m, keysym));
                    } else if is_function(keysym) {
                        out.send.extend(modified_function(&m, keysym));
                    } else if is_editing(keysym) {
                        out.send
                            .extend(modified_editing(&m, keysym, modes.linux_console_keys));
                    }
                    return out;
                }
            };
            if let Some(b) = data {
                out.send.push(b);
            }
            return out;
        }

        // Printable: Latin-1 and Unicode keysyms (0x01000000 + code point).
        if keysym <= 0xFF || (0x0100_0000..=0x0110_FFFF).contains(&keysym) {
            let cp = if keysym <= 0xFF {
                keysym
            } else {
                keysym - 0x0100_0000
            };
            out.send.extend(utf8(cp));
            return out;
        }

        // Typeable keypad keys.
        if (0xFFAA..=0xFFB9).contains(&keysym) {
            let value = (keysym - 0xFF80) as u8;
            if modes.vt100_func_keys && modes.app_keypad {
                match value {
                    b'0'..=b'9' => out
                        .send
                        .extend_from_slice(&[0x1b, b'O', b'p' + (value - b'0')]),
                    b'-' => out.send.extend_from_slice(b"\x1bOm"),
                    b'.' => out.send.extend_from_slice(b"\x1bOn"),
                    b'*' => out.send.extend_from_slice(b"\x1bOl"),
                    b'+' => out.send.extend_from_slice(b"\x1bOM"),
                    other => out.send.push(other),
                }
            } else {
                out.send.push(value);
            }
            return out;
        }

        // Non-printable keys.
        let s: &[u8] = match keysym {
            ks::BACKSPACE => {
                out.send.push(modes.backspace);
                return out;
            }
            ks::TAB | ks::KP_TAB => b"\x09",
            ks::LF => b"\x0A",
            ks::ENTER | ks::KP_ENTER => b"\x0D",
            ks::ESCAPE => b"\x1b",
            // Home is always `ESC [ 1 ~` (the linux_console_keys branch for Home is dead code in C).
            ks::HOME | ks::KP_HOME => b"\x1b[1~",
            ks::END | ks::KP_END => {
                if modes.linux_console_keys {
                    b"\x1b[4~"
                } else {
                    b"\x1b[F"
                }
            }
            _ if m.any() && arrow_suffix(keysym).is_some() => {
                out.send.extend(modified_arrow(&m, keysym));
                return out;
            }
            ks::LEFT | ks::KP_LEFT => {
                if modes.app_cursor {
                    b"\x1bOD"
                } else {
                    b"\x1b[D"
                }
            }
            ks::UP | ks::KP_UP => {
                if modes.app_cursor {
                    b"\x1bOA"
                } else {
                    b"\x1b[A"
                }
            }
            ks::RIGHT | ks::KP_RIGHT => {
                if modes.app_cursor {
                    b"\x1bOC"
                } else {
                    b"\x1b[C"
                }
            }
            ks::DOWN | ks::KP_DOWN => {
                if modes.app_cursor {
                    b"\x1bOB"
                } else {
                    b"\x1b[B"
                }
            }
            ks::PAGE_UP | ks::KP_PAGE_UP => b"\x1b[5~",
            ks::PAGE_DOWN | ks::KP_PAGE_DOWN => b"\x1b[6~",
            ks::INSERT | ks::KP_INSERT => b"\x1b[2~",
            ks::DELETE | ks::KP_DELETE => b"\x1b[3~",
            // F1-F4 / KP_F1-F4
            ks::F1 | ks::KP_F1 => {
                if modes.vt100_func_keys {
                    b"\x1bOP"
                } else {
                    b"\x1b[[A"
                }
            }
            0xFFBF | 0xFF92 => {
                if modes.vt100_func_keys {
                    b"\x1bOQ"
                } else {
                    b"\x1b[[B"
                }
            }
            0xFFC0 | 0xFF93 => {
                if modes.vt100_func_keys {
                    b"\x1bOR"
                } else {
                    b"\x1b[[C"
                }
            }
            0xFFC1 | ks::KP_F4 => {
                if modes.vt100_func_keys {
                    b"\x1bOS"
                } else {
                    b"\x1b[[D"
                }
            }
            0xFFC2 => {
                if modes.vt100_func_keys {
                    b"\x1b[15~"
                } else {
                    b"\x1b[[E"
                }
            }
            0xFFC3 => b"\x1b[17~",
            0xFFC4 => b"\x1b[18~",
            0xFFC5 => b"\x1b[19~",
            0xFFC6 => b"\x1b[20~",
            0xFFC7 => b"\x1b[21~",
            // C quirk preserved: unmodified F11/F12 are 22/23 (modified use 23/24).
            0xFFC8 => b"\x1b[22~",
            0xFFC9 => b"\x1b[23~",
            _ => {
                return out; // unknown keysym: ignored
            }
        };
        out.send.extend_from_slice(s);
        out
    }
}
