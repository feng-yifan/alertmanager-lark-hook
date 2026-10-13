//! AlertManager Hook 接收流水线（`docs/business-rules.md` §1）。

use std::time::Duration;

use alkh_domain::alert::{
    classify, decide, event_hash, AlertGroup, AlertManagerPayload, DeliveryAction, DeliveryState,
    EventKind, NormalizedEvent,
};
use alkh_domain::audit::{action as audit_action, skip_reason, AuditActor, AuditLog, Outcome};
use alkh_domain::channel::Channel;
use alkh_domain::hook::{hash_token, Hook};
use alkh_domain::ids::{AlertGroupId, DeliveryId};
use alkh_domain::policy::NotificationPolicy;
use alkh_domain::Timestamp;
use serde::Serialize;
use serde_json::json;
use tokio::time::Instant;

use crate::aggregator::{
    Aggregator, BucketReason, PendingBucket, RateLimiter, DEFAULT_DEDUP_WINDOW_SECS,
};
use crate::deps::Deps;
use crate::error::AppResult;
use crate::sender::NotificationSender;

/// 单个聚合桶的最大重调度次数，避免异常的自我延续。
const MAX_BUCKET_ATTEMPTS: u32 = 10;

#[derive(Debug, Clone, Serialize, Default)]
pub struct IntakeOutcome {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<EventKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<DeliveryAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_group_id: Option<AlertGroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<DeliveryId>,
    /// 本次投递的实际落库状态（`active` / `pending` / `failed` ...）。
    /// 指标与告警排查都以它为准，而不是"决策动作"。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_state: Option<DeliveryState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deferred: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl IntakeOutcome {
    fn skipped(reason: &str) -> Self {
        Self {
            ok: true,
            skipped: Some(reason.to_string()),
            ..Default::default()
        }
    }

    fn deferred(event: EventKind, reason: &str) -> Self {
        Self {
            ok: true,
            event: Some(event),
            deferred: true,
            skipped: Some(reason.to_string()),
            ..Default::default()
        }
    }
}

pub struct IntakeService {
    deps: Deps,
    sender: NotificationSender,
    aggregator: Aggregator,
    limiter: RateLimiter,
}

impl IntakeService {
    pub fn new(deps: Deps) -> Self {
        let sender = NotificationSender::new(deps.clone());
        Self {
            deps,
            sender,
            aggregator: Aggregator::default(),
            limiter: RateLimiter::default(),
        }
    }

    pub fn deps(&self) -> &Deps {
        &self.deps
    }

    pub fn sender(&self) -> &NotificationSender {
        &self.sender
    }

    pub fn aggregator(&self) -> &Aggregator {
        &self.aggregator
    }

