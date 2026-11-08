//! RDP connection sequence: X.224 negotiation, TLS upgrade, optional NLA, capability exchange.

use crate::settings::{CertPin, RdpSettings, Security};
use guac_session::{SessionError, Status};
use ironrdp_connector::sspi::generator::NetworkRequest;
use ironrdp_connector::{
    ClientConnector, Config, ConnectionOutcome, ConnectionResult, ConnectorError, Credentials,
    DesktopSize,
};
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_dvc::DrdynvcClient;
use ironrdp_pdu::gcc::KeyboardType;
use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_tls::CertificateValidation;
use ironrdp_tokio::{NetworkClient, TokioFramed};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use tokio::net::TcpStream;
use x509_cert::der::Encode;

pub type Framed = TokioFramed<ironrdp_tls::TlsStream<TcpStream>>;

/// CredSSP may ask for a Kerberos KDC; only NTLM is supported, so refuse network requests.
struct NoNetwork;

impl NetworkClient for NoNetwork {
    async fn send(&mut self, _request: &NetworkRequest) -> Result<Vec<u8>, ConnectorError> {
        Err(ironrdp_connector::general_err!(
            "Kerberos network authentication is not supported"
        ))
    }
}

fn performance(s: &RdpSettings) -> PerformanceFlags {
    let mut flags = PerformanceFlags::empty();
    if !s.wallpaper {
        flags |= PerformanceFlags::DISABLE_WALLPAPER;
    }
    if !s.full_window_drag {
        flags |= PerformanceFlags::DISABLE_FULLWINDOWDRAG;
    }
    if !s.menu_animations {
        flags |= PerformanceFlags::DISABLE_MENUANIMATIONS;
    }
    if !s.font_smoothing {
        flags |= PerformanceFlags::DISABLE_CURSOR_SHADOW;
    } else {
        flags |= PerformanceFlags::ENABLE_FONT_SMOOTHING;
    }
    if s.desktop_composition {
        flags |= PerformanceFlags::ENABLE_DESKTOP_COMPOSITION;
    }
    flags
}

pub fn build_config(s: &RdpSettings, width: u32, height: u32) -> Config {
    Config {
        desktop_size: DesktopSize {
            width: width.clamp(200, crate::image::MAX_DIMENSION) as u16,
            height: height.clamp(200, crate::image::MAX_DIMENSION) as u16,
        },
        monitor_layout: None,
        desktop_scale_factor: 0,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        enable_tls: s.security != Security::Nla,
        enable_credssp: s.security != Security::Tls,
        enable_standard_rdp_security: false,
        credentials: Credentials::UsernamePassword {
            username: s.username.clone(),
            password: s.password.clone(),
        },
        domain: s.domain.clone(),
        client_build: 0,
        client_name: s.client_name.clone(),
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: 0x0409,
        connection_type: ironrdp_pdu::gcc::ConnectionType::Lan,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_string(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: RailSupportLevel::empty(),
        platform: MajorPlatformType::UNSPECIFIED,
        hardware_id: None,
        request_data: None,
        autologon: !s.username.is_empty(),
        enable_audio_playback: false,
        enable_audio_capture: false,
        performance_flags: performance(s),
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        compression_type: None,
        enable_server_pointer: true,
        pointer_software_rendering: true,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: false,
    }
}

/// SHA-256 of the certificate's DER encoding, as 64 lowercase hex characters.
pub fn fingerprint(cert: &x509_cert::Certificate) -> Option<String> {
    let der = cert.to_der().ok()?;
    Some(
        Sha256::digest(&der)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// What the user needs to decide whether to trust an RDP server's certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    pub sha256: String,
    pub subject: String,
    pub issuer: String,
}

/// Perform only the X.224 negotiation and TLS handshake, accepting any certificate, and report it.
/// Nothing is authenticated and no session is opened.
pub async fn probe_certificate(
    host: &str,
    port: u16,
    timeout_secs: u64,
) -> Result<CertInfo, SessionError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs.max(1)),
        probe_certificate_inner(host, port, timeout_secs),
    )
    .await
    .map_err(|_| {
        SessionError::plugin(Status::UpstreamTimeout, "RDP certificate probe timed out.")
    })?
}

async fn probe_certificate_inner(
    host: &str,
    port: u16,
    timeout_secs: u64,
) -> Result<CertInfo, SessionError> {
    let stream = guac_session::connect_tcp(host, port, timeout_secs).await?;
    let peer: SocketAddr = stream
        .peer_addr()
        .map_err(|e| SessionError::plugin(Status::UpstreamUnavailable, e.to_string()))?;
    let mut params = guac_session::Params::new();
    params.insert("hostname", host);
    let mut settings = RdpSettings::from_params(&params)
        .ok_or_else(|| SessionError::plugin(Status::ClientBadRequest, "No hostname provided."))?;
    // Discovery sends no credentials: permit either TLS or NLA to obtain the certificate.
    // Actual connections still enforce the security mode chosen by the user.
    settings.security = Security::Any;
    let mut connector = ClientConnector::new(build_config(&settings, 1024, 768), peer);
    let mut framed = TokioFramed::new(stream);
    ironrdp_tokio::connect_begin(&mut framed, &mut connector)
        .await
        .map_err(upstream)?;
    let (_tls, cert) = ironrdp_tls::upgrade_with_certificate_validation(
        framed.into_inner_no_leftover(),
        host,
        CertificateValidation::DangerouslyAcceptInvalidCertificate,
    )
    .await
    .map_err(|e| upstream(format!("TLS handshake failed: {e}")))?;
    Ok(CertInfo {
        sha256: fingerprint(&cert).ok_or_else(|| upstream("cannot encode the certificate"))?,
        subject: cert.tbs_certificate().subject().to_string(),
        issuer: cert.tbs_certificate().issuer().to_string(),
    })
}

