use serde::{Deserialize, Serialize};

use crate::error::{DomainError, DomainResult};
use crate::Timestamp;

/// AES-256-GCM 密文 + nonce（均 base64）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedSecret {
    /// 12 字节 nonce 的 base64。
    pub nonce: String,
    /// 密文（含 GCM tag）的 base64。
    pub ciphertext: String,
}

/// 数据保留策略（天）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub alert_group_days: u32,
    pub delivery_days: u32,
    pub audit_days: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            alert_group_days: 30,
            delivery_days: 30,
            audit_days: 14,
        }
    }
}

impl RetentionPolicy {
    pub const MAX_DAYS: u32 = 3650;

    pub fn validate(&self) -> DomainResult<()> {
        for (name, v) in [
            ("alert_group_days", self.alert_group_days),
            ("delivery_days", self.delivery_days),
            ("audit_days", self.audit_days),
        ] {
            if v == 0 || v > Self::MAX_DAYS {
                return Err(DomainError::validation(format!(
                    "{name} 必须在 1..={} 之间",
                    Self::MAX_DAYS
                )));
            }
        }
        Ok(())
    }
}

/// 应用设置（单例聚合根）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub lark_app_id: Option<String>,
    /// 加密后的 app_secret；永不回显明文。
    pub lark_app_secret_enc: Option<EncryptedSecret>,
    pub lark_base_url: String,
    /// 生成 hook 绝对地址用的对外基址。
    pub public_base_url: Option<String>,
    pub retention: RetentionPolicy,
    pub updated_at: Timestamp,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            lark_app_id: None,
            lark_app_secret_enc: None,
            lark_base_url: "https://open.feishu.cn".into(),
            public_base_url: None,
            retention: RetentionPolicy::default(),
            updated_at: crate::now(),
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> DomainResult<()> {
        if let Some(id) = self.lark_app_id.as_deref() {
            let id = id.trim();
            if !id.is_empty() && id.len() > 128 {
                return Err(DomainError::validation("app_id 过长"));
            }
        }
        let base = self.lark_base_url.trim();
        if base.is_empty() {
            return Err(DomainError::validation("lark_base_url 不能为空"));
        }
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(DomainError::validation(
                "lark_base_url 必须以 http(s):// 开头",
            ));
        }
        if let Some(p) = self.public_base_url.as_deref() {
            let p = p.trim();
            if !p.is_empty() && !p.starts_with("http://") && !p.starts_with("https://") {
                return Err(DomainError::validation(
                    "public_base_url 必须以 http(s):// 开头",
                ));
            }
        }
        self.retention.validate()
    }

    pub fn is_configured(&self) -> bool {
        self.lark_app_id
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
            && self.lark_app_secret_enc.is_some()
    }

    /// 生成某 token 的完整 hook URL（无 public_base_url 时退化为相对路径）。
    pub fn hook_url(&self, token: &str) -> String {
        let path = crate::hook::hook_path(token);
        match self
            .public_base_url
            .as_deref()
            .map(|s| s.trim_end_matches('/'))
        {
            Some(base) if !base.is_empty() => format!("{base}{path}"),
            _ => path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_must_be_absolute() {
        let bad = AppSettings {
            lark_base_url: "open.feishu.cn".into(),
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        let good = AppSettings {
            lark_base_url: "https://open.feishu.cn".into(),
            ..Default::default()
        };
        assert!(good.validate().is_ok());
    }

    #[test]
    fn retention_bounds() {
        assert!(RetentionPolicy {
            audit_days: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(RetentionPolicy {
            audit_days: 14,
            ..Default::default()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn hook_url_uses_public_base() {
        let mut s = AppSettings::default();
        assert_eq!(s.hook_url("alkh_x"), "/api/v1/hooks/alertmanager/alkh_x");
        s.public_base_url = Some("https://alkh.example.com/".into());
        assert_eq!(
            s.hook_url("alkh_x"),
            "https://alkh.example.com/api/v1/hooks/alertmanager/alkh_x"
        );
    }
}
