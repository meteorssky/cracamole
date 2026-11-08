//! Web 與桌面應用程式共用的管理員帳密驗證。
use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use std::sync::Arc;
use tokio::sync::Semaphore;
use zeroize::Zeroize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    Busy,
    Internal,
}

pub struct Credentials {
    username: String,
    password_hash: Arc<String>,
    hashing: Arc<Semaphore>,
}

impl Credentials {
    pub async fn new(username: String, mut password: String) -> Result<Self, String> {
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
            hashing: Arc::new(Semaphore::new(2)),
        })
    }

    pub async fn verify(&self, username: &str, mut password: String) -> Result<bool, VerifyError> {
        if password.len() > 256 {
            password.zeroize();
            return Ok(false);
        }
        let Ok(permit) = self.hashing.clone().try_acquire_owned() else {
            password.zeroize();
            return Err(VerifyError::Busy);
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
        .map_err(|_| VerifyError::Internal)?;
        Ok(valid && valid_user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn validates_credentials_and_rejects_invalid_input() {
        assert!(Credentials::new("admin".into(), "short".into())
            .await
            .is_err());
        let c = Credentials::new("admin".into(), "correct horse battery staple".into())
            .await
            .unwrap();
        assert!(c
            .verify("admin", "correct horse battery staple".into())
            .await
            .unwrap());
        assert!(!c
            .verify("other", "correct horse battery staple".into())
            .await
            .unwrap());
        assert!(!c.verify("admin", "wrong".into()).await.unwrap());
        assert!(!c.verify("admin", "x".repeat(257)).await.unwrap());
    }
}
