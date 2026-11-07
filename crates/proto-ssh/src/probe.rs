//! Read the SSH host key without offering any credentials or opening a terminal.

use crate::{hostkey, session::load_known_hosts, SshSettings};
use guac_session::{connect_tcp, Params, SessionError, Status};
use russh::client;
use russh::keys::{HashAlg, PublicKeyBase64};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct HostKeyInfo {
    pub public_key: String,
    pub fingerprint: String,
    pub trusted: bool,
    pub matches: bool,
}

struct Probe {
    result: Arc<Mutex<Option<HostKeyInfo>>>,
    entries: Vec<hostkey::Entry>,
    host: String,
    port: u16,
}

impl client::Handler for Probe {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        if let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = key {
            *self.result.lock().unwrap() = Some(HostKeyInfo {
                public_key: key.to_openssh()?,
                fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
                trusted: !self.entries.is_empty(),
                matches: hostkey::verify(
                    &self.entries,
                    &self.host,
                    self.port,
                    &key.public_key_bytes(),
                ) == hostkey::Verdict::Match,
            });
        }
        // Always stop at the host-key check. Never authenticate during discovery.
        Ok(false)
    }
}

pub async fn probe_host_key(
    host: &str,
    port: u16,
    configured: Option<&str>,
    timeout_secs: u64,
) -> Result<HostKeyInfo, SessionError> {
    tokio::time::timeout(Duration::from_secs(timeout_secs.max(1)), async {
        let mut params = Params::new();
        params.insert("hostname", host);
        if let Some(key) = configured {
            params.insert("host-key", key);
        }
        let settings = SshSettings::from_params(&params, None).unwrap();
        let entries = load_known_hosts(&settings)?;
        let result = Arc::new(Mutex::new(None));
        let stream = connect_tcp(host, port, timeout_secs).await?;
        let _ = client::connect_stream(
            Arc::new(client::Config::default()),
            stream,
            Probe {
                result: result.clone(),
                entries,
                host: host.to_string(),
                port,
            },
        )
        .await;
        let info = result.lock().unwrap().take();
        info.ok_or_else(|| {
            SessionError::plugin(Status::UpstreamError, "Unable to read SSH host key.")
        })
    })
    .await
    .map_err(|_| SessionError::plugin(Status::UpstreamTimeout, "SSH host key probe timed out."))?
}
