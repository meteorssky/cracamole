use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use axum::http::{header, HeaderMap, StatusCode};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroize;

pub const COOKIE: &str = "cracamole_session";
pub const SESSION_TTL: Duration = Duration::from_secs(8 * 60 * 60);

pub struct Session {
    pub expires: Instant,
    pub cancel: CancellationToken,
}
pub struct Auth {
    username: String,
    password_hash: Arc<String>,
    sessions: Mutex<HashMap<String, Session>>,
    attempts: Mutex<HashMap<IpAddr, (Instant, u32)>>,
    hashing: Arc<Semaphore>,
    pub secure_cookie: bool,
}

impl Auth {
    pub async fn new(
        username: String,
        mut password: String,
        secure_cookie: bool,
    ) -> Result<Self, String> {
        if username.is_empty()
            || username.len() > 128
            || password.chars().count() < 12
            || password.len() > 256
        {
            password.zeroize();
            return Err("管理員帳號不可空白；密碼至少 12 字元，且不超過 256 bytes".into());
        }
        let hash = tokio::task::spawn_blocking(move || {
            let hash = Argon2::default()
                .hash_password(password.as_bytes())
                .map(|h| h.to_string())
                .map_err(|e| e.to_string());
            password.zeroize();
            hash
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(Self {
            username,
            password_hash: Arc::new(hash),
            sessions: Mutex::new(HashMap::new()),
            attempts: Mutex::new(HashMap::new()),
            hashing: Arc::new(Semaphore::new(2)),
            secure_cookie,
        })
    }

    pub async fn login(
        &self,
        ip: IpAddr,
        username: String,
        mut password: String,
    ) -> Result<String, StatusCode> {
        let now = Instant::now();
        {
            let mut attempts = self.attempts.lock().await;
            attempts.retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(60));
            if attempts.len() >= 1024 && !attempts.contains_key(&ip) {
                password.zeroize();
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
            let attempt = attempts.entry(ip).or_insert((now, 0));
            attempt.1 += 1;
            if attempt.1 > 10 {
                password.zeroize();
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
        }
        if password.len() > 256 {
            password.zeroize();
            return Err(StatusCode::UNAUTHORIZED);
        }
        let Ok(permit) = self.hashing.clone().try_acquire_owned() else {
            password.zeroize();
            return Err(StatusCode::TOO_MANY_REQUESTS);
        };
        let hash = self.password_hash.clone();
        let valid_user = username == self.username;
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let valid = PasswordHash::new(&hash).ok().is_some_and(|h| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &h)
                    .is_ok()
            });
            password.zeroize();
            valid
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !valid || !valid_user {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, session| {
            let keep = session.expires > now;
            if !keep {
                session.cancel.cancel();
            }
            keep
        });
        if sessions.len() >= 128 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        sessions.insert(
            token.clone(),
            Session {
                expires: Instant::now() + SESSION_TTL,
                cancel: CancellationToken::new(),
            },
        );
        Ok(token)
    }

    pub async fn authorize(
        &self,
        headers: &HeaderMap,
    ) -> Result<(Instant, CancellationToken), StatusCode> {
        let token = token(headers).ok_or(StatusCode::UNAUTHORIZED)?;
        let mut sessions = self.sessions.lock().await;
        match sessions.get(token) {
            Some(session) if session.expires > Instant::now() => {
                Ok((session.expires, session.cancel.clone()))
            }
            _ => {
                if let Some(session) = sessions.remove(token) {
                    session.cancel.cancel();
                }
                Err(StatusCode::UNAUTHORIZED)
            }
        }
    }

    pub async fn logout(&self, headers: &HeaderMap) {
        if let Some(token) = token(headers) {
            if let Some(session) = self.sessions.lock().await.remove(token) {
                session.cancel.cancel();
            }
        }
    }
    pub fn cookie(&self, token: &str) -> String {
        format!(
            "{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}{}",
            SESSION_TTL.as_secs(),
            if self.secure_cookie { "; Secure" } else { "" }
        )
    }
}

fn token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == COOKIE).then_some(value)
        })
}

pub fn same_origin(headers: &HeaderMap) -> Result<(), StatusCode> {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::FORBIDDEN)?;
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::FORBIDDEN)?;
    let uri = origin
        .parse::<axum::http::Uri>()
        .map_err(|_| StatusCode::FORBIDDEN)?;
    if !matches!(uri.scheme_str(), Some("http" | "https"))
        || uri
            .authority()
            .is_none_or(|a| !a.as_str().eq_ignore_ascii_case(host))
    {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn web_expired_session_revokes_the_cookie_and_active_tunnel() {
        let auth = Auth::new("admin".into(), "a-long-admin-password".into(), true)
            .await
            .unwrap();
        let token = auth
            .login(
                "127.0.0.1".parse().unwrap(),
                "admin".into(),
                "a-long-admin-password".into(),
            )
            .await
            .unwrap();
        assert!(auth.cookie(&token).ends_with("; Secure"));
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, format!("{COOKIE}={token}").parse().unwrap());
        let (_, cancel) = auth.authorize(&headers).await.unwrap();
        auth.sessions.lock().await.get_mut(&token).unwrap().expires =
            Instant::now() - Duration::from_secs(1);
        assert_eq!(
            auth.authorize(&headers).await.unwrap_err(),
            StatusCode::UNAUTHORIZED
        );
        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    async fn web_bootstrap_has_no_default_or_short_admin_password() {
        assert!(Auth::new("admin".into(), "".into(), false).await.is_err());
        assert!(Auth::new("admin".into(), "short".into(), false)
            .await
            .is_err());
        assert!(Auth::new("admin".into(), "四個字元".into(), false)
            .await
            .is_err());
    }
}
