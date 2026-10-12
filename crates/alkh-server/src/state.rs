//! 应用状态：由 `main` 组装的组合根。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use alkh_app::channels::ChannelService;
use alkh_app::intake::IntakeService;
use alkh_app::policies::PolicyService;
use alkh_app::query::QueryService;
use alkh_app::settings::SettingsService;
use alkh_app::templates::TemplateService;
use alkh_app::{auth::AuthService, Deps};

use crate::config::Config;
use crate::metrics::Metrics;

#[derive(Clone)]
pub struct AppState {
    pub deps: Deps,
    pub config: Arc<Config>,
    pub metrics: Arc<Metrics>,
    pub intake: Arc<IntakeService>,
    pub channels: Arc<ChannelService>,
    pub policies: Arc<PolicyService>,
    pub templates: Arc<TemplateService>,
    pub settings: Arc<SettingsService>,
    pub auth: Arc<AuthService>,
    pub query: Arc<QueryService>,
    /// 连接池句柄：组合根与就绪探针使用（业务代码只经端口访问数据库）。
    pub pool: sqlx::SqlitePool,
    /// `/readyz` 用的就绪标志。
    pub ready: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(
        deps: Deps,
        config: Arc<Config>,
        metrics: Arc<Metrics>,
        pool: sqlx::SqlitePool,
    ) -> Self {
        Self {
            intake: Arc::new(IntakeService::new(deps.clone())),
            channels: Arc::new(ChannelService::new(deps.clone())),
            policies: Arc::new(PolicyService::new(deps.clone())),
            templates: Arc::new(TemplateService::new(deps.clone())),
            settings: Arc::new(SettingsService::new(deps.clone())),
            auth: Arc::new(AuthService::new(deps.clone())),
            query: Arc::new(QueryService::new(deps.clone())),
            deps,
            config,
            metrics,
            pool,
            ready: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn sender(&self) -> &alkh_app::sender::NotificationSender {
        self.intake.sender()
    }
}
