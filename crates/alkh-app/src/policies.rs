//! 通知策略用例。

use alkh_domain::audit::{action as audit_action, AuditActor, AuditLog};
use alkh_domain::ids::{CardTemplateId, ChannelId};
use alkh_domain::policy::{
    GroupUpdatePolicy, LabelMatcher, MessageType, NotificationPolicy, RepeatPolicy, ResolvedPolicy,
};
use alkh_domain::template::TemplateKind;
use alkh_domain::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::deps::Deps;
use crate::error::{AppError, AppResult};

/// PUT 策略的请求体：**不含** `channel_id`（由路径决定）与 `updated_at`（服务端生成）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyInput {
    pub message_type: MessageType,
    pub card_template_id: Option<CardTemplateId>,
    pub pin_unresolved: bool,
    pub repeat_policy: RepeatPolicy,
    pub resolved_policy: ResolvedPolicy,
    pub group_update_policy: GroupUpdatePolicy,
    pub label_matchers: Vec<LabelMatcher>,
    pub aggregation_window_secs: u32,
    pub min_send_interval_secs: u32,
    pub max_alerts_per_message: u32,
    pub mute_until: Option<Timestamp>,
}

impl Default for PolicyInput {
    fn default() -> Self {
        let d = NotificationPolicy::default_for(ChannelId::default());
        Self {
            message_type: d.message_type,
            card_template_id: d.card_template_id,
            pin_unresolved: d.pin_unresolved,
            repeat_policy: d.repeat_policy,
            resolved_policy: d.resolved_policy,
            group_update_policy: d.group_update_policy,
            label_matchers: d.label_matchers,
            aggregation_window_secs: d.aggregation_window_secs,
            min_send_interval_secs: d.min_send_interval_secs,
            max_alerts_per_message: d.max_alerts_per_message,
            mute_until: d.mute_until,
        }
    }
}

impl PolicyInput {
    pub fn into_policy(self, channel_id: ChannelId, updated_at: Timestamp) -> NotificationPolicy {
        NotificationPolicy {
            channel_id,
            message_type: self.message_type,
            card_template_id: self.card_template_id,
            pin_unresolved: self.pin_unresolved,
            repeat_policy: self.repeat_policy,
            resolved_policy: self.resolved_policy,
            group_update_policy: self.group_update_policy,
            label_matchers: self.label_matchers,
            aggregation_window_secs: self.aggregation_window_secs,
            min_send_interval_secs: self.min_send_interval_secs,
            max_alerts_per_message: self.max_alerts_per_message,
            mute_until: self.mute_until,
            updated_at,
        }
    }
}

pub struct PolicyService {
    deps: Deps,
}

impl PolicyService {
    pub fn new(deps: Deps) -> Self {
        Self { deps }
    }

    pub async fn get(&self, channel_id: &ChannelId) -> AppResult<NotificationPolicy> {
        if self.deps.channels.get(channel_id).await?.is_none() {
            return Err(AppError::not_found("渠道不存在"));
        }
        Ok(self
            .deps
            .policies
            .get(channel_id)
            .await?
            .unwrap_or_else(|| NotificationPolicy::default_for(channel_id.clone())))
    }

    /// 全量替换策略（校验见 `docs/business-rules.md` §6）。
    pub async fn upsert(
        &self,
        channel_id: &ChannelId,
        input: PolicyInput,
        actor: &AuditActor,
    ) -> AppResult<NotificationPolicy> {
        let channel = self
            .deps
            .channels
            .get(channel_id)
            .await?
            .ok_or_else(|| AppError::not_found("渠道不存在"))?;

        let mut policy = input.into_policy(channel_id.clone(), self.deps.now());
        policy.channel_id = channel_id.clone();
        policy.updated_at = self.deps.now();
        // R15：应用消息流卡片只能投递给用户级渠道。
        policy.validate(&channel)?;

        // 交叉校验模板类型与消息类型
        if let Some(tid) = policy.card_template_id.clone() {
            let template = self
                .deps
                .templates
                .get(&tid)
                .await?
                .ok_or_else(|| AppError::not_found("卡片模板不存在"))?;
            let want = match policy.message_type {
                MessageType::CardMessage => TemplateKind::CardMessage,
                MessageType::AppFeedCard => TemplateKind::AppFeedCard,
            };
            if template.kind != want {
                return Err(AppError::validation(format!(
                    "模板「{}」类型为 {}，与所选消息类型 {} 不匹配",
                    template.name,
                    template.kind.as_str(),
                    policy.message_type.as_str()
                )));
            }
        }

        self.deps.policies.upsert(&policy).await?;

        let log = AuditLog::ok(actor.clone(), audit_action::POLICY_UPDATED, self.deps.now())
            .target(channel.id.to_string())
            .detail(json!({
                "message_type": policy.message_type.as_str(),
                "card_template_id": policy.card_template_id.as_ref().map(|t| t.to_string()),
                "pin_unresolved": policy.pin_unresolved,
                "repeat_policy": format!("{:?}", policy.repeat_policy),
                "resolved_policy": format!("{:?}", policy.resolved_policy),
                "group_update_policy": format!("{:?}", policy.group_update_policy),
                "matcher_count": policy.label_matchers.len(),
                "aggregation_window_secs": policy.aggregation_window_secs,
                "min_send_interval_secs": policy.min_send_interval_secs,
                "max_alerts_per_message": policy.max_alerts_per_message,
                "mute_until": policy.mute_until,
            }));
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, "审计写入失败");
        }
        Ok(policy)
    }
}
