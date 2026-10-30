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

/// 卡片内时间变量的默认展示时区（IANA 名）。
pub const DEFAULT_TIMEZONE: &str = "UTC";

/// 卡片内时间变量的展示格式：`2026-10-13 08:00:00 +08:00`。
pub const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S %:z";

fn default_timezone() -> String {
    DEFAULT_TIMEZONE.to_string()
}

/// 时间展示时区（IANA 名，如 `Asia/Shanghai`）。
///
/// AlertManager 推来的时间戳（`startsAt` / `endsAt`）一律是 UTC —— 这是数据层口径，
/// 本服务不改写它；但卡片里给人看的字符串按本类型换算，取值来自应用设置
/// （[`AppSettings::timezone`]，默认 [`DEFAULT_TIMEZONE`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayTimeZone(chrono_tz::Tz);

impl Default for DisplayTimeZone {
    fn default() -> Self {
        Self(chrono_tz::UTC)
    }
}

impl DisplayTimeZone {
    /// 解析 IANA 时区名；非法名返回 `None`（`UTC` 是合法值）。
    pub fn parse(name: &str) -> Option<Self> {
        name.trim().parse::<chrono_tz::Tz>().ok().map(Self)
    }

    /// 规范化后的 IANA 名（如 `Asia/Shanghai`）。
    pub fn name(self) -> &'static str {
        self.0.name()
    }

    /// 把 UTC 时间戳渲染成该时区下的展示字符串（见 [`TIME_FORMAT`]）。
    pub fn format(self, t: Timestamp) -> String {
        t.with_timezone(&self.0).format(TIME_FORMAT).to_string()
    }

    /// 同 [`Self::format`]，`None` 渲染为空串。
    pub fn format_opt(self, t: Option<Timestamp>) -> String {
        t.map(|t| self.format(t)).unwrap_or_default()
    }
}

/// 应用设置（单例聚合根）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub lark_app_id: Option<String>,
    /// 加密后的 app_secret；永不回显明文。
    pub lark_app_secret_enc: Option<EncryptedSecret>,
    /// 卡片回调（`card.action.trigger`）验签用的 Verification Token；加密存储、永不回显明文。
    #[serde(default)]
    pub lark_verification_token_enc: Option<EncryptedSecret>,
    pub lark_base_url: String,
    /// 生成 hook 绝对地址用的对外基址。
    pub public_base_url: Option<String>,
    /// 卡片内时间的展示时区（IANA 名，默认 `UTC`）；只影响展示字符串。
    #[serde(default = "default_timezone")]
    pub timezone: String,
    pub retention: RetentionPolicy,
    pub updated_at: Timestamp,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            lark_app_id: None,
            lark_app_secret_enc: None,
            lark_verification_token_enc: None,
            lark_base_url: "https://open.feishu.cn".into(),
            public_base_url: None,
            timezone: default_timezone(),
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
        if DisplayTimeZone::parse(&self.timezone).is_none() {
            return Err(DomainError::validation(format!(
                "timezone 非法：需 IANA 时区名（如 UTC / Asia/Shanghai），收到 `{}`",
                self.timezone
            )));
        }
        self.retention.validate()
    }

    /// 生效的展示时区（`timezone` 经 [`Self::validate`] 校验过；异常值退化为 UTC）。
    pub fn display_timezone(&self) -> DisplayTimeZone {
        DisplayTimeZone::parse(&self.timezone).unwrap_or_default()
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
    use chrono::TimeZone as _;

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

    #[test]
    fn timezone_must_be_iana_name() {
        assert_eq!(AppSettings::default().timezone, DEFAULT_TIMEZONE);
        let bad = AppSettings {
            timezone: "Asia/Shangai".into(),
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        let good = AppSettings {
            timezone: "Asia/Shanghai".into(),
            ..Default::default()
        };
        assert!(good.validate().is_ok());
    }

    #[test]
    fn display_timezone_renders_configured_zone() {
        let t = chrono::Utc
            .with_ymd_and_hms(2026, 10, 16, 3, 4, 19)
            .unwrap();
        assert_eq!(
            DisplayTimeZone::default().format(t),
            "2026-10-16 03:04:19 +00:00"
        );

        let cst = DisplayTimeZone::parse("Asia/Shanghai").unwrap();
        assert_eq!(cst.name(), "Asia/Shanghai");
        assert_eq!(cst.format(t), "2026-10-16 11:04:19 +08:00");
        assert_eq!(cst.format_opt(None), "");
        assert_eq!(cst.format_opt(Some(t)), "2026-10-16 11:04:19 +08:00");
    }
}
