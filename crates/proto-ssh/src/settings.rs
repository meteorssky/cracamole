use guac_session::Params;

pub const DEFAULT_PORT: u16 = 22;
pub const DEFAULT_TIMEOUT_SECS: u64 = 10;
pub const SYSTEM_KNOWN_HOSTS: &str = "/etc/guacamole/ssh_known_hosts";

/// Secret string that overwrites its memory on drop and never prints.
#[derive(Clone, Default)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}

#[derive(Debug, Clone)]
pub struct SshSettings {
    pub hostname: String,
    pub port: u16,
    pub timeout_secs: u64,
    pub username: Option<String>,
    pub password: Option<Secret>,
    pub private_key: Option<Secret>,
    pub passphrase: Option<Secret>,
    pub public_key: Option<String>,
    pub host_key: Option<String>,
    pub command: Option<String>,
    pub server_alive_interval: u64,
    pub terminal_type: String,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub backspace: u8,
    pub warnings: Vec<String>,
}

impl SshSettings {
    /// `None` when `hostname` is missing.
    pub fn from_params(p: &Params, client_timezone: Option<&str>) -> Option<Self> {
        let hostname = p.get("hostname")?.to_string();
        let port = p
            .get("port")
            .and_then(|s| s.trim().parse::<u16>().ok().filter(|&n| n > 0))
            .unwrap_or(DEFAULT_PORT);
        let mut warnings = vec![];
        if p.get_bool("enable-sftp") {
            warnings.push("\"enable-sftp\" is not supported yet and is ignored".into());
        }
        if p.get("recording-path").is_some() || p.get("typescript-path").is_some() {
            warnings.push("session recording is not supported yet and is ignored".into());
        }
        Some(Self {
            hostname,
            port,
            timeout_secs: p
                .get_u32("timeout")
                .map(u64::from)
                .unwrap_or(DEFAULT_TIMEOUT_SECS),
            username: p.get("username").map(str::to_string),
            password: p.get("password").map(Secret::new),
            private_key: p.get("private-key").map(Secret::new),
            passphrase: p.get("passphrase").map(Secret::new),
            public_key: p.get("public-key").map(str::to_string),
            host_key: p.get("host-key").map(str::to_string),
            command: p.get("command").map(str::to_string),
            server_alive_interval: p
                .get_u32("server-alive-interval")
                .map(u64::from)
                .unwrap_or(0),
            terminal_type: p.get("terminal-type").unwrap_or("linux").to_string(),
            locale: p.get("locale").map(str::to_string),
            timezone: p.get("timezone").or(client_timezone).map(str::to_string),
            backspace: p
                .get_u32("backspace")
                .map(|b| b.min(255) as u8)
                .unwrap_or(127),
            warnings,
        })
    }
}
