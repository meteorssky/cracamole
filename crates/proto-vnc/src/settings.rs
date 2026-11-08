use crate::format::PixelFormat;
use guac_session::Params;

/// Text encoding used for the RFB cut-text messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardEncoding {
    /// The encoding the RFB specification mandates.
    Latin1,
    /// What macOS Screen Sharing actually exchanges for non-ASCII text.
    Utf8,
}

#[derive(Debug, Clone)]
pub struct VncSettings {
    pub hostname: String,
    pub port: u16,
    pub timeout_secs: u64,
    pub username: String,
    pub password: String,
    pub read_only: bool,
    pub format: PixelFormat,
    pub clipboard: ClipboardEncoding,
    pub disable_copy: bool,
    pub disable_paste: bool,
    pub warnings: Vec<String>,
}

impl VncSettings {
    pub fn from_params(p: &Params) -> Option<Self> {
        let hostname = p.get("hostname").filter(|h| !h.is_empty())?.to_string();
        let mut warnings = vec![];
        let port = match p.get("port") {
            None => 5900,
            Some(v) => v
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .unwrap_or_else(|| {
                    warnings.push(format!("Invalid port \"{v}\"; using 5900."));
                    5900
                }),
        };
        let swap = p.get_bool("swap-red-blue");
        let format = match p.get("color-depth").map(str::trim) {
            None | Some("24") | Some("32") => PixelFormat::rgb32(swap),
            Some("16") => PixelFormat::rgb565(swap),
            Some("8") => PixelFormat::bgr233(swap),
            Some(other) => {
                warnings.push(format!("Unsupported color depth \"{other}\"; using 24."));
                PixelFormat::rgb32(swap)
            }
        };
        let clipboard = match p
            .get("clipboard-encoding")
            .map(|v| v.trim().to_ascii_uppercase())
            .as_deref()
        {
            None | Some("ISO8859-1") | Some("ISO-8859-1") => ClipboardEncoding::Latin1,
            Some("UTF-8") | Some("UTF8") => ClipboardEncoding::Utf8,
            Some(other) => {
                warnings.push(format!(
                    "Unsupported clipboard encoding \"{other}\"; using ISO8859-1."
                ));
                ClipboardEncoding::Latin1
            }
        };
        Some(Self {
            hostname,
            port,
            timeout_secs: p.get_u32("timeout").map_or(15, u64::from).clamp(1, 120),
            username: p.get("username").unwrap_or("").to_string(),
            password: p.get("password").unwrap_or("").to_string(),
            read_only: p.get_bool("read-only"),
            format,
            clipboard,
            disable_copy: p.get_bool("disable-copy"),
            disable_paste: p.get_bool("disable-paste"),
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, &str)]) -> Params {
        Params::from_pairs(pairs.iter().copied())
    }

    #[test]
    fn defaults_follow_the_vnc_conventions() {
        let s = VncSettings::from_params(&params(&[("hostname", "mac.local")])).unwrap();
        assert_eq!(s.port, 5900);
        assert_eq!(s.timeout_secs, 15);
        assert_eq!(s.format, PixelFormat::rgb32(false));
        assert_eq!(s.clipboard, ClipboardEncoding::Latin1);
        assert!(s.warnings.is_empty() && !s.read_only);
        assert!(VncSettings::from_params(&params(&[("port", "5900")])).is_none());
    }

    #[test]
    fn invalid_values_fall_back_with_a_warning() {
        let s = VncSettings::from_params(&params(&[
            ("hostname", "h"),
            ("port", "99999"),
            ("color-depth", "12"),
            ("clipboard-encoding", "EBCDIC"),
            ("timeout", "9999"),
        ]))
        .unwrap();
        assert_eq!((s.port, s.timeout_secs), (5900, 120));
        assert_eq!(s.warnings.len(), 3);
    }

    #[test]
    fn options_are_parsed() {
        let s = VncSettings::from_params(&params(&[
            ("hostname", "h"),
            ("color-depth", "16"),
            ("swap-red-blue", "true"),
            ("clipboard-encoding", "utf-8"),
            ("read-only", "true"),
        ]))
        .unwrap();
        assert_eq!(s.format, PixelFormat::rgb565(true));
        assert_eq!(s.clipboard, ClipboardEncoding::Utf8);
        assert!(s.read_only);
    }
}