    /// 接收一次 AlertManager 推送。**只要状态已持久化就返回 `Ok`**，发送失败交给后台重试。
    pub async fn handle_hook(&self, token: &str, body: &[u8]) -> AppResult<IntakeOutcome> {
        let now = self.deps.now();

        // 1. 校验 token
        let hook = match self
            .deps
            .hooks
            .find_by_token_hash(&hash_token(token))
            .await?
        {
            Some(h) => h,
            None => return Err(crate::error::AppError::not_found("unknown_hook")),
        };
        if !hook.enabled {
            self.audit(skip_reason::HOOK_DISABLED, &hook.id, None, now)
                .await;
            return Ok(IntakeOutcome::skipped(skip_reason::HOOK_DISABLED));
        }

        let channel = self
            .deps
            .channels
            .get(&hook.channel_id)
            .await?
            .ok_or_else(|| crate::error::AppError::not_found("渠道不存在"))?;
        if !channel.enabled {
            self.audit(
                skip_reason::CHANNEL_DISABLED,
                &hook.id,
                Some(&channel.id),
                now,
            )
            .await;
            return Ok(IntakeOutcome::skipped(skip_reason::CHANNEL_DISABLED));
        }

        // 2. 解析 payload
        let payload: AlertManagerPayload = serde_json::from_slice(body).map_err(|e| {
            crate::error::AppError::validation(format!("AlertManager payload 解析失败: {e}"))
        })?;
        if payload.alerts.is_empty() {
            self.audit(skip_reason::NO_ALERTS, &hook.id, Some(&channel.id), now)
                .await;
            return Ok(IntakeOutcome::skipped(skip_reason::NO_ALERTS));
        }
        if let Some(v) = payload.version.as_deref() {
            if v != alkh_domain::alert::ALERTMANAGER_PROTOCOL_VERSION {
                tracing::warn!(
                    version = v,
                    "收到非预期版本的 AlertManager payload，按兼容模式处理"
                );
            }
        }

        let mut event = NormalizedEvent::from_payload(&payload)?;
        let policy = self
            .deps
            .policies
            .get(&channel.id)
            .await?
            .unwrap_or_else(|| NotificationPolicy::default_for(channel.id.clone()));

        // 3. 标签过滤（R11）
        if policy.has_filters() {
            let matched = event.matching_alerts(&policy);
            if matched.is_empty() {
                self.audit(skip_reason::FILTERED, &hook.id, Some(&channel.id), now)
                    .await;
                return Ok(IntakeOutcome::skipped(skip_reason::FILTERED));
            }
            event.alerts = matched;
            // 过滤后重新推导分组状态
            event.status = if event.alerts.iter().any(|a| a.is_firing()) {
                alkh_domain::alert::GroupStatus::Firing
            } else {
                alkh_domain::alert::GroupStatus::Resolved
            };
        }

        // 4. 幂等去重（R9）：原子认领。
        //
        // 认领必须发生在处理**之前**（否则两个并发的重复投递会同时通过），
        // 但处理失败时必须**归还**认领 —— 否则 AlertManager 的退避重试会落进
        // 同一个去重窗口、被判为 `duplicate` 并收到 **200**，于是 AM 认为投递成功，
        // 这条告警就永久丢失了（R9 的语义是「跳过已处理的重复」，不是「跳过失败的」）。
        let hash = event_hash(&hook.id, &event);
        if !self
            .deps
            .dedup
            .claim(&hook.id, &hash, now, DEFAULT_DEDUP_WINDOW_SECS)
            .await?
        {
            self.audit(skip_reason::DUPLICATE, &hook.id, Some(&channel.id), now)
                .await;
            return Ok(IntakeOutcome::skipped(skip_reason::DUPLICATE));
        }
        let _ = self.deps.hooks.record_call(&hook.id, now).await;

        // 5. 聚合窗口（R6）：事件已受理并入桶，保留下面的认领。
        if policy.aggregation_window_secs > 0 {
            let key = classify_pending(&self.deps, &hook, &event).await;
            self.schedule(
                &hook,
                &channel,
                event,
                Instant::now() + Duration::from_secs(policy.aggregation_window_secs as u64),
                BucketReason::Aggregation,
            );
            return Ok(IntakeOutcome::deferred(key, "aggregating"));
        }

        match self
            .process_event(&hook, &channel, &policy, event, hash.clone(), now)
            .await
        {
            Ok(outcome) => Ok(outcome),
            Err(err) => {
                // 处理失败（DB 写失败 / 渲染失败等）：归还认领，让 AM 的重试能被
                // 正常处理，而不是被当成重复丢弃。
                if let Err(release_err) = self.deps.dedup.release(&hook.id, &hash).await {
                    tracing::warn!(
                        hook_id = %hook.id,
                        error = %release_err,
                        "归还幂等认领失败：AlertManager 的重试可能被误判为重复"
                    );
                }
                Err(err)
            }
        }
    }

    /// 冲刷到期的聚合桶；返回处理成功的事件数。由 `alkh-server` 定时调用。
    pub async fn flush_due(&self) -> usize {
        let due = self.aggregator.take_due(Instant::now());
        let mut handled = 0usize;
        for bucket in due {
            if self.flush_one(bucket).await {
                handled += 1;
            }
        }
        handled
    }