/// `outer: inner: innermost`, so the log says why (for example which CredSSP step failed).
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        let part = inner.to_string();
        if !text.ends_with(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        source = inner.source();
    }
    text
}

fn upstream(e: impl std::fmt::Display) -> SessionError {
    SessionError::plugin(Status::UpstreamError, format!("RDP connection failed: {e}"))
}

pub async fn establish(
    s: &RdpSettings,
    stream: TcpStream,
    width: u32,
    height: u32,
    clipboard: Option<tokio::sync::mpsc::UnboundedSender<crate::clipboard::ClipEvent>>,
) -> Result<(Framed, ConnectionResult), SessionError> {
    let peer: SocketAddr = stream
        .peer_addr()
        .map_err(|e| SessionError::plugin(Status::UpstreamUnavailable, e.to_string()))?;
    let mut connector = ClientConnector::new(build_config(s, width, height), peer);
    // Dynamic virtual channel host with Display Control, so the desktop can follow the browser size.
    connector.attach_static_channel(
        DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new()))),
    );
    if let Some(tx) = clipboard {
        connector.attach_static_channel(ironrdp_cliprdr::CliprdrClient::new(Box::new(
            crate::clipboard::Backend::new(tx),
        )));
    }
    let mut framed = TokioFramed::new(stream);
    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector)
        .await
        .map_err(upstream)?;

    // A pinned fingerprint is checked after the handshake (below). It is stricter than `ignore-cert`,
    // so it wins when both are set.
    let pinned = match &s.cert_pin {
        CertPin::Invalid => {
            return Err(SessionError::plugin(
                Status::ClientBadRequest,
                "The trusted certificate fingerprint is not a valid SHA-256 value.",
            ))
        }
        CertPin::Fingerprint(pin) => Some(pin.clone()),
        CertPin::None => None,
    };
    if s.ignore_cert && pinned.is_none() {
        // Without a pin anyone on the path can present a certificate. Plain TLS would then send
        // the password in clear text, so only NLA (which at worst leaks a salted challenge) is allowed.
        if s.security != Security::Nla {
            return Err(SessionError::plugin(
                Status::ClientBadRequest,
                "ignore-cert without a pinned fingerprint requires security=nla.",
            ));
        }
        tracing::warn!(host = %s.hostname, "RDP server certificate is not verified (ignore-cert)");
    }
    let validation = if s.ignore_cert || pinned.is_some() {
        CertificateValidation::DangerouslyAcceptInvalidCertificate
    } else {
        CertificateValidation::Strict
    };
    let initial = framed.into_inner_no_leftover();
    let (tls, cert) =
        ironrdp_tls::upgrade_with_certificate_validation(initial, &s.hostname, validation)
            .await
            .map_err(|e| {
                SessionError::plugin(
                    Status::UpstreamError,
                    format!("TLS handshake with the RDP server failed: {e}"),
                )
            })?;
    if let Some(pin) = &pinned {
        let actual = fingerprint(&cert).unwrap_or_default();
        if &actual != pin {
            return Err(SessionError::plugin(
                Status::UpstreamError,
                format!(
                    "The RDP server certificate does not match the trusted one (SHA-256 {actual})."
                ),
            ));
        }
    }
    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&cert)
        .ok_or_else(|| upstream("the server certificate has no public key"))?
        .to_vec();

    let mut framed = TokioFramed::new(tls);
    let outcome = ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut NoNetwork,
        s.hostname.clone().into(),
        server_public_key,
        None,
    )
    .await
    .map_err(|e| {
        let text = error_chain(&e);
        let lower = text.to_lowercase();
        if lower.contains("logon") || lower.contains("credssp") || lower.contains("access denied") {
            SessionError::plugin(
                Status::ClientUnauthorized,
                format!("RDP authentication failed: {text}"),
            )
        } else {
            upstream(text)
        }
    })?;
    match outcome {
        ConnectionOutcome::Connected(result) => Ok((framed, result)),
        _ => Err(upstream("server redirection is not supported")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn certificate_probe_deadline_covers_x224_and_tls_and_closes_socket() {
        for selected_protocol in [None, Some(1u8), Some(2u8)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let peer = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut data = [0; 2048];
                assert!(stream.read(&mut data).await.unwrap() > 0);
                if let Some(protocol) = selected_protocol {
                    stream
                        // Select TLS or HYBRID, then stall before the certificate is sent.
                        .write_all(&[
                            3, 0, 0, 19, 14, 0xd0, 0, 0, 0, 0, 0, 2, 0, 8, 0, protocol, 0, 0, 0,
                        ])
                        .await
                        .unwrap();
                }
                loop {
                    match stream.read(&mut data).await {
                        Ok(0) | Err(_) => break,
                        _ => {}
                    }
                }
            });
            let error = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                probe_certificate("127.0.0.1", port, 1),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert_eq!(
                error.status(),
                Status::UpstreamTimeout,
                "selected_protocol={selected_protocol:?}: {error}"
            );
            tokio::time::timeout(std::time::Duration::from_secs(1), peer)
                .await
                .unwrap()
                .unwrap();
        }
    }
}
