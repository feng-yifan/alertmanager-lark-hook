use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{DomainError, DomainResult};
use crate::ids::{ChannelId, HookId};
use crate::Timestamp;

/// hook token 的固定前缀，便于人眼识别与日志脱敏。
pub const TOKEN_PREFIX: &str = "alkh_";
/// 随机部分长度（base62 字符数，约 256 bit 熵）。
pub const TOKEN_RANDOM_LEN: usize = 43;

/// 生成一个新的 hook token（明文只在创建/轮换时返回一次）。
pub fn generate_token() -> String {
    let mut rng = rand::thread_rng();
    let body: String = (0..TOKEN_RANDOM_LEN)
        .map(|_| char::from(rng.sample(Alphanumeric)))
        .collect();
    format!("{TOKEN_PREFIX}{body}")
}

/// token 的 SHA-256 十六进制摘要；数据库中只存该值。
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// 用于 UI 展示的 token 前缀片段（永不泄露完整 token）。
pub fn token_prefix(token: &str) -> String {
    let body = token.strip_prefix(TOKEN_PREFIX).unwrap_or(token);
    let head: String = body.chars().take(4).collect();
    format!("{TOKEN_PREFIX}{head}")
}

/// AlertManager receiver 需要填写的相对路径。
pub fn hook_path(token: &str) -> String {
    format!("/api/v1/hooks/alertmanager/{token}")
}

/// 接入端点聚合根：一个 Hook = AlertManager 配置里的一个 receiver 目标。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    pub id: HookId,
    pub channel_id: ChannelId,
    pub name: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub enabled: bool,
    pub last_called_at: Option<Timestamp>,
    pub call_count: u64,
    pub created_at: Timestamp,
    pub rotated_at: Option<Timestamp>,
}

impl Hook {
    pub const MAX_NAME_LEN: usize = 64;

    /// 创建 Hook，返回 (Hook, 明文 token)。
    pub fn issue(channel_id: ChannelId, name: impl Into<String>) -> (Self, String) {
        let token = generate_token();
        let now = crate::now();
        let hook = Self {
            id: HookId::new(),
            channel_id,
            name: name.into(),
            token_hash: hash_token(&token),
            token_prefix: token_prefix(&token),
            enabled: true,
            last_called_at: None,
            call_count: 0,
            created_at: now,
            rotated_at: None,
        };
        (hook, token)
    }

    /// 轮换 token，返回新的明文 token。
    pub fn rotate(&mut self) -> String {
        let token = generate_token();
        self.token_hash = hash_token(&token);
        self.token_prefix = token_prefix(&token);
        self.rotated_at = Some(crate::now());
        token
    }

    pub fn validate(&self) -> DomainResult<()> {
        if self.name.trim().is_empty() {
            return Err(DomainError::validation("Hook 名称不能为空"));
        }
        if self.name.chars().count() > Self::MAX_NAME_LEN {
            return Err(DomainError::validation(format!(
                "Hook 名称不能超过 {} 个字符",
                Self::MAX_NAME_LEN
            )));
        }
        Ok(())
    }

    pub fn matches_token(&self, token: &str) -> bool {
        // 常数时间比较，避免通过响应时间推断 token。
        let candidate = hash_token(token);
        constant_time_eq(candidate.as_bytes(), self.token_hash.as_bytes())
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_token_matches_and_hash_is_stable() {
        let (hook, token) = Hook::issue(ChannelId::new(), "ops");
        assert!(token.starts_with(TOKEN_PREFIX));
        assert_eq!(token.len(), TOKEN_PREFIX.len() + TOKEN_RANDOM_LEN);
        assert!(hook.matches_token(&token));
        assert!(!hook.matches_token("alkh_wrong"));
        assert_eq!(hook.token_hash, hash_token(&token));
    }

    #[test]
    fn rotate_invalidates_old_token() {
        let (mut hook, old) = Hook::issue(ChannelId::new(), "ops");
        let new = hook.rotate();
        assert!(!hook.matches_token(&old));
        assert!(hook.matches_token(&new));
        assert_ne!(old, new);
    }

    #[test]
    fn secret_is_not_stored_in_plaintext() {
        let (hook, token) = Hook::issue(ChannelId::new(), "ops");
        assert_ne!(hook.token_hash, token);
        assert!(hook.token_prefix.len() < token.len());
    }
}
