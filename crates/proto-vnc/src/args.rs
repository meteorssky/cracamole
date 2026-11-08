//! Connection parameters advertised in `args` (names follow Apache Guacamole's VNC plugin).

pub const ARGS: &[&str] = &[
    "hostname",
    "port",
    "timeout",
    "username",
    "password",
    "read-only",
    "swap-red-blue",
    "color-depth",
    "clipboard-encoding",
    "disable-copy",
    "disable-paste",
];
