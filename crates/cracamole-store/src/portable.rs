//! Web 與桌面版共用的連線設定交換格式，不包含已保存的機密欄位。
use crate::{validate, Connection, Draft, Invalid, PublicConnection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FORMAT: &str = "cracamole-connections";
const VERSION: u32 = 1;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 100;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundle {
    format: String,
    version: u32,
    connections: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    protocol: String,
    params: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PortableError {
    TooLarge,
    InvalidFormat,
    UnsupportedVersion,
    InvalidCount,
    SecretField,
    InvalidEntry(Invalid),
}

fn checked(entry: Entry) -> Result<Draft, PortableError> {
    if entry
        .params
        .keys()
        .any(|key| super::SECRETS.contains(&key.as_str()))
    {
        return Err(PortableError::SecretField);
    }
    let draft = Draft {
        name: entry.name,
        protocol: entry.protocol,
        params: entry.params,
    };
    validate(&draft).map_err(PortableError::InvalidEntry)?;
    Ok(draft)
}

/// 將公開設定序列化；ID、機密與「已設定機密」的中繼資料不會輸出。
pub fn export(connections: &[PublicConnection]) -> Result<String, PortableError> {
    if connections.is_empty() || connections.len() > MAX_ENTRIES {
        return Err(PortableError::InvalidCount);
    }
    let input_bytes = connections.iter().fold(0usize, |total, connection| {
        connection.params.iter().fold(
            total
                .saturating_add(connection.name.len())
                .saturating_add(connection.protocol.len()),
            |size, (key, value)| size.saturating_add(key.len()).saturating_add(value.len()),
        )
    });
    if input_bytes > MAX_BYTES {
        return Err(PortableError::TooLarge);
    }
    let mut entries = Vec::with_capacity(connections.len());
    for connection in connections {
        let entry = Entry {
            name: connection.name.clone(),
            protocol: connection.protocol.clone(),
            params: connection.params.clone(),
        };
        let draft = checked(entry)?;
        entries.push(Entry {
            name: draft.name,
            protocol: draft.protocol,
            params: draft.params,
        });
    }
    let json = serde_json::to_string_pretty(&Bundle {
        format: FORMAT.into(),
        version: VERSION,
        connections: entries,
    })
    .map_err(|_| PortableError::InvalidFormat)?;
    if json.len() > MAX_BYTES {
        return Err(PortableError::TooLarge);
    }
    Ok(json)
}

/// 從完整連線產生公開設定後再匯出，不複製機密至交換格式。
pub fn export_connections(connections: &[Connection]) -> Result<String, PortableError> {
    if connections.is_empty() || connections.len() > MAX_ENTRIES {
        return Err(PortableError::InvalidCount);
    }
    let input_bytes = connections.iter().fold(0usize, |total, connection| {
        connection
            .params
            .iter()
            .filter(|(key, _)| !super::SECRETS.contains(&key.as_str()))
            .fold(
                total
                    .saturating_add(connection.name.len())
                    .saturating_add(connection.protocol.len()),
                |size, (key, value)| size.saturating_add(key.len()).saturating_add(value.len()),
            )
    });
    if input_bytes > MAX_BYTES {
        return Err(PortableError::TooLarge);
    }
    export(
        &connections
            .iter()
            .map(Connection::public)
            .collect::<Vec<_>>(),
    )
}

/// 匯入只建立草稿；呼叫端須讓使用者確認後才保存。
pub fn import(json: &str) -> Result<Vec<Draft>, PortableError> {
    if json.len() > MAX_BYTES {
        return Err(PortableError::TooLarge);
    }
    if !json.trim_start().starts_with('{') {
        return import_uris(json);
    }
    let bundle: Bundle = serde_json::from_str(json).map_err(|_| PortableError::InvalidFormat)?;
    if bundle.format != FORMAT {
        return Err(PortableError::InvalidFormat);
    }
    if bundle.version != VERSION {
        return Err(PortableError::UnsupportedVersion);
    }
    if bundle.connections.is_empty() || bundle.connections.len() > MAX_ENTRIES {
        return Err(PortableError::InvalidCount);
    }
    bundle.connections.into_iter().map(checked).collect()
}

/// 端點 URI 只交換協定、主機、連接埠及帳號；其餘設定請使用 JSON。
pub fn export_uris(connections: &[PublicConnection]) -> Result<String, PortableError> {
    if connections.is_empty() || connections.len() > MAX_ENTRIES {
        return Err(PortableError::InvalidCount);
    }
    let mut lines = Vec::with_capacity(connections.len());
    for entry in connections {
        let checked = checked(Entry {
            name: entry.name.clone(),
            protocol: entry.protocol.clone(),
            params: entry.params.clone(),
        })?;
        let host = checked
            .params
            .get("hostname")
            .ok_or(PortableError::InvalidFormat)?;
        let host = uri_host(host)?;
        let username = checked
            .params
            .get("username")
            .map(String::as_str)
            .unwrap_or("");
        let domain = if checked.protocol == "rdp" {
            checked
                .params
                .get("domain")
                .map(String::as_str)
                .unwrap_or("")
        } else {
            ""
        };
        if checked.protocol == "rdp"
            && (username.contains('\\')
                || domain.contains('\\')
                || (!domain.is_empty() && username.is_empty()))
        {
            return Err(PortableError::InvalidFormat);
        }
        let username = if domain.is_empty() {
            username.to_string()
        } else {
            format!("{domain}\\{username}")
        };
        let mut authority = String::new();
        if !username.is_empty() {
            if username.contains(':') || username.chars().any(char::is_control) {
                return Err(PortableError::InvalidFormat);
            }
            for b in username.bytes() {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                    authority.push(char::from(b));
                } else {
                    use std::fmt::Write;
                    write!(authority, "%{b:02X}").map_err(|_| PortableError::InvalidFormat)?;
                }
            }
            authority.push('@');
        }
        authority.push_str(&host);
        if let Some(port) = checked.params.get("port").filter(|port| !port.is_empty()) {
            authority.push(':');
            authority.push_str(port);
        }
        lines.push(format!("{}://{}", checked.protocol, authority));
    }
    let text = lines.join("\n");
    if text.len() > MAX_BYTES {
        return Err(PortableError::TooLarge);
    }
    Ok(text)
}

fn uri_host(host: &str) -> Result<String, PortableError> {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        return Ok(format!("[{host}]"));
    }
    if host.is_empty()
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
    {
        return Err(PortableError::InvalidFormat);
    }
    Ok(host.to_owned())
}

