use crate::login::{compile, LoginFilter};
use guac_session::Params;
use regex::Regex;

pub const DEFAULT_PORT: u16 = 23;
pub const DEFAULT_TIMEOUT_SECS: u64 = 10;
pub const DEFAULT_USERNAME_REGEX: &str = "[Ll]ogin:";
pub const DEFAULT_PASSWORD_REGEX: &str = "[Pp]assword:";

#[derive(Debug)]
pub struct TelnetSettings {
    pub hostname: String,
    pub port: u16,
    pub timeout_secs: u64,
    pub terminal_type: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub username_regex: Option<Regex>,
    pub password_regex: Option<Regex>,
    pub login_success_regex: Option<Regex>,
    pub login_failure_regex: Option<Regex>,
    /// Problems found while parsing; logged by the caller.
    pub warnings: Vec<String>,
}

fn regex_param(
    p: &Params,
    name: &str,
    default: Option<&str>,
    warnings: &mut Vec<String>,
) -> Option<Regex> {
    let pat = p.get(name).or(default)?;
    match compile(pat) {
        Ok(r) => Some(r),
        Err(e) => {
            warnings.push(format!(
                "Regular expression \"{pat}\" for \"{name}\" is invalid: {e}"
            ));
            None
        }
    }
}

impl TelnetSettings {
    /// Returns `None` when `hostname` is missing (the connection cannot proceed).
    pub fn from_params(p: &Params) -> Option<Self> {
        let hostname = p.get("hostname")?.to_string();
        let mut warnings = vec![];
        let port = p
            .get("port")
            .and_then(|s| s.trim().parse::<u16>().ok().filter(|&n| n > 0))
            .unwrap_or(DEFAULT_PORT);
        let timeout_secs = p
            .get_u32("timeout")
            .map(u64::from)
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        let username = p.get("username").map(str::to_string);
        let password = p.get("password").map(str::to_string);

        // Prompt regexes only matter if the matching credential was supplied.
        let username_regex = if username.is_some() {
            regex_param(
                p,
                "username-regex",
                Some(DEFAULT_USERNAME_REGEX),
                &mut warnings,
            )
        } else {
            None
        };
        let password_regex = if password.is_some() {
            regex_param(
                p,
                "password-regex",
                Some(DEFAULT_PASSWORD_REGEX),
                &mut warnings,
            )
        } else {
            None
        };
        let mut success = regex_param(p, "login-success-regex", None, &mut warnings);
        let mut failure = regex_param(p, "login-failure-regex", None, &mut warnings);
        // Both detection regexes are required together, or both are ignored.
        if success.is_some() != failure.is_some() {
            let (kept, other) = if success.is_some() {
                ("login-success-regex", "login-failure-regex")
            } else {
                ("login-failure-regex", "login-success-regex")
            };
            warnings.push(format!(
                "Ignoring provided value for \"{kept}\" as \"{other}\" must also be provided."
            ));
            success = None;
            failure = None;
        }

        Some(Self {
            hostname,
            port,
            timeout_secs,
            terminal_type: p.get("terminal-type").unwrap_or("linux").to_string(),
            username,
            password,
            username_regex,
            password_regex,
            login_success_regex: success,
            login_failure_regex: failure,
            warnings,
        })
    }

    /// The terminal stays hidden until login success is seen, but only if detection is configured.
    pub fn waits_for_login(&self) -> bool {
        self.login_success_regex.is_some()
    }

    pub fn login_filter(&self) -> Option<LoginFilter> {
        let f = LoginFilter::new(
            self.username.clone(),
            self.username_regex.clone(),
            self.password.clone(),
            self.password_regex.clone(),
            self.login_success_regex.clone(),
            self.login_failure_regex.clone(),
        );
        f.is_active().then_some(f)
    }
}
