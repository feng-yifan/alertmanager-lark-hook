//! 应用设置用例：飞书凭据（加密存储）、公开地址、保留策略。

use alkh_domain::audit::{action as audit_action, AuditActor, AuditLog};
use alkh_domain::ports::LarkCredential;
use alkh_domain::settings::{AppSettings, DisplayTimeZone, RetentionPolicy};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::deps::Deps;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub lark_app_id: Option<String>,
    /// 只暴露"是否已配置"，永不回显明文。
    pub app_secret_set: bool,
    /// 卡片回调 Verification Token 是否已配置（永不回显明文）。
    pub verification_token_set: bool,
    pub lark_base_url: String,
    pub public_base_url: Option<String>,
    /// 卡片内时间的展示时区（IANA 名，默认 `UTC`）。
    pub timezone: String,
    pub retention: RetentionPolicy,
    pub updated_at: alkh_domain::Timestamp,
    pub configured: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsInput {
    #[serde(default)]
    pub lark_app_id: Option<String>,
    /// `null`（缺省）表示不变；空串表示清除；非空表示设置。
    #[serde(default)]
    pub app_secret: Option<String>,
    /// 卡片回调（`card.action.trigger`）验签用的 Verification Token；
    /// `null`（缺省）不变，空串清除，非空设置。
    #[serde(default)]
    pub verification_token: Option<String>,
    pub lark_base_url: String,
    #[serde(default)]
    pub public_base_url: Option<String>,
    /// 卡片内时间的展示时区（IANA 名）；`null`（缺省）表示不变。
    #[serde(default)]
    pub timezone: Option<String>,
    pub retention: RetentionPolicy,
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialCheck {
    pub ok: bool,
    pub chat_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub struct SettingsService {
    deps: Deps,
}

impl SettingsService {
    pub fn new(deps: Deps) -> Self {
        Self { deps }
    }

    pub async fn view(&self) -> AppResult<SettingsView> {
        Ok(Self::to_view(&self.deps.settings.get().await?))
    }

    fn to_view(s: &AppSettings) -> SettingsView {
        SettingsView {
            lark_app_id: s.lark_app_id.clone(),
            app_secret_set: s.lark_app_secret_enc.is_some(),
            verification_token_set: s.lark_verification_token_enc.is_some(),
            lark_base_url: s.lark_base_url.clone(),
            public_base_url: s.public_base_url.clone(),
            timezone: s.timezone.clone(),
            retention: s.retention,
            updated_at: s.updated_at,
            configured: s.is_configured(),
        }
    }

    pub async fn update(
        &self,
        input: SettingsInput,
        actor: &AuditActor,
    ) -> AppResult<SettingsView> {
        let mut s = self.deps.settings.get().await?;
        let old_configured = s.is_configured();

        s.lark_app_id = normalize(input.lark_app_id);

        match input.app_secret.as_deref() {
            None => {}
            Some("") => s.lark_app_secret_enc = None,
            Some(secret) => {
                if secret.trim().len() < 8 {
                    return Err(AppError::validation("app_secret 看起来不合法（过短）"));
                }
                s.lark_app_secret_enc = Some(self.deps.cipher.encrypt(secret.trim())?);
            }
        }

        match input.verification_token.as_deref() {
            None => {}
            Some("") => s.lark_verification_token_enc = None,
            Some(token) => {
                let token = token.trim();
                if token.len() < 8 {
                    return Err(AppError::validation(
                        "Verification Token 看起来不合法（过短）",
                    ));
                }
                s.lark_verification_token_enc = Some(self.deps.cipher.encrypt(token)?);
            }
        }

        s.lark_base_url = input.lark_base_url.trim().to_string();
        s.public_base_url = normalize(input.public_base_url);
        if let Some(name) = input.timezone.as_deref() {
            // 存规范化后的 IANA 名：非法值在这里就拒绝，不让它落库。
            let tz = DisplayTimeZone::parse(name).ok_or_else(|| {
                AppError::validation(format!(
                    "timezone 非法：需 IANA 时区名（如 UTC / Asia/Shanghai），收到 `{}`",
                    name.trim()
                ))
            })?;
            s.timezone = tz.name().to_string();
        }
        s.retention = input.retention;
        s.updated_at = self.deps.now();
        s.validate()?;

        self.deps.settings.save(&s).await?;

        let log = AuditLog::ok(
            actor.clone(),
            audit_action::SETTINGS_UPDATED,
            self.deps.now(),
        )
        .target("settings")
        .detail(json!({
            "app_id": s.lark_app_id,
            "app_secret_changed": input.app_secret.is_some(),
            "verification_token_changed": input.verification_token.is_some(),
            "lark_base_url": s.lark_base_url,
            "public_base_url": s.public_base_url,
            "timezone": s.timezone,
            "configured_before": old_configured,
            "configured_after": s.is_configured(),
        }));
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, "审计写入失败");
        }

        Ok(Self::to_view(&s))
    }

    /// 校验飞书凭据：换取 tenant_access_token 并拉一次群列表。
    pub async fn check(&self) -> AppResult<CredentialCheck> {
        let settings = self.deps.settings.get().await?;
        if !settings.is_configured() {
            return Ok(CredentialCheck {
                ok: false,
                chat_count: 0,
                error_code: None,
                message: Some("尚未配置 app_id / app_secret".into()),
            });
        }
        match self.deps.lark.verify_credentials().await {
            Ok(chats) => Ok(CredentialCheck {
                ok: true,
                chat_count: chats.len(),
                error_code: None,
                message: None,
            }),
            Err(e) => {
                let (code, message) = match e {
                    alkh_domain::PortError::Lark(l) => (Some(l.code), l.message),
                    other => (None, other.to_string()),
                };
                Ok(CredentialCheck {
                    ok: false,
                    chat_count: 0,
                    error_code: code,
                    message: Some(message),
                })
            }
        }
    }

    /// 供 `LarkCredentials` 端口使用：返回解密后的凭据。
    pub async fn current_credential(&self) -> AppResult<Option<LarkCredential>> {
        let s = self.deps.settings.get().await?;
        let (Some(app_id), Some(enc)) = (s.lark_app_id.clone(), s.lark_app_secret_enc.clone())
        else {
            return Ok(None);
        };
        if app_id.trim().is_empty() {
            return Ok(None);
        }
        let secret = self.deps.cipher.decrypt(&enc)?;
        Ok(Some(LarkCredential {
            app_id,
            app_secret: secret,
            base_url: s.lark_base_url,
        }))
    }

    /// 供卡片回调端点使用：返回解密后的 Verification Token（未配置返回 None）。
    pub async fn callback_verification_token(&self) -> AppResult<Option<String>> {
        let s = self.deps.settings.get().await?;
        match s.lark_verification_token_enc {
            Some(enc) => Ok(Some(self.deps.cipher.decrypt(&enc)?)),
            None => Ok(None),
        }
    }
}

fn normalize(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_treats_blank_as_none() {
        assert_eq!(normalize(Some("  ".into())), None);
        assert_eq!(normalize(Some(" x ".into())), Some("x".into()));
        assert_eq!(normalize(None), None);
    }
}
