//! 依赖装配：把 `alkh-infra` 的适配器注入到 `alkh-app` 的端口上。
//!
//! 单独成模块是为了让集成测试能够注入 Fake 网关（真实 HTTP 栈 + 内存 SQLite）。

use std::sync::Arc;

use alkh_app::Deps;
use alkh_domain::ports::{
    AdminUserRepository, AlertGroupRepository, AuditLogRepository, CardTemplateRepository,
    ChannelRepository, Clock, DeliveryRepository, EventDedup, HookRepository, LarkGateway,
    NotificationPolicyRepository, PasswordHasher, SecretCipher, SessionRepository,
    SettingsRepository,
};
use alkh_infra::sqlite::{
    alert_groups::SqliteAlertGroupRepository, audit::SqliteAuditLogRepository,
    channels::SqliteChannelRepository, dedup::SqliteEventDedup,
    deliveries::SqliteDeliveryRepository, hooks::SqliteHookRepository,
    policies::SqliteNotificationPolicyRepository, settings::SqliteSettingsRepository,
    templates::SqliteCardTemplateRepository, users::SqliteAdminUserRepository,
    users::SqliteSessionRepository,
};
use alkh_infra::{AesGcmCipher, Argon2Hasher, LarkClient, SystemClock};
use sqlx::SqlitePool;

use crate::config::Config;
use crate::credentials::SettingsCredentialProvider;
use crate::metrics::Metrics;
use crate::state::AppState;

/// 构建全部端口实现。
///
/// `lark_override` 为 `Some` 时使用注入的网关（集成测试用 Fake），否则构造真实的
/// [`LarkClient`]（凭据来自数据库中的设置，运行时可在管理后台更换）。
pub async fn build_deps(
    pool: &SqlitePool,
    cipher: Arc<dyn SecretCipher>,
    hasher: Arc<dyn PasswordHasher>,
    lark_override: Option<Arc<dyn LarkGateway>>,
    metrics: Arc<Metrics>,
) -> anyhow::Result<Deps> {
    let channels: Arc<dyn ChannelRepository> = Arc::new(SqliteChannelRepository::new(pool.clone()));
    let hooks: Arc<dyn HookRepository> = Arc::new(SqliteHookRepository::new(pool.clone()));
    let templates: Arc<dyn CardTemplateRepository> =
        Arc::new(SqliteCardTemplateRepository::new(pool.clone()));
    let policies: Arc<dyn NotificationPolicyRepository> =
        Arc::new(SqliteNotificationPolicyRepository::new(pool.clone()));
    let groups: Arc<dyn AlertGroupRepository> =
        Arc::new(SqliteAlertGroupRepository::new(pool.clone()));
    let deliveries: Arc<dyn DeliveryRepository> =
        Arc::new(SqliteDeliveryRepository::new(pool.clone()));
    let settings: Arc<dyn SettingsRepository> =
        Arc::new(SqliteSettingsRepository::new(pool.clone()));
    let users: Arc<dyn AdminUserRepository> =
        Arc::new(SqliteAdminUserRepository::new(pool.clone()));
    let sessions: Arc<dyn SessionRepository> = Arc::new(SqliteSessionRepository::new(pool.clone()));
    let audit: Arc<dyn AuditLogRepository> = Arc::new(SqliteAuditLogRepository::new(pool.clone()));
    let dedup: Arc<dyn EventDedup> = Arc::new(SqliteEventDedup::new(pool.clone()));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

    let lark: Arc<dyn LarkGateway> = match lark_override {
        Some(gateway) => gateway,
        None => {
            let credentials = Arc::new(SettingsCredentialProvider::new(
                settings.clone(),
                cipher.clone(),
            ));
            Arc::new(LarkClient::new(credentials)?)
        }
    };

    Ok(Deps {
        channels,
        hooks,
        templates,
        policies,
        groups,
        deliveries,
        settings,
        users,
        sessions,
        audit,
        dedup,
        lark,
        cipher,
        hasher,
        clock,
        metrics,
    })
}

/// 由配置与连接池组装完整的 [`AppState`]。
pub async fn build_state(
    config: Arc<Config>,
    pool: SqlitePool,
    lark_override: Option<Arc<dyn LarkGateway>>,
) -> anyhow::Result<AppState> {
    let cipher: Arc<dyn SecretCipher> = match config.master_key.as_deref() {
        Some(key) => Arc::new(AesGcmCipher::parse_master_key(key)?),
        None => Arc::new(AesGcmCipher::load_or_create(&config.master_key_path())?),
    };
    let hasher: Arc<dyn PasswordHasher> = Arc::new(Argon2Hasher);

    let metrics = Metrics::new();
    let deps = build_deps(&pool, cipher, hasher, lark_override, metrics.clone()).await?;
    apply_env_defaults(&deps, &config).await?;

    Ok(AppState::new(deps, config, metrics, pool))
}

/// 把环境变量提供的默认值写入数据库（仅当库中尚无值时；之后以管理后台为准）。
pub async fn apply_env_defaults(deps: &Deps, config: &Config) -> anyhow::Result<()> {
    let mut settings = deps.settings.get().await?;
    let mut changed = false;

    if settings.public_base_url.is_none() {
        if let Some(base) = config.public_base_url.clone() {
            settings.public_base_url = Some(base);
            changed = true;
        }
    } else if let Some(env_base) = config.public_base_url.as_deref() {
        // 库中已有值时**环境变量不再生效**（之后以管理后台为准）。文档此前让人去改
        // ALKH_PUBLIC_BASE_URL，改了没反应还找不到原因 —— 这里必须说出来。
        let stored = settings.public_base_url.as_deref().unwrap_or_default();
        if stored.trim_end_matches('/') != env_base.trim().trim_end_matches('/') {
            tracing::warn!(
                env = env_base,
                stored,
                "ALKH_PUBLIC_BASE_URL 与库中已保存的值不一致：环境变量只在库中为空时生效，                 之后以管理后台「设置」页为准。请到后台修改对外地址。"
            );
        }
    }
    if settings.lark_base_url != config.lark_base_url {
        // lark_base_url 相反：环境变量**总是**覆盖库中值。也要说清楚，否则运维
        // 在后台改了地址、重启后又变回去，会以为是 bug。
        if !settings.lark_base_url.trim().is_empty() {
            tracing::warn!(
                env = %config.lark_base_url,
                stored = %settings.lark_base_url,
                "ALKH_LARK_BASE_URL 会覆盖管理后台里保存的值（环境变量优先级更高）"
            );
        }
        settings.lark_base_url = config.lark_base_url.clone();
        changed = true;
    }
    if changed {
        settings.updated_at = deps.now();
        settings.validate()?;
        deps.settings.save(&settings).await?;
        tracing::info!("已从环境变量写入初始设置");
    }
    Ok(())
}
