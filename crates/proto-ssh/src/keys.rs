use russh::keys::PrivateKey;

#[derive(Debug, PartialEq, Eq)]
pub enum KeyError {
    /// The key is encrypted and no (or the wrong) passphrase was supplied.
    NeedsPassphrase,
    Invalid(String),
}

/// Parse a private key (OpenSSH, PEM or PuTTY format). Mirrors the C two-step import: first without
/// a passphrase, then — only if that failed — with one.
pub fn load_private_key(text: &str, passphrase: Option<&str>) -> Result<PrivateKey, KeyError> {
    match russh::keys::decode_secret_key(text, passphrase) {
        Ok(k) => Ok(k),
        Err(russh::keys::Error::KeyIsEncrypted) => Err(KeyError::NeedsPassphrase),
        Err(e) => {
            // A wrong passphrase on an encrypted key surfaces as a decryption/format error.
            if passphrase.is_some() || looks_encrypted(text) {
                if passphrase.is_some() {
                    Err(KeyError::Invalid(format!("unable to decrypt key: {e}")))
                } else {
                    Err(KeyError::NeedsPassphrase)
                }
            } else {
                Err(KeyError::Invalid(e.to_string()))
            }
        }
    }
}

fn looks_encrypted(text: &str) -> bool {
    text.contains("ENCRYPTED")
        || text.contains("Proc-Type: 4,ENCRYPTED")
        || text.contains("Encryption: aes")
}
