//! Telnet (RFC 854) stream decoder/encoder: IAC escaping and option negotiation commands.

pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
pub const SB: u8 = 250;
pub const SE: u8 = 240;

pub const OPT_ECHO: u8 = 1;
pub const OPT_SGA: u8 = 3;
pub const OPT_TTYPE: u8 = 24;
pub const OPT_NAWS: u8 = 31;
pub const OPT_NEW_ENVIRON: u8 = 39;
pub const OPT_MSSP: u8 = 70;
pub const OPT_COMPRESS2: u8 = 86;

/// Hard limit on a subnegotiation payload; larger ones are discarded.
pub const MAX_SB: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Application data (IAC IAC already unescaped).
    Data(Vec<u8>),
    Will(u8),
    Wont(u8),
    Do(u8),
    Dont(u8),
    /// Subnegotiation: option and payload (IAC IAC unescaped).
    Sb(u8, Vec<u8>),
    /// Any other two-byte command (NOP, GA, ...), ignored by callers.
    Command(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    Iac,
    /// After IAC WILL/WONT/DO/DONT, waiting for the option byte.
    Negotiate(u8),
    /// After IAC SB, waiting for the option byte.
    SbOption,
    SbData,
    SbIac,
}

#[derive(Debug)]
pub struct Decoder {
    state: State,
    sb_opt: u8,
    sb: Vec<u8>,
    sb_overflow: bool,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            state: State::Data,
            sb_opt: 0,
            sb: vec![],
            sb_overflow: false,
        }
    }

    /// Decode a chunk. Chunks may split commands at any byte.
    pub fn decode(&mut self, input: &[u8]) -> Vec<Event> {
        let mut events = vec![];
        let mut data = vec![];
        let flush = |data: &mut Vec<u8>, events: &mut Vec<Event>| {
            if !data.is_empty() {
                events.push(Event::Data(std::mem::take(data)));
            }
        };
        for &b in input {
            match self.state {
                State::Data => {
                    if b == IAC {
                        self.state = State::Iac;
                    } else {
                        data.push(b);
                    }
                }
                State::Iac => match b {
                    IAC => {
                        data.push(IAC);
                        self.state = State::Data;
                    }
                    WILL | WONT | DO | DONT => self.state = State::Negotiate(b),
                    SB => self.state = State::SbOption,
                    other => {
                        flush(&mut data, &mut events);
                        events.push(Event::Command(other));
                        self.state = State::Data;
                    }
                },
                State::Negotiate(cmd) => {
                    flush(&mut data, &mut events);
                    events.push(match cmd {
                        WILL => Event::Will(b),
                        WONT => Event::Wont(b),
                        DO => Event::Do(b),
                        _ => Event::Dont(b),
                    });
                    self.state = State::Data;
                }
                State::SbOption => {
                    self.sb_opt = b;
                    self.sb.clear();
                    self.sb_overflow = false;
                    self.state = State::SbData;
                }
                State::SbData => {
                    if b == IAC {
                        self.state = State::SbIac;
                    } else {
                        self.push_sb(b);
                    }
                }
                State::SbIac => match b {
                    IAC => {
                        self.push_sb(IAC);
                        self.state = State::SbData;
                    }
                    SE => {
                        flush(&mut data, &mut events);
                        if !self.sb_overflow {
                            events.push(Event::Sb(self.sb_opt, std::mem::take(&mut self.sb)));
                        }
                        self.sb.clear();
                        self.state = State::Data;
                    }
                    // Malformed: IAC followed by something else inside SB. Abort the subnegotiation.
                    _ => {
                        self.sb.clear();
                        self.state = State::Data;
                    }
                },
            }
        }
        flush(&mut data, &mut events);
        events
    }

    fn push_sb(&mut self, b: u8) {
        if self.sb.len() >= MAX_SB {
            self.sb_overflow = true;
        } else {
            self.sb.push(b);
        }
    }
}

/// Escape application data for sending: 0xFF -> 0xFF 0xFF.
pub fn escape_data(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for &b in data {
        out.push(b);
        if b == IAC {
            out.push(IAC);
        }
    }
    out
}

pub fn command(cmd: u8, opt: u8) -> Vec<u8> {
    vec![IAC, cmd, opt]
}

/// IAC SB opt payload IAC SE (payload is escaped).
pub fn subnegotiation(opt: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![IAC, SB, opt];
    out.extend(escape_data(payload));
    out.extend([IAC, SE]);
    out
}
