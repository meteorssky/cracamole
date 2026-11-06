//! Command line and `guacd.conf` handling, compatible with the C `guacd`.
//!
//! Flags: `-l PORT -b HOST -p PIDFILE -L LEVEL -C CERT -K KEY -f -v` (getopt style, values may be attached: `-l4822`).
//! File: INI with `[server] bind_host bind_port`, `[daemon] pid_file log_level`.

use std::fmt;

pub const DEFAULT_BIND_HOST: &str = "localhost";
pub const DEFAULT_BIND_PORT: &str = "4822";
pub const CONF_FILE: &str = "/etc/guacamole/guacd.conf";
pub const VERSION_LINE: &str = concat!(
    "Guacamole proxy daemon (guacd) version ",
    env!("CARGO_PKG_VERSION")
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Error,
    Warning,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "error" => Self::Error,
            "warning" => Self::Warning,
            "info" => Self::Info,
            "debug" => Self::Debug,
            "trace" => Self::Trace,
            _ => return None,
        })
    }

    pub fn tracing_level(self) -> tracing::Level {
        match self {
            Self::Error => tracing::Level::ERROR,
            Self::Warning => tracing::Level::WARN,
            Self::Info => tracing::Level::INFO,
            Self::Debug => tracing::Level::DEBUG,
            Self::Trace => tracing::Level::TRACE,
        }
    }
}

const BAD_LEVEL: &str =
    "Invalid log level. Valid levels are: \"trace\", \"debug\", \"info\", \"warning\", and \"error\".";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind_host: String,
    pub bind_port: String,
    pub pid_file: Option<String>,
    pub foreground: bool,
    pub print_version: bool,
    pub log_level: LogLevel,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_host: DEFAULT_BIND_HOST.into(),
            bind_port: DEFAULT_BIND_PORT.into(),
            pid_file: None,
            foreground: false,
            print_version: false,
            log_level: LogLevel::Info,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

pub fn usage(prog: &str) -> String {
    format!("USAGE: {prog} [-l LISTENPORT] [-b LISTENADDRESS] [-p PIDFILE] [-L LEVEL] [-f] [-v]")
}

impl Config {
    /// Apply `guacd.conf` contents. Errors carry `line:column`-less messages like the C parser's.
    pub fn apply_conf(&mut self, text: &str) -> Result<(), ConfigError> {
        let mut section = String::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let err = |m: &str| ConfigError(format!("Line {}: {m}", n + 1));
            if let Some(rest) = line.strip_prefix('[') {
                let name = rest
                    .strip_suffix(']')
                    .ok_or_else(|| err("Unterminated section header"))?;
                section = name.trim().to_string();
                continue;
            }
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| err("Expected \"name = value\""))?;
            let k = k.trim();
            // Strip an inline comment and surrounding quotes.
            let v = v
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
            match (section.as_str(), k) {
                ("server", "bind_host") => self.bind_host = v,
                ("server", "bind_port") => self.bind_port = v,
                ("daemon", "pid_file") => self.pid_file = Some(v),
                ("daemon", "log_level") => {
                    self.log_level = LogLevel::parse(&v).ok_or_else(|| err(BAD_LEVEL))?;
                }
                ("ssl", _) => return Err(err("SSL support not compiled in")),
                _ => return Err(err("Invalid parameter or section name")),
            }
        }
        Ok(())
    }

    /// Parse command line arguments (without the program name).
    pub fn apply_args<I: IntoIterator<Item = String>>(
        &mut self,
        args: I,
    ) -> Result<(), ConfigError> {
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            let Some(flags) = arg.strip_prefix('-').filter(|f| !f.is_empty()) else {
                return Err(ConfigError("unexpected argument".into()));
            };
            let mut chars = flags.chars();
            while let Some(opt) = chars.next() {
                match opt {
                    'f' => self.foreground = true,
                    'v' => self.print_version = true,
                    'l' | 'b' | 'p' | 'L' | 'C' | 'K' => {
                        let rest: String = chars.by_ref().collect();
                        let value = if rest.is_empty() {
                            it.next().ok_or_else(|| {
                                ConfigError(format!("option requires an argument -- '{opt}'"))
                            })?
                        } else {
                            rest
                        };
                        match opt {
                            'l' => self.bind_port = value,
                            'b' => self.bind_host = value,
                            'p' => self.pid_file = Some(value),
                            'L' => self.log_level = LogLevel::parse(&value).ok_or_else(|| ConfigError(BAD_LEVEL.into()))?,
                            _ => {
                                return Err(ConfigError(format!(
                                    "This guacd does not have SSL/TLS support compiled in (option -{opt})."
                                )))
                            }
                        }
                    }
                    other => return Err(ConfigError(format!("invalid option -- '{other}'"))),
                }
            }
        }
        Ok(())
    }
}
