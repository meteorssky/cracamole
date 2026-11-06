//! Server host key verification against `known_hosts` data (the `host-key` parameter or
//! `/etc/guacamole/ssh_known_hosts`), following the C `guac_common_ssh_verify_host_key` outcomes.

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Comma-separated host patterns; `None` for a bare "keytype base64" key that matches any host.
    pub hosts: Option<String>,
    pub key_type: String,
    pub key_blob: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// An entry for this host has exactly this key.
    Match,
    /// No entry applies to this host.
    NotFound,
    /// Entries exist for this host but none has this key.
    Mismatch,
}

fn decode_b64(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .ok()
}

/// Parse known_hosts text (one entry per line). Also accepts a lone base64 public key blob, or
/// "keytype base64". Unparseable lines are skipped and reported in the second element.
pub fn parse(text: &str) -> (Vec<Entry>, Vec<String>) {
    let mut entries = vec![];
    let mut errors = vec![];
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts: Vec<&str> = line.split_whitespace().collect();
        // Optional marker (@cert-authority / @revoked) is not supported: skip such lines.
        if parts.first().is_some_and(|p| p.starts_with('@')) {
            errors.push(format!("line {}: markers are not supported", n + 1));
            continue;
        }
        let entry = match parts.len() {
            1 => decode_b64(parts[0]).and_then(|blob| {
                key_type_of(&blob).map(|t| Entry {
                    hosts: None,
                    key_type: t,
                    key_blob: blob,
                })
            }),
            2 => decode_b64(parts[1]).map(|blob| Entry {
                hosts: None,
                key_type: parts[0].into(),
                key_blob: blob,
            }),
            _ => {
                parts.truncate(3);
                decode_b64(parts[2]).map(|blob| Entry {
                    hosts: Some(parts[0].to_string()),
                    key_type: parts[1].to_string(),
                    key_blob: blob,
                })
            }
        };
        match entry {
            Some(e) => entries.push(e),
            None => errors.push(format!("line {}: invalid host key entry", n + 1)),
        }
    }
    (entries, errors)
}

/// The algorithm name embedded at the start of an SSH public key blob.
fn key_type_of(blob: &[u8]) -> Option<String> {
    let len = u32::from_be_bytes(blob.get(..4)?.try_into().ok()?) as usize;
    String::from_utf8(blob.get(4..4 + len)?.to_vec()).ok()
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    fn go(p: &[char], t: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('*', rest)) => (0..=t.len()).any(|i| go(rest, &t[i..])),
            Some(('?', rest)) => !t.is_empty() && go(rest, &t[1..]),
            Some((c, rest)) => t.first() == Some(c) && go(rest, &t[1..]),
        }
    }
    go(&p, &t)
}

/// Does the host pattern list match `host:port`? Supports plain names, `[host]:port`, `*`/`?`
/// wildcards, `!negation` and hashed (`|1|salt|hash`) entries.
fn hosts_match(patterns: &str, host: &str, port: u16) -> bool {
    let bracketed = format!("[{host}]:{port}");
    // Non-default ports are written as [host]:port; the default port as the bare name.
    let canonical = if port == 22 {
        host.to_string()
    } else {
        bracketed.clone()
    };
    let mut matched = false;
    for pat in patterns.split(',') {
        let (neg, pat) = match pat.strip_prefix('!') {
            Some(p) => (true, p),
            None => (false, pat),
        };
        let hit = if let Some(rest) = pat.strip_prefix("|1|") {
            let mut it = rest.split('|');
            match (
                it.next().and_then(decode_b64),
                it.next().and_then(decode_b64),
            ) {
                (Some(salt), Some(hash)) => Hmac::<Sha1>::new_from_slice(&salt)
                    .map(|m| {
                        m.chain_update(canonical.as_bytes())
                            .verify_slice(&hash)
                            .is_ok()
                    })
                    .unwrap_or(false),
                _ => false,
            }
        } else {
            glob_match(&pat.to_ascii_lowercase(), &canonical.to_ascii_lowercase())
        };
        if hit {
            if neg {
                return false;
            }
            matched = true;
        }
    }
    matched
}

/// Compare the key the server presented (SSH wire-format blob) with the known entries.
pub fn verify(entries: &[Entry], host: &str, port: u16, server_blob: &[u8]) -> Verdict {
    let mut applicable = false;
    for e in entries {
        let applies = match &e.hosts {
            None => true,
            Some(h) => hosts_match(h, host, port),
        };
        if !applies {
            continue;
        }
        applicable = true;
        if e.key_blob == server_blob {
            return Verdict::Match;
        }
    }
    if applicable {
        Verdict::Mismatch
    } else {
        Verdict::NotFound
    }
}
