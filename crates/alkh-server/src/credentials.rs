//! 把「设置仓储 + 密钥解密」适配成飞书网关需要的 `LarkCredentials` 端口。
//!
//! 这样 `LarkClient` 与 `SettingsService` 之间不会形成循环依赖：网关只依赖这个窄端口。

use std::sync::Arc;

use alkh_domain::error::PortResult;
use alkh_domain::ports::{LarkCredential, LarkCredentials, SecretCipher, SettingsRepository};
use async_trait::async_trait;

pub struct SettingsCredentialProvider {
    settings: Arc<dyn SettingsRepository>,
    cipher: Arc<dyn SecretCipher>,
}

impl SettingsCredentialProvider {
    pub fn new(settings: Arc<dyn SettingsRepository>, cipher: Arc<dyn SecretCipher>) -> Self {
        Self { settings, cipher }
    }
}

#[async_trait]
impl LarkCredentials for SettingsCredentialProvider {
    async fn current(&self) -> PortResult<Option<LarkCredential>> {
        let settings = self.settings.get().await?;
        let (Some(app_id), Some(enc)) = (
            settings.lark_app_id.clone(),
            settings.lark_app_secret_enc.clone(),
        ) else {
            return Ok(None);
        };
        if app_id.trim().is_empty() {
            return Ok(None);
        }
        let app_secret = self.cipher.decrypt(&enc)?;
        Ok(Some(LarkCredential {
            app_id,
            app_secret,
            base_url: settings.lark_base_url,
        }))
    }
}
