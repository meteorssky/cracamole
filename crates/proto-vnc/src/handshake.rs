//! RFB 3.3 / 3.7 / 3.8 connection setup: version, security negotiation, authentication, init.

use crate::auth::{self, ArdOffer};
use crate::decode::{bad, checked_name_len};
use crate::settings::VncSettings;
use guac_session::{SessionError, Status};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SEC_NONE: u8 = 1;
pub const SEC_VNC: u8 = 2;
/// Apple Remote Desktop authentication (macOS Screen Sharing).
pub const SEC_ARD: u8 = 30;

#[derive(Debug, PartialEq, Eq)]
pub struct ServerInit {
    pub width: u32,
    pub height: u32,
    pub name: String,
}

/// Source of random bytes; replaced in tests.
pub type Random = fn(&mut [u8]) -> Result<(), String>;

fn io(e: std::io::Error) -> SessionError {
    SessionError::plugin(
        Status::UpstreamUnavailable,
        format!("VNC connection lost: {e}"),
    )
}

fn unauthorized(message: impl Into<String>) -> SessionError {
    SessionError::plugin(Status::ClientUnauthorized, message)
}

async fn read_reason<S: AsyncRead + Unpin>(s: &mut S) -> Result<String, SessionError> {
    let len = checked_name_len(s.read_u32().await.map_err(io)?)?;
    let mut text = vec![0u8; len];
    s.read_exact(&mut text).await.map_err(io)?;
    Ok(String::from_utf8_lossy(&text).chars().take(200).collect())
}

/// Version the client speaks for a server announcing `major.minor`.
fn client_minor(major: u32, minor: u32) -> Result<u32, SessionError> {
    match (major, minor) {
        (3, m) if m >= 8 => Ok(8),
        (3, 7) => Ok(7),
        (3, _) => Ok(3),
        _ => Err(bad(format!("unsupported protocol version {major}.{minor}"))),
    }
}

/// Pick the security type. Order of preference follows what the user configured:
/// an account name selects ARD (macOS), a bare password selects classic VNC authentication.
fn choose_security(offered: &[u8], settings: &VncSettings) -> Result<u8, SessionError> {
    let has = |t: u8| offered.contains(&t);
    let user = !settings.username.is_empty();
    let pass = !settings.password.is_empty();
    if has(SEC_ARD) && user && pass {
        return Ok(SEC_ARD);
    }
    if has(SEC_VNC) && pass {
        return Ok(SEC_VNC);
    }
    if has(SEC_ARD) {
        return Err(unauthorized(
            "This computer (macOS Screen Sharing) needs both an account name and a password.",
        ));
    }
    if has(SEC_VNC) {
        return Err(unauthorized("This VNC server needs a password."));
    }
    // A server that offers no authentication must not receive credentials the user configured
    // for it: that is the shape of a downgrade by a man in the middle.
    if has(SEC_NONE) && !user && !pass {
        return Ok(SEC_NONE);
    }
    if has(SEC_NONE) {
        return Err(bad(
            "the server offers no authentication although credentials are configured; refusing to continue",
        ));
    }
    Err(bad(format!(
        "no supported security type (server offered {offered:?}; supported: 1 None, 2 VNC password, 30 Apple Remote Desktop)"
    )))
}

pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    settings: &VncSettings,
    random: Random,
) -> Result<ServerInit, SessionError> {
    // --- version ---
    let mut version = [0u8; 12];
    s.read_exact(&mut version).await.map_err(io)?;
    let text = std::str::from_utf8(&version).unwrap_or("");
    let (major, minor) = text
        .strip_prefix("RFB ")
        .and_then(|r| r.strip_suffix('\n'))
        .and_then(|r| r.split_once('.'))
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
        .ok_or_else(|| bad("the remote port does not speak VNC (RFB)"))?;
    let minor = client_minor(major, minor)?;
    s.write_all(format!("RFB 003.{minor:03}\n").as_bytes())
        .await
        .map_err(io)?;

    // --- security type ---
    let offered: Vec<u8> = if minor == 3 {
        let t = s.read_u32().await.map_err(io)?;
        if t == 0 {
            return Err(unauthorized(format!(
                "The server refused the connection: {}",
                read_reason(s).await?
            )));
        }
        vec![u8::try_from(t).unwrap_or(0)]
    } else {
        let n = s.read_u8().await.map_err(io)?;
        if n == 0 {
            return Err(unauthorized(format!(
                "The server refused the connection: {}",
                read_reason(s).await?
            )));
        }
        let mut types = vec![0u8; usize::from(n)];
        s.read_exact(&mut types).await.map_err(io)?;
        types
    };
    let chosen = choose_security(&offered, settings)?;
    if minor >= 7 {
        s.write_u8(chosen).await.map_err(io)?;
    }
    tracing::debug!("VNC security type {chosen} selected (offered {offered:?}).");

    // --- authentication ---
    match chosen {
        SEC_VNC => {
            let mut challenge = [0u8; 16];
            s.read_exact(&mut challenge).await.map_err(io)?;
            s.write_all(&auth::vnc_response(&settings.password, &challenge))
                .await
                .map_err(io)?;
        }
        SEC_ARD => ard(s, settings, random).await?,
        _ => {}
    }
    s.flush().await.map_err(io)?;

    // --- security result (not sent for "None" before 3.8) ---
    if minor >= 8 || chosen != SEC_NONE {
        let result = s.read_u32().await.map_err(io)?;
        if result != 0 {
            let reason = if minor >= 8 {
                read_reason(s).await.unwrap_or_default()
            } else {
                String::new()
            };
            return Err(unauthorized(if reason.is_empty() {
                "Authentication failed: check the account name and password.".to_string()
            } else {
                format!("Authentication failed: {reason}")
            }));
        }
    }

    // --- init ---
    s.write_u8(1).await.map_err(io)?; // shared session
    s.flush().await.map_err(io)?;
    let mut init = [0u8; 24];
    s.read_exact(&mut init).await.map_err(io)?;
    let width = u32::from(u16::from_be_bytes([init[0], init[1]]));
    let height = u32::from(u16::from_be_bytes([init[2], init[3]]));
    let name_len = checked_name_len(u32::from_be_bytes([init[20], init[21], init[22], init[23]]))?;
    let mut name = vec![0u8; name_len];
    s.read_exact(&mut name).await.map_err(io)?;
    Ok(ServerInit {
        width,
        height,
        name: String::from_utf8_lossy(&name)
            .chars()
            .filter(|c| !c.is_control())
            .collect(),
    })
}

async fn ard<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    settings: &VncSettings,
    random: Random,
) -> Result<(), SessionError> {
    let generator = s.read_u16().await.map_err(io)?;
    let key_len = usize::from(s.read_u16().await.map_err(io)?);
    if !(auth::MIN_DH_BYTES..=auth::MAX_DH_BYTES).contains(&key_len) {
        return Err(bad("the server sent unusable Diffie-Hellman parameters"));
    }
    let mut prime = vec![0u8; key_len];
    s.read_exact(&mut prime).await.map_err(io)?;
    let mut server_public = vec![0u8; key_len];
    s.read_exact(&mut server_public).await.map_err(io)?;

    let mut private = vec![0u8; key_len];
    random(&mut private).map_err(bad)?;
    let mut filler = [0u8; 128];
    random(&mut filler).map_err(bad)?;
    let (cipher, public) = auth::ard_response(
        &ArdOffer {
            generator,
            prime: &prime,
            server_public: &server_public,
        },
        &settings.username,
        &settings.password,
        &private,
        &filler,
    )
    .map_err(bad)?;
    let mut reply = cipher;
    reply.extend_from_slice(&public);
    s.write_all(&reply).await.map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use guac_session::Params;

    fn settings(user: &str, pass: &str) -> VncSettings {
        let mut p = Params::new();
        p.insert("hostname", "h");
        p.insert("username", user);
        p.insert("password", pass);
        VncSettings::from_params(&p).unwrap()
    }

    #[test]
    fn security_choice_prefers_the_configured_credentials() {
        let both = settings("admin", "secret");
        assert_eq!(choose_security(&[1, 2, 30], &both).unwrap(), SEC_ARD);
        assert_eq!(choose_security(&[2, 1], &both).unwrap(), SEC_VNC);
        let pass_only = settings("", "secret");
        assert_eq!(choose_security(&[30, 2], &pass_only).unwrap(), SEC_VNC);
        let nothing = settings("", "");
        assert_eq!(choose_security(&[1], &nothing).unwrap(), SEC_NONE);
    }

    #[test]
    fn security_choice_never_downgrades_or_guesses() {
        let both = settings("admin", "secret");
        // Credentials are configured but the server offers none: possible man in the middle.
        assert!(choose_security(&[1], &both).is_err());
        // Unsupported types only (e.g. 33, 35, 36 from newer Apple servers).
        assert!(choose_security(&[33, 35, 36], &both).is_err());
        // ARD without an account name cannot work: say so instead of failing mid-exchange.
        let e = choose_security(&[30], &settings("", "secret")).unwrap_err();
        assert_eq!(e.status(), Status::ClientUnauthorized);
        assert!(choose_security(&[2], &settings("", "")).is_err());
    }

    #[test]
    fn versions_are_negotiated_down() {
        assert_eq!(client_minor(3, 889).unwrap(), 8); // Apple announces 003.889
        assert_eq!(client_minor(3, 8).unwrap(), 8);
        assert_eq!(client_minor(3, 7).unwrap(), 7);
        assert_eq!(client_minor(3, 3).unwrap(), 3);
        assert_eq!(client_minor(3, 5).unwrap(), 3);
        assert!(client_minor(4, 0).is_err());
    }
}