fn uri_user(encoded: &str) -> Result<String, PortableError> {
    let bytes = encoded.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let pair = bytes
                .get(i + 1..i + 3)
                .ok_or(PortableError::InvalidFormat)?;
            let hex = std::str::from_utf8(pair).map_err(|_| PortableError::InvalidFormat)?;
            output.push(u8::from_str_radix(hex, 16).map_err(|_| PortableError::InvalidFormat)?);
            i += 3;
        } else {
            let b = bytes[i];
            if !(b.is_ascii_alphanumeric() || b"-._~".contains(&b)) {
                return Err(PortableError::InvalidFormat);
            }
            output.push(b);
            i += 1;
        }
    }
    let user = String::from_utf8(output).map_err(|_| PortableError::InvalidFormat)?;
    if user.is_empty() || user.contains(':') || user.chars().any(char::is_control) {
        return Err(PortableError::InvalidFormat);
    }
    Ok(user)
}

/// 解析一行一筆的端點 URI；拒絕密碼、查詢、路徑與片段。
pub fn import_uris(text: &str) -> Result<Vec<Draft>, PortableError> {
    if text.len() > MAX_BYTES {
        return Err(PortableError::TooLarge);
    }
    let lines: Vec<_> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() || lines.len() > MAX_ENTRIES {
        return Err(PortableError::InvalidCount);
    }
    lines
        .into_iter()
        .map(|line| {
            let (protocol, authority) = line
                .trim()
                .split_once("://")
                .ok_or(PortableError::InvalidFormat)?;
            if !["ssh", "telnet", "rdp", "vnc"].contains(&protocol)
                || authority.contains(['/', '?', '#', '\\'])
            {
                return Err(PortableError::InvalidFormat);
            }
            let (username, target) = match authority.split_once('@') {
                Some((user, target)) if !target.contains('@') => (Some(uri_user(user)?), target),
                Some(_) => return Err(PortableError::InvalidFormat),
                None => (None, authority),
            };
            let (host, port) = if let Some(rest) = target.strip_prefix('[') {
                let (host, rest) = rest.split_once(']').ok_or(PortableError::InvalidFormat)?;
                host.parse::<std::net::Ipv6Addr>()
                    .map_err(|_| PortableError::InvalidFormat)?;
                let port = if rest.is_empty() {
                    None
                } else {
                    Some(rest.strip_prefix(':').ok_or(PortableError::InvalidFormat)?)
                };
                (host, port)
            } else {
                let (host, port) = match target.split_once(':') {
                    Some((host, port)) => (host, Some(port)),
                    None => (target, None),
                };
                uri_host(host)?;
                (host, port)
            };
            if let Some(port) = port {
                if port.is_empty()
                    || !port.bytes().all(|b| b.is_ascii_digit())
                    || !port.parse::<u16>().is_ok_and(|p| p > 0)
                {
                    return Err(PortableError::InvalidFormat);
                }
            }
            let mut params = BTreeMap::from([("hostname".into(), host.into())]);
            if let Some(user) = username {
                if protocol == "rdp" && user.contains('\\') {
                    let (domain, user) =
                        user.split_once('\\').ok_or(PortableError::InvalidFormat)?;
                    if domain.is_empty() || user.is_empty() || user.contains('\\') {
                        return Err(PortableError::InvalidFormat);
                    }
                    params.insert("domain".into(), domain.into());
                    params.insert("username".into(), user.into());
                } else {
                    params.insert("username".into(), user);
                }
            }
            if let Some(port) = port {
                params.insert("port".into(), port.into());
            }
            let name = match params.get("username") {
                Some(user) => format!("{user}@{host}"),
                None => host.to_string(),
            };
            checked(Entry {
                name,
                protocol: protocol.into(),
                params,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(protocol: &str) -> Connection {
        Connection {
            id: "private-id".into(),
            name: format!("群組 / {protocol}"),
            protocol: protocol.into(),
            params: BTreeMap::from([
                ("hostname".into(), "example.test".into()),
                ("username".into(), "alice".into()),
                ("password".into(), "secret-password".into()),
            ]),
        }
    }

    #[test]
    fn uris_round_trip_endpoints_and_reject_ambiguous_credentials() {
        let mut ssh = connection("ssh").public();
        ssh.params.insert("hostname".into(), "2001:db8::1".into());
        ssh.params.insert("username".into(), "使用者@local".into());
        ssh.params.insert("port".into(), "2222".into());
        let mut rdp = connection("rdp").public();
        rdp.params.insert("domain".into(), "CORP".into());
        let text = export_uris(&[ssh, rdp]).unwrap();
        assert!(text.contains("ssh://%E4%BD%BF%E7%94%A8%E8%80%85%40local@[2001:db8::1]:2222"));
        assert!(text.contains("rdp://CORP%5Calice@example.test"));
        let entries = import_uris(&text).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].params["username"], "使用者@local");
        assert_eq!(entries[0].params["hostname"], "2001:db8::1");
        assert_eq!(entries[0].params["port"], "2222");
        assert_eq!(entries[1].params["domain"], "CORP");
        assert_eq!(entries[1].params["username"], "alice");
        for input in [
            "ssh://user:secret@host",
            "ssh://user%3Asecret@host",
            "ssh://host/path",
            "ssh://host?password=secret",
            "ssh://host#fragment",
            "ssh://host:0",
            "ssh://2001:db8::1",
            "ssh://user%ZZ@host",
            "rdp://CORP%5C@host",
            "rdp://CORP%5Calice%5Cextra@host",
            "ssh://host@other@host",
            "http://host",
            "ssh://host\\evil",
        ] {
            assert!(import_uris(input).is_err(), "accepted {input}");
        }
        assert!(import_uris(&vec!["ssh://host"; 101].join("\n")).is_err());
    }

    #[test]
    fn round_trip_omits_credentials_ids_and_secret_metadata() {
        let mut entries = [
            connection("ssh"),
            connection("rdp"),
            connection("vnc"),
            connection("telnet"),
        ];
        entries[0]
            .params
            .insert("private-key".into(), "secret-private-key".into());
        entries[0]
            .params
            .insert("passphrase".into(), "secret-passphrase".into());
        entries[0]
            .params
            .insert("host-key".into(), "ssh-ed25519 public-fixture".into());
        let text = export_connections(&entries).unwrap();
        for secret in [
            "secret-password",
            "secret-private-key",
            "secret-passphrase",
            "password",
            "private-key",
            "passphrase",
            "private-id",
            "configured_secrets",
        ] {
            assert!(!text.contains(secret));
        }
        let drafts = import(&text).unwrap();
        assert_eq!(drafts.len(), 4);
        for (draft, entry) in drafts.iter().zip(entries) {
            assert_eq!(draft.name, entry.name);
            assert_eq!(draft.protocol, entry.protocol);
            assert_eq!(draft.params, entry.public().params);
        }
        assert_eq!(
            drafts[0].params.get("host-key").map(String::as_str),
            Some("ssh-ed25519 public-fixture")
        );
    }

    #[test]
    fn rejects_secrets_unknown_fields_versions_and_limits() {
        let entry = connection("ssh");
        let text = export_connections(&[entry]).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["connections"][0]["params"]["password"] = "secret".into();
        assert_eq!(
            import(&value.to_string()).err(),
            Some(PortableError::SecretField)
        );
        value["connections"][0]["params"]
            .as_object_mut()
            .unwrap()
            .remove("password");
        value["connections"][0]["id"] = "unexpected".into();
        assert_eq!(
            import(&value.to_string()).err(),
            Some(PortableError::InvalidFormat)
        );
        value["connections"][0]
            .as_object_mut()
            .unwrap()
            .remove("id");
        value["version"] = 2.into();
        assert_eq!(
            import(&value.to_string()).err(),
            Some(PortableError::UnsupportedVersion)
        );
        assert_eq!(
            import(&" ".repeat(MAX_BYTES + 1)).err(),
            Some(PortableError::TooLarge)
        );
        assert_eq!(export(&[]), Err(PortableError::InvalidCount));
        let mut forged = connection("ssh").public();
        forged.params.insert("password".into(), "secret".into());
        assert_eq!(export(&[forged]), Err(PortableError::SecretField));
    }
}
