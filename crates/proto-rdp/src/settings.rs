use guac_session::Params;

/// Which RDP security protocols the client may negotiate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// NLA (CredSSP) or TLS, whichever the server prefers.
    Any,
    Nla,
    Tls,
}

#[derive(Debug, Clone)]
pub struct RdpSettings {
    pub hostname: String,
    pub port: u16,
    pub timeout_secs: u64,
    pub domain: Option<String>,
    pub username: String,
    pub password: String,
    pub security: Security,
    pub ignore_cert: bool,
    pub read_only: bool,
    pub disable_copy: bool,
    pub disable_paste: bool,
    pub client_name: String,
    pub wallpaper: bool,
    pub font_smoothing: bool,
    pub full_window_drag: bool,
    pub desktop_composition: bool,
    pub menu_animations: bool,
    pub warnings: Vec<String>,
}

impl RdpSettings {
    pub fn from_params(p: &Params) -> Option<Self> {
        let hostname = p.get("hostname").filter(|h| !h.is_empty())?.to_string();
        let mut warnings = vec![];
        let port = match p.get("port").filter(|v| !v.is_empty()) {
            None => 3389,
            Some(v) => v
                .parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .unwrap_or_else(|| {
                    warnings.push(format!("Invalid port \"{v}\"; using 3389."));
                    3389
                }),
        };
        let security = match p.get("security").unwrap_or("") {
            "" | "any" => Security::Any,
            "nla" => Security::Nla,
            "tls" => Security::Tls,
            other => {
                warnings.push(format!("Unsupported security mode \"{other}\"; using any."));
                Security::Any
            }
        };
        Some(Self {
            hostname,
            port,
            timeout_secs: p.get_u32("timeout").map_or(15, u64::from).clamp(1, 120),
            domain: p
                .get("domain")
                .filter(|d| !d.is_empty())
                .map(str::to_string),
            username: p.get("username").unwrap_or("").to_string(),
            password: p.get("password").unwrap_or("").to_string(),
            security,
            ignore_cert: p.get_bool("ignore-cert"),
            read_only: p.get_bool("read-only"),
            disable_copy: p.get_bool("disable-copy"),
            disable_paste: p.get_bool("disable-paste"),
            client_name: p
                .get("client-name")
                .filter(|n| !n.is_empty())
                .unwrap_or("cracamole")
                .chars()
                .take(15)
                .collect(),
            wallpaper: p.get_bool("enable-wallpaper"),
            font_smoothing: p.get_bool("enable-font-smoothing"),
            full_window_drag: p.get_bool("enable-full-window-drag"),
            desktop_composition: p.get_bool("enable-desktop-composition"),
            menu_animations: p.get_bool("enable-menu-animations"),
            warnings,
        })
    }
}
