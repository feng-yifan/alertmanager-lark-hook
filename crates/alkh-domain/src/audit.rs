use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Timestamp;

/// 审计主体。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditActor {
    /// 管理后台用户。
    Admin { username: String },
    /// AlertManager 经 hook 推送。
    AlertManager { hook_id: String },
    /// 后台任务。
    System { job: String },
}

impl AuditActor {
    pub fn admin(username: impl Into<String>) -> Self {
        Self::Admin {
            username: username.into(),
        }
    }
    pub fn alertmanager(hook_id: impl Into<String>) -> Self {
        Self::AlertManager {
            hook_id: hook_id.into(),
        }
    }
    pub fn system(job: impl Into<String>) -> Self {
        Self::System { job: job.into() }
    }

    pub fn as_str(&self) -> String {
        match self {
            AuditActor::Admin { .. } => "admin".into(),
            AuditActor::AlertManager { .. } => "alertmanager".into(),
            AuditActor::System { .. } => "system".into(),
        }
    }
}

/// 审计结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    /// 被规则跳过（未投递），`reason` 取值见 `SkipReason`。
    Skipped {
        reason: String,
    },
    Failed {
        error: String,
    },
}

impl Outcome {
    pub fn skipped(reason: impl Into<String>) -> Self {
        Self::Skipped {
            reason: reason.into(),
        }
    }
    pub fn failed(error: impl Into<String>) -> Self {
        Self::Failed {
            error: error.into(),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Skipped { .. } => "skipped",
            Outcome::Failed { .. } => "failed",
        }
    }
}

/// 跳过的标准原因（稳定字符串，供前端筛选与指标聚合）。
pub mod skip_reason {
    pub const HOOK_DISABLED: &str = "hook_disabled";
    pub const CHANNEL_DISABLED: &str = "channel_disabled";
    pub const FILTERED: &str = "filtered";
    pub const MUTED: &str = "muted";
    pub const DUPLICATE: &str = "duplicate";
    pub const NO_ALERTS: &str = "no_alerts";
    pub const POLICY_SUPPRESS: &str = "policy_suppress";
    pub const LARK_NOT_CONFIGURED: &str = "lark_not_configured";
}

/// 审计日志（只追加）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: i64,
    pub at: Timestamp,
    pub actor: AuditActor,
    /// 动作名，如 `hook.received` / `message.sent` / `channel.synced`。
    pub action: String,
    pub target: Option<String>,
    pub outcome: Outcome,
    pub detail: Value,
}

impl AuditLog {
    pub fn new(
        actor: AuditActor,
        action: impl Into<String>,
        outcome: Outcome,
        at: Timestamp,
    ) -> Self {
        Self {
            id: 0,
            at,
            actor,
            action: action.into(),
            target: None,
            outcome,
            detail: Value::Null,
        }
    }

    pub fn ok(actor: AuditActor, action: impl Into<String>, at: Timestamp) -> Self {
        Self::new(actor, action, Outcome::Ok, at)
    }

    pub fn skipped(
        actor: AuditActor,
        action: impl Into<String>,
        reason: impl Into<String>,
        at: Timestamp,
    ) -> Self {
        Self::new(actor, action, Outcome::skipped(reason), at)
    }

    pub fn failed(
        actor: AuditActor,
        action: impl Into<String>,
        error: impl Into<String>,
        at: Timestamp,
    ) -> Self {
        Self::new(actor, action, Outcome::failed(error), at)
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn detail(mut self, detail: Value) -> Self {
        self.detail = detail;
        self
    }
}

/// 审计动作名常量。
pub mod action {
    pub const HOOK_RECEIVED: &str = "hook.received";
    pub const MESSAGE_SENT: &str = "message.sent";
    pub const MESSAGE_UPDATED: &str = "message.updated";
    pub const MESSAGE_RECALLED: &str = "message.recalled";
    pub const DELIVERY_FAILED: &str = "delivery.failed";
    pub const CHANNEL_SYNCED: &str = "channel.synced";
    pub const CHANNEL_CREATED: &str = "channel.created";
    pub const CHANNEL_UPDATED: &str = "channel.updated";
    pub const CHANNEL_DELETED: &str = "channel.deleted";
    pub const HOOK_CREATED: &str = "hook.created";
    pub const HOOK_ROTATED: &str = "hook.rotated";
    pub const HOOK_DELETED: &str = "hook.deleted";
    pub const POLICY_UPDATED: &str = "policy.updated";
    pub const TEMPLATE_CREATED: &str = "template.created";
    pub const TEMPLATE_UPDATED: &str = "template.updated";
    pub const TEMPLATE_DELETED: &str = "template.deleted";
    pub const SETTINGS_UPDATED: &str = "settings.updated";
    pub const AUTH_LOGIN: &str = "auth.login";
    pub const AUTH_LOGOUT: &str = "auth.logout";
    pub const AUTH_PASSWORD_CHANGED: &str = "auth.password_changed";
    pub const TEST_MESSAGE: &str = "message.test";
}
