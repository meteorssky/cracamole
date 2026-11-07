//! Option negotiation policy, following the C implementation's option table:
//!
//! | option       | we offer (us) | they may enable (them) |
//! |--------------|---------------|------------------------|
//! | ECHO         | no            | yes (server echoes)    |
//! | TTYPE        | yes           | no                     |
//! | NAWS         | yes           | no                     |
//! | NEW-ENVIRON  | yes           | no                     |
//! | MSSP         | no            | yes (data ignored)     |
//! | COMPRESS2    | no            | **no** (deviation: MCCP2 decompression is not implemented) |
//!
//! Everything else is refused. Repeated requests for an option already in the requested state
//! are not answered again, which prevents negotiation loops.

use crate::codec::*;
use std::collections::HashSet;

pub struct Negotiator {
    terminal_type: String,
    username: Option<Vec<u8>>,
    cols: u16,
    rows: u16,
    us: HashSet<u8>,
    them: HashSet<u8>,
    /// Options we already refused, per direction (`true` = their DO request, `false` = their WILL).
    refused: HashSet<(bool, u8)>,
    /// Local echo is needed while the server does not echo (C default: on).
    pub local_echo: bool,
    pub naws_enabled: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reaction {
    /// Bytes to send to the server.
    pub send: Vec<u8>,
    /// Application data to hand to the terminal.
    pub data: Vec<u8>,
}

fn supports_us(opt: u8) -> bool {
    matches!(opt, OPT_TTYPE | OPT_NAWS | OPT_NEW_ENVIRON)
}

fn accepts_them(opt: u8) -> bool {
    matches!(opt, OPT_ECHO | OPT_MSSP)
}

impl Negotiator {
    pub fn set_username_bytes(&mut self, username: Option<Vec<u8>>) {
        self.username = username;
    }

    pub fn new(terminal_type: &str, username: Option<&str>, cols: u16, rows: u16) -> Self {
        Self {
            terminal_type: terminal_type.to_string(),
            username: username.map(|s| s.as_bytes().to_vec()),
            cols,
            rows,
            us: HashSet::new(),
            them: HashSet::new(),
            refused: HashSet::new(),
            local_echo: true,
            naws_enabled: false,
        }
    }

    fn naws(&self) -> Vec<u8> {
        let mut p = vec![];
        p.extend(self.cols.to_be_bytes());
        p.extend(self.rows.to_be_bytes());
        subnegotiation(OPT_NAWS, &p)
    }

    /// Window size changed; returns bytes to send (empty unless the server asked for NAWS).
    pub fn resize(&mut self, cols: u16, rows: u16) -> Vec<u8> {
        self.cols = cols;
        self.rows = rows;
        if self.naws_enabled {
            self.naws()
        } else {
            vec![]
        }
    }

    pub fn handle(&mut self, ev: Event) -> Reaction {
        let mut r = Reaction::default();
        match ev {
            Event::Data(d) => r.data = d,
            Event::Command(_) => {}
            Event::Do(opt) => {
                if supports_us(opt) {
                    if self.us.insert(opt) {
                        r.send.extend(command(WILL, opt));
                        if opt == OPT_NAWS {
                            self.naws_enabled = true;
                            r.send.extend(self.naws());
                        }
                    }
                } else if self.refused.insert((true, opt)) {
                    r.send.extend(command(WONT, opt));
                }
            }
            Event::Dont(opt) => {
                if self.us.remove(&opt) {
                    r.send.extend(command(WONT, opt));
                    if opt == OPT_NAWS {
                        self.naws_enabled = false;
                    }
                }
            }
            Event::Will(opt) => {
                if accepts_them(opt) {
                    if self.them.insert(opt) {
                        r.send.extend(command(DO, opt));
                        if opt == OPT_ECHO {
                            self.local_echo = false; // remote will echo
                        }
                    }
                } else if self.refused.insert((false, opt)) {
                    r.send.extend(command(DONT, opt));
                }
            }
            Event::Wont(opt) => {
                if self.them.remove(&opt) {
                    r.send.extend(command(DONT, opt));
                }
                if opt == OPT_ECHO {
                    self.local_echo = true; // remote won't echo
                }
            }
            Event::Sb(OPT_TTYPE, p) => {
                // SEND = 1 -> IS = 0 <type>
                if p.first() == Some(&1) {
                    let mut body = vec![0];
                    body.extend(self.terminal_type.as_bytes());
                    r.send.extend(subnegotiation(OPT_TTYPE, &body));
                }
            }
            Event::Sb(OPT_NEW_ENVIRON, p) => {
                // SEND = 1. Only a request for the whole environment (empty list) gets USER.
                if p.first() == Some(&1) && p.len() == 1 {
                    let mut body = vec![0]; // IS
                    if let Some(u) = &self.username {
                        body.push(0); // VAR
                        body.extend(b"USER");
                        body.push(1); // VALUE
                        body.extend(u);
                    }
                    r.send.extend(subnegotiation(OPT_NEW_ENVIRON, &body));
                }
            }
            Event::Sb(..) => {}
        }
        r
    }
}
