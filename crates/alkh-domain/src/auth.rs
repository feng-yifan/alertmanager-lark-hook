use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{DomainError, DomainResult};
use crate::ids::{AdminUserId, SessionId};
use crate::Timestamp;

/// 管理后台用户。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUser {
    pub id: AdminUserId,
    pub username: String,
    /// Argon2id PHC 字符串。
    pub password_hash: String,
    pub must_change_password: bool,
    pub created_at: Timestamp,
    pub last_login_at: Option<Timestamp>,
}

impl AdminUser {
    pub const MIN_PASSWORD_LEN: usize = 8;
    pub const MAX_USERNAME_LEN: usize = 64;

    pub fn validate_username(username: &str) -> DomainResult<()> {
        let u = username.trim();
        if u.is_empty() {
            return Err(DomainError::validation("用户名不能为空"));
        }
        if u.chars().count() > Self::MAX_USERNAME_LEN {
            return Err(DomainError::validation(format!(
                "用户名不能超过 {} 个字符",
                Self::MAX_USERNAME_LEN
            )));
        }
        if u.chars().any(|c| c.is_whitespace() || c == ':') {
            return Err(DomainError::validation("用户名不能包含空白或冒号"));
        }
        Ok(())
    }

    pub fn validate_password(password: &str) -> DomainResult<()> {
        if password.chars().count() < Self::MIN_PASSWORD_LEN {
            return Err(DomainError::validation(format!(
                "密码至少需要 {} 个字符",
                Self::MIN_PASSWORD_LEN
            )));
        }
        if password.chars().count() > 256 {
            return Err(DomainError::validation("密码过长"));
        }
        Ok(())
    }
}

/// 会话 token 的随机部分长度（base62）。
pub const SESSION_TOKEN_LEN: usize = 43;

pub fn generate_session_token() -> String {
    let mut rng = rand::thread_rng();
    (0..SESSION_TOKEN_LEN)
        .map(|_| char::from(rng.sample(Alphanumeric)))
        .collect()
}

/// 会话 token 只以 SHA-256 形式落库。
pub fn hash_session_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub user_id: AdminUserId,
    pub token_hash: String,
    pub expires_at: Timestamp,
    pub created_at: Timestamp,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
}

impl Session {
    pub const DEFAULT_TTL_HOURS: i64 = 12;

    pub fn issue(
        user_id: AdminUserId,
        now: Timestamp,
        user_agent: Option<String>,
        ip: Option<String>,
    ) -> (Self, String) {
        let token = generate_session_token();
        let session = Self {
            id: SessionId::new(),
            user_id,
            token_hash: hash_session_token(&token),
            expires_at: now + chrono::Duration::hours(Self::DEFAULT_TTL_HOURS),
            created_at: now,
            user_agent,
            ip,
        };
        (session, token)
    }

    pub fn is_valid_at(&self, now: Timestamp) -> bool {
        self.expires_at > now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_password_rejected() {
        assert!(AdminUser::validate_password("short").is_err());
        assert!(AdminUser::validate_password("longenough1").is_ok());
    }

    #[test]
    fn username_rules() {
        assert!(AdminUser::validate_username("").is_err());
        assert!(AdminUser::validate_username("a b").is_err());
        assert!(AdminUser::validate_username("admin").is_ok());
    }

    #[test]
    fn session_expiry() {
        let now = crate::now();
        let (s, token) = Session::issue(AdminUserId::new(), now, None, None);
        assert!(s.is_valid_at(now));
        assert_eq!(s.token_hash, hash_session_token(&token));
        assert!(!s.is_valid_at(now + chrono::Duration::hours(24)));
    }
}
