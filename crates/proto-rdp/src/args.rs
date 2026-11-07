//! Connection parameters advertised in `args` (names follow Apache Guacamole's RDP plugin).

pub const ARGS: &[&str] = &[
    "hostname",
    "port",
    "timeout",
    "domain",
    "username",
    "password",
    "security",
    "ignore-cert",
    "server-cert-sha256",
    "color-depth",
    "read-only",
    "disable-copy",
    "disable-paste",
    "enable-wallpaper",
    "enable-font-smoothing",
    "enable-full-window-drag",
    "enable-desktop-composition",
    "enable-menu-animations",
    "client-name",
    "keyboard-layout",
];
