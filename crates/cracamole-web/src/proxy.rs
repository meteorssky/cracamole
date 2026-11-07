//! Only explicitly trusted TCP peers may supply the client address, in one overwritten header.

use crate::ApiError;
use axum::http::{HeaderMap, StatusCode};
use std::net::IpAddr;

pub fn parse_trusted_proxies(value: &str) -> Result<Vec<IpAddr>, String> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|s| {
            s.trim()
                .parse()
                .map_err(|_| "CRACAMOLE_TRUSTED_PROXIES 必須是逗號分隔的代理 IP 位址".into())
        })
        .collect()
}

pub(crate) fn client_ip(
    headers: &HeaderMap,
    peer: IpAddr,
    trusted: &[IpAddr],
) -> Result<IpAddr, ApiError> {
    if !trusted.contains(&peer) {
        return Ok(peer);
    }
    let mut values = headers.get_all("x-real-ip").iter();
    let parsed = values
        .next()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    match (parsed, values.next()) {
        (Some(ip), None) => Ok(ip),
        _ => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "可信代理未提供有效的 X-Real-IP",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_forwarded_addresses_cannot_change_the_limit_bucket() {
        let peer = "192.0.2.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.8".parse().unwrap());
        headers.insert("x-forwarded-for", "203.0.113.9".parse().unwrap());
        assert_eq!(client_ip(&headers, peer, &[]).unwrap(), peer);
        assert_eq!(
            client_ip(&headers, peer, &[peer]).unwrap(),
            "203.0.113.8".parse::<IpAddr>().unwrap()
        );
        headers.append("x-real-ip", "203.0.113.7".parse().unwrap());
        assert!(client_ip(&headers, peer, &[peer]).is_err());
        headers.remove("x-real-ip");
        assert!(client_ip(&headers, peer, &[peer]).is_err());
        headers.insert("x-real-ip", "203.0.113.8,203.0.113.9".parse().unwrap());
        assert!(client_ip(&headers, peer, &[peer]).is_err());
    }
    #[test]
    fn configuration_is_explicit_and_invalid_entries_fail_closed() {
        assert!(parse_trusted_proxies("").unwrap().is_empty());
        assert_eq!(parse_trusted_proxies("127.0.0.1, ::1").unwrap().len(), 2);
        for bad in ["*", "0.0.0.0/0", "localhost", "127.0.0.1,"] {
            assert!(parse_trusted_proxies(bad).is_err());
        }
    }
}
