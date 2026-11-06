//! Automatic login: watch remote output for prompts (as the C `guac_telnet_search`).

use guac_protocol::Status;
use guac_terminal::{FilterOutput, RemoteFilter};
use regex::{Regex, RegexBuilder};

/// Longest line kept for matching (C: 1024 bytes).
const MAX_LINE: usize = 1023;

pub fn compile(pattern: &str) -> Result<Regex, regex::Error> {
    // C uses POSIX extended, case-insensitive.
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .multi_line(true)
        .build()
}

pub struct LoginFilter {
    username: Option<String>,
    password: Option<String>,
    username_re: Option<Regex>,
    password_re: Option<Regex>,
    success_re: Option<Regex>,
    failure_re: Option<Regex>,
    line: Vec<u8>,
}

impl LoginFilter {
    pub fn new(
        username: Option<String>,
        username_re: Option<Regex>,
        password: Option<String>,
        password_re: Option<Regex>,
        success_re: Option<Regex>,
        failure_re: Option<Regex>,
    ) -> Self {
        Self {
            username,
            password,
            username_re,
            password_re,
            success_re,
            failure_re,
            line: vec![],
        }
    }

    /// Whether this filter has any work to do at all.
    pub fn is_active(&self) -> bool {
        self.username_re.is_some()
            || self.password_re.is_some()
            || self.success_re.is_some()
            || self.failure_re.is_some()
    }

    fn search_line(&mut self, line: &str, out: &mut FilterOutput) {
        if let Some(re) = &self.username_re {
            if re.is_match(line) {
                if let Some(u) = &self.username {
                    out.send.extend_from_slice(u.as_bytes());
                    out.send.push(b'\r');
                }
                self.username_re = None;
            }
        }
        if let Some(re) = &self.password_re {
            if re.is_match(line) {
                if let Some(p) = &self.password {
                    out.send.extend_from_slice(p.as_bytes());
                    out.send.push(b'\r');
                }
                // After the password is sent neither prompt is searched for again.
                self.username_re = None;
                self.password_re = None;
            }
        }
        if let Some(re) = &self.success_re {
            if re.is_match(line) {
                out.start = true;
                self.stop_all();
                return;
            }
        }
        if let Some(re) = &self.failure_re {
            if re.is_match(line) {
                out.abort = Some((Status::ClientUnauthorized, "Login failed".to_string()));
                self.stop_all();
            }
        }
    }

    fn stop_all(&mut self) {
        self.username_re = None;
        self.password_re = None;
        self.success_re = None;
        self.failure_re = None;
    }
}

impl RemoteFilter for LoginFilter {
    fn on_remote_data(&mut self, data: &[u8]) -> FilterOutput {
        let mut out = FilterOutput::default();
        for &b in data {
            if b == b'\n' {
                if !self.line.is_empty() {
                    let l = String::from_utf8_lossy(&self.line).into_owned();
                    self.search_line(&l, &mut out);
                    self.line.clear();
                }
            } else if self.line.len() < MAX_LINE {
                self.line.push(b);
            }
        }
        // An unfinished line may be a prompt ("login: ").
        if !self.line.is_empty() {
            let l = String::from_utf8_lossy(&self.line).into_owned();
            self.search_line(&l, &mut out);
        }
        out
    }
}
