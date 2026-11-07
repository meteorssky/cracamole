//! RDP connection sequence: X.224 negotiation, TLS upgrade, optional NLA, capability exchange.

use crate::settings::{RdpSettings, Security};
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
use std::net::SocketAddr;
use tokio::net::TcpStream;

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
            width: width.clamp(200, 8192) as u16,
            height: height.clamp(200, 8192) as u16,
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

    let validation = if s.ignore_cert {
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
        let text = e.to_string();
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
