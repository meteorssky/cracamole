use crate::error::ParseError;
use crate::instruction::Instruction;
use crate::limits::{MAX_ELEMENTS, MAX_ELEMENT_LEN};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Reading decimal length digits.
    Length,
    /// Reading characters of element content.
    Content,
    /// Content finished; expecting ',' or ';'.
    Terminator,
    Failed,
}

/// Incremental parser. Feed arbitrary byte chunks; completed instructions are queued.
///
/// Element lengths count Unicode characters (not bytes), as in the C implementation.
#[derive(Debug)]
pub struct Parser {
    state: State,
    len_acc: usize,
    len_digits: usize,
    remaining_chars: usize,
    /// Continuation bytes still owed by the current multi-byte character.
    cont_bytes: usize,
    cur: Vec<u8>,
    elements: Vec<String>,
    ready: VecDeque<Instruction>,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Length,
            len_acc: 0,
            len_digits: 0,
            remaining_chars: 0,
            cont_bytes: 0,
            cur: Vec::new(),
            elements: Vec::new(),
            ready: VecDeque::new(),
        }
    }

    /// True if a partial instruction is buffered.
    pub fn is_mid_instruction(&self) -> bool {
        self.state != State::Length || self.len_digits > 0 || !self.elements.is_empty()
    }

    fn fail(&mut self, e: ParseError) -> Result<(), ParseError> {
        self.state = State::Failed;
        self.ready.clear();
        Err(e)
    }

    /// Feed bytes. After an error the parser is permanently failed.
    pub fn feed(&mut self, data: &[u8]) -> Result<(), ParseError> {
        for &b in data {
            match self.state {
                State::Failed => return Err(ParseError::Poisoned),
                State::Length => match b {
                    b'0'..=b'9' => {
                        self.len_acc = self.len_acc * 10 + (b - b'0') as usize;
                        self.len_digits += 1;
                        if self.len_acc > MAX_ELEMENT_LEN {
                            return self.fail(ParseError::ElementTooLong);
                        }
                    }
                    b'.' if self.len_digits > 0 => {
                        if self.elements.len() >= MAX_ELEMENTS {
                            return self.fail(ParseError::TooManyElements);
                        }
                        self.remaining_chars = self.len_acc;
                        self.cur.clear();
                        self.cont_bytes = 0;
                        self.state = if self.remaining_chars == 0 {
                            State::Terminator
                        } else {
                            State::Content
                        };
                    }
                    _ => return self.fail(ParseError::BadLength),
                },
                State::Content => {
                    if self.cont_bytes == 0 {
                        // Start of a character: determine its byte width.
                        self.cont_bytes = match b {
                            0x00..=0x7F => 0,
                            0xC2..=0xDF => 1,
                            0xE0..=0xEF => 2,
                            0xF0..=0xF4 => 3,
                            _ => return self.fail(ParseError::InvalidUtf8),
                        };
                        self.cur.push(b);
                        if self.cont_bytes == 0 {
                            self.remaining_chars -= 1;
                        }
                    } else {
                        if b & 0xC0 != 0x80 {
                            return self.fail(ParseError::InvalidUtf8);
                        }
                        self.cur.push(b);
                        self.cont_bytes -= 1;
                        if self.cont_bytes == 0 {
                            self.remaining_chars -= 1;
                        }
                    }
                    if self.remaining_chars == 0 && self.cont_bytes == 0 {
                        self.state = State::Terminator;
                    }
                }
                State::Terminator => {
                    if b != b',' && b != b';' {
                        return self.fail(ParseError::BadTerminator);
                    }
                    let s = match String::from_utf8(std::mem::take(&mut self.cur)) {
                        Ok(s) => s,
                        Err(_) => return self.fail(ParseError::InvalidUtf8),
                    };
                    self.elements.push(s);
                    self.len_acc = 0;
                    self.len_digits = 0;
                    self.state = State::Length;
                    if b == b';' {
                        let mut it = std::mem::take(&mut self.elements).into_iter();
                        let opcode = it.next().unwrap_or_default();
                        self.ready.push_back(Instruction {
                            opcode,
                            args: it.collect(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Pop the next completed instruction.
    pub fn next_instruction(&mut self) -> Option<Instruction> {
        self.ready.pop_front()
    }
}