    async fn flush_one(&self, bucket: PendingBucket) -> bool {
        if bucket.attempts > MAX_BUCKET_ATTEMPTS {
            tracing::warn!(
                hook_id = %bucket.hook_id,
                group_key = %bucket.group_key,
                attempts = bucket.attempts,
                "聚合桶重调度次数超限，丢弃"
            );
            return false;
        }

        let hook = match self.deps.hooks.get(&bucket.hook_id).await {
            Ok(Some(h)) => h,
            Ok(None) => return false,
            Err(e) => {
                tracing::error!(error = %e, "冲刷聚合桶时读取 hook 失败");
                return false;
            }
        };
        let channel = match self.deps.channels.get(&bucket.channel_id).await {
            Ok(Some(c)) => c,
            Ok(None) => return false,
            Err(e) => {
                tracing::error!(error = %e, "冲刷聚合桶时读取渠道失败");
                return false;
            }
        };
        if !hook.enabled || !channel.enabled {
            return false;
        }
        let policy = match self.deps.policies.get(&channel.id).await {
            Ok(Some(p)) => p,
            _ => NotificationPolicy::default_for(channel.id.clone()),
        };

        let hash = event_hash(&hook.id, &bucket.event);
        let now = self.deps.now();
        match self
            .process_event(&hook, &channel, &policy, bucket.event, hash, now)
            .await
        {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(error = %e, "冲刷聚合桶失败");
                false
            }
        }
    }

    fn schedule(
        &self,
        hook: &Hook,
        channel: &Channel,
        event: NormalizedEvent,
        deadline: Instant,
        reason: BucketReason,
    ) {
        self.aggregator.submit(PendingBucket {
            hook_id: hook.id.clone(),
            channel_id: channel.id.clone(),
            group_key: event.group_key.clone(),
            event,
            deadline,
            reason,
            attempts: 0,
        });
    }

    async fn process_event(
        &self,
        hook: &Hook,
        channel: &Channel,
        policy: &NotificationPolicy,
        event: NormalizedEvent,
        hash: String,
        now: Timestamp,
    ) -> AppResult<IntakeOutcome> {
        // R7 最小发送间隔：未满足则推迟而不是丢弃。
        let min_interval = Duration::from_secs(policy.min_send_interval_secs as u64);
        if let Some(until) = self.limiter.ready_at(&channel.id, min_interval) {
            let kind = classify_pending(&self.deps, hook, &event).await;
            self.schedule(hook, channel, event, until, BucketReason::RateLimited);
            return Ok(IntakeOutcome::deferred(kind, "rate_limited"));
        }

        let existing = self.deps.groups.find(&hook.id, &event.group_key).await?;
        let kind = classify(existing.as_ref(), &event);

        // 上一轮活跃投递（可更新 / 可撤回）
        let previous = match existing.as_ref().and_then(|g| g.active_delivery_id.clone()) {
            Some(id) => self
                .deps
                .deliveries
                .get(&id)
                .await?
                .filter(|d| d.state == DeliveryState::Active),
            None => None,
        };
        let action = decide(kind, policy, previous.is_some());

        // 推进分组状态（静默期同样推进，R5）
        let mut group = match &existing {
            Some(g) => {
                let mut g = g.clone();
                g.apply_event(&event, &hash, now);
                g
            }
            None => {
                let mut g = AlertGroup::new(hook.id.clone(), channel.id.clone(), &event, now);
                g.last_event_hash = Some(hash.clone());
                g
            }
        };

        if policy.is_muted(now) {
            self.deps.groups.upsert(&group).await?;
            self.audit(skip_reason::MUTED, &hook.id, Some(&channel.id), now)
                .await;
            return Ok(IntakeOutcome {
                event: Some(kind),
                alert_group_id: Some(group.id.clone()),
                ..IntakeOutcome::skipped(skip_reason::MUTED)
            });
        }

        if action == DeliveryAction::Suppress {
            self.deps.groups.upsert(&group).await?;
            self.audit(
                skip_reason::POLICY_SUPPRESS,
                &hook.id,
                Some(&channel.id),
                now,
            )
            .await;
            return Ok(IntakeOutcome {
                event: Some(kind),
                alert_group_id: Some(group.id.clone()),
                ..IntakeOutcome::skipped(skip_reason::POLICY_SUPPRESS)
            });
        }

        let prepared = self.sender.prepare(&group, channel, policy).await?;
        // 渲染成功后再落库分组，避免"发送前的中间态"污染统计。
        self.deps.groups.upsert(&group).await?;

        let result = self
            .sender
            .deliver(
                &prepared,
                kind.into(),
                action,
                previous.as_ref(),
                Some(&hash),
            )
            .await?;

        match result.current.state {
            DeliveryState::Active => {
                group.active_delivery_id = Some(result.current.id.clone());
                group.mark_notified(now);
            }
            DeliveryState::Resolved => {
                group.active_delivery_id = None;
                group.mark_notified(now);
            }
            _ => {}
        }
        self.deps.groups.upsert(&group).await?;
        self.limiter.record(&channel.id);

        self.audit_sent(hook, channel, kind, action, &result, now)
            .await;

        Ok(IntakeOutcome {
            ok: true,
            event: Some(kind),
            action: Some(action),
            alert_group_id: Some(group.id.clone()),
            delivery_id: Some(result.current.id.clone()),
            delivery_state: Some(result.current.state),
            skipped: None,
            deferred: false,
        })
    }

    async fn audit(
        &self,
        reason: &str,
        hook_id: &alkh_domain::ids::HookId,
        channel_id: Option<&alkh_domain::ids::ChannelId>,
        now: Timestamp,
    ) {
        let log = AuditLog::skipped(
            AuditActor::alertmanager(hook_id.to_string()),
            audit_action::HOOK_RECEIVED,
            reason,
            now,
        )
        .target(channel_id.map(|c| c.to_string()).unwrap_or_default())
        .detail(json!({ "reason": reason }));
        self.write_audit(log).await;
    }

    async fn audit_sent(
        &self,
        hook: &Hook,
        channel: &Channel,
        kind: EventKind,
        action: DeliveryAction,
        result: &crate::sender::DeliveryResult,
        now: Timestamp,
    ) {
        let d = &result.current;
        let actor = AuditActor::alertmanager(hook.id.to_string());
        let outcome = if d.state == DeliveryState::Failed {
            Outcome::failed(d.last_error.clone().unwrap_or_else(|| "投递失败".into()))
        } else {
            Outcome::Ok
        };
        let log = AuditLog::new(
            actor,
            if d.state == DeliveryState::Failed {
                audit_action::DELIVERY_FAILED
            } else if d.state == DeliveryState::Resolved {
                audit_action::MESSAGE_UPDATED
            } else {
                audit_action::MESSAGE_SENT
            },
            outcome,
            now,
        )
        .target(channel.id.to_string())
        .detail(json!({
            "event": kind.as_str(),
            "action": action.as_str(),
            "alert_group_id": result.current.alert_group_id.to_string(),
            "delivery_id": d.id.to_string(),
            // 用 serde 表示（snake_case）与 HTTP 响应的 delivery_state 保持一致；
            // format!("{:?}") 会把 Rust 变体名（Pending/Active）写进审计。
            "delivery_state": serde_json::to_value(d.state)
                .unwrap_or(serde_json::Value::Null),
            "message_type": d.message_type.as_str(),
            "message_id": d.lark_message_id,
            "card_entity_id": d.card_entity_id,
            "app_feed_biz_id": d.app_feed_biz_id,
            "fell_back_to_send_new": result.fell_back_to_send_new,
        }));
        self.write_audit(log).await;
    }

    async fn write_audit(&self, log: AuditLog) {
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, action = %log.action, "审计日志写入失败");
        }
    }
}

/// 分类但不产生副作用（用于 deferred 场景返回事件类型）。
async fn classify_pending(deps: &Deps, hook: &Hook, event: &NormalizedEvent) -> EventKind {
    match deps.groups.find(&hook.id, &event.group_key).await {
        Ok(existing) => alkh_domain::alert::classify(existing.as_ref(), event),
        Err(_) => alkh_domain::alert::classify(None, event),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_serializes_without_none_fields() {
        let o = IntakeOutcome::skipped("filtered");
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["skipped"], "filtered");
        assert!(v.get("event").is_none());
        assert!(v.get("deferred").is_none());
    }

    #[test]
    fn deferred_outcome_marks_flag() {
        let o = IntakeOutcome::deferred(EventKind::NewFiring, "aggregating");
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["deferred"], true);
        assert_eq!(v["event"], "new_firing");
    }
}
