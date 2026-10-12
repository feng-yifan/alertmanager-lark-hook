use std::sync::Arc;

use alkh_domain::ports::*;
use alkh_domain::Timestamp;

/// 应用层依赖集合（端口注入）。所有用例都通过它访问外部世界。
#[derive(Clone)]
pub struct Deps {
    pub channels: Arc<dyn ChannelRepository>,
    pub hooks: Arc<dyn HookRepository>,
    pub templates: Arc<dyn CardTemplateRepository>,
    pub policies: Arc<dyn NotificationPolicyRepository>,
    pub groups: Arc<dyn AlertGroupRepository>,
    pub deliveries: Arc<dyn DeliveryRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub users: Arc<dyn AdminUserRepository>,
    pub sessions: Arc<dyn SessionRepository>,
    pub audit: Arc<dyn AuditLogRepository>,
    pub dedup: Arc<dyn EventDedup>,
    pub lark: Arc<dyn LarkGateway>,
    pub cipher: Arc<dyn SecretCipher>,
    pub hasher: Arc<dyn PasswordHasher>,
    pub clock: Arc<dyn Clock>,
}

impl Deps {
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }
}

impl std::fmt::Debug for Deps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deps").finish_non_exhaustive()
    }
}
