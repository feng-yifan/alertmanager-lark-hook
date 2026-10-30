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
    MAX_BUCKET_ATTEMPTS,
};
use crate::deps::Deps;
use crate::error::AppResult;
use crate::sender::NotificationSender;

/// 一次聚合冲刷的结果统计，供 `alkh-server` 更新指标与审计。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlushReport {
    /// 已成功处理。
    pub handled: usize,
    /// 处理失败但可恢复：已放回队列、等待退避重试。
    pub requeued: usize,
    /// 永久失败或重试超限而放弃。
    pub dropped: usize,
}

enum FlushOutcome {
    Done,
    /// 可恢复的失败：把事件放回队列重试。
    ///
    /// 用 `Box` 是因为 `PendingBucket` 带整份事件快照（>280 字节），
    /// 直接内联会让枚举尺寸膨胀（失败路径才会分配，可接受）。
    Requeue(Box<PendingBucket>),
    /// 永久失败（hook 被删、尝试次数超限等）。
    Drop,
}

/// 聚合桶失败重试的基础退避（按尝试次数线性增长并封顶）。
const BUCKET_RETRY_BASE_SECS: u64 = 2;
const BUCKET_RETRY_MAX_SECS: u64 = 60;

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

/// 「忽略并 unpin」/「恢复 Pin」的结果。
#[derive(Debug, Clone, Serialize)]
pub struct IgnoreOutcome {
    /// 分组是否存在（不存在时为 false，回调据此提示）。
    pub found: bool,
    /// 本次撤销的 Pin 条数。
    pub unpinned: usize,
}

/// 按 key 串行化的异步锁。
///
/// 用途：`(hook, group_key)` 上的「读取分组 → 推进状态 → 落库 → 发消息」必须整体
/// 串行，否则两个并发事件各自读到同一份快照，后写者覆盖前者，前者的状态推进
/// （`resolved_count` / `last_seen_at` / `event_count`）静默丢失，甚至两边都判定
/// 「需要发新消息」而出现重复卡片。
///
/// **只在进程内有效**：本项目明确单副本部署（嵌入式 SQLite 单写者 + 内存态聚合器 /
/// 限流器 / 卡片序号），与 README 及 `deploy/kubernetes.yaml` 的 `replicas: 1` 约束一致。
#[derive(Default)]
pub struct KeyedLocks {
    inner:
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
}

impl KeyedLocks {
    /// 取得某个 key 的独占锁；返回值 drop 时自动释放。
    pub async fn lock(&self, key: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let entry = {
            let mut guard = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // 顺手回收无人持有的条目（map 自己算一个引用），避免 key 空间无限增长。
            guard.retain(|_, v| std::sync::Arc::strong_count(v) > 1);
            guard
                .entry(key.to_string())
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        entry.lock_owned().await
    }

    /// 当前仍在跟踪的 key 数量（测试用）。
    pub fn tracked(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

pub struct IntakeService {
    deps: Deps,
    sender: NotificationSender,
    aggregator: Aggregator,
    limiter: RateLimiter,
    /// 同一 `(hook, group_key)` 的读改写串行化，见 [`KeyedLocks`]。
    group_locks: KeyedLocks,
}

impl IntakeService {
    pub fn new(deps: Deps) -> Self {
        let sender = NotificationSender::new(deps.clone());
        Self {
            deps,
            sender,
            aggregator: Aggregator::default(),
            group_locks: KeyedLocks::default(),
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

    /// 冲刷所有到期桶，返回本次处理结果统计。
    pub async fn flush_due(&self) -> FlushReport {
        let due = self.aggregator.take_due(Instant::now());
        self.flush_buckets(due).await
    }

    /// 优雅停机时冲刷**全部**待处理桶（不论是否到期）。
    ///
    /// 聚合桶只存在于进程内存中，直接退出会把已受理（已回 200）的事件整批丢掉。
    /// 关停前冲一次，至少覆盖「发布 / 滚动重启」这一最常见场景。
    pub async fn flush_all_pending(&self) -> FlushReport {
        let all = self.aggregator.take_all();
        if !all.is_empty() {
            tracing::info!(pending = all.len(), "停机前冲刷待处理的聚合桶");
        }
        self.flush_buckets(all).await
    }

    async fn flush_buckets(&self, buckets: Vec<PendingBucket>) -> FlushReport {
        let mut report = FlushReport::default();
        for bucket in buckets {
            match self.flush_one(bucket).await {
                FlushOutcome::Done => report.handled += 1,
                FlushOutcome::Requeue(bucket) => {
                    let bucket = *bucket;
                    // 退避后重试。这里以前是直接丢弃，于是「当时飞书不可用 / 模板临时
                    // 损坏 / 渠道被临时停用」会静默吃掉整条事件 —— 而 AM 早已收到 200，
                    // 永远不会重发，告警就此消失。
                    let backoff = (BUCKET_RETRY_BASE_SECS * bucket.attempts.max(1) as u64)
                        .min(BUCKET_RETRY_MAX_SECS);
                    let retry_at = Instant::now() + Duration::from_secs(backoff);
                    if self
                        .aggregator
                        .requeue_after_failure(bucket.clone(), retry_at)
                    {
                        report.requeued += 1;
                        tracing::debug!(
                            hook_id = %bucket.hook_id,
                            group_key = %bucket.group_key,
                            attempts = bucket.attempts,
                            backoff_secs = backoff,
                            "聚合桶冲刷失败，已放回队列退避重试"
                        );
                    } else {
                        report.dropped += 1;
                        tracing::error!(
                            hook_id = %bucket.hook_id,
                            group_key = %bucket.group_key,
                            attempts = bucket.attempts,
                            "聚合桶重试次数超限，放弃该事件（AM 已收到 200，不会重发）"
                        );
                    }
                }
                FlushOutcome::Drop => report.dropped += 1,
            }
        }
        report
    }

    async fn flush_one(&self, bucket: PendingBucket) -> FlushOutcome {
        if bucket.attempts > MAX_BUCKET_ATTEMPTS {
            tracing::warn!(
                hook_id = %bucket.hook_id,
                group_key = %bucket.group_key,
                attempts = bucket.attempts,
                "聚合桶重调度次数超限，丢弃"
            );
            return FlushOutcome::Drop;
        }

        // hook 已被删除 ⇒ 永久失败，重试没有意义。
        let hook = match self.deps.hooks.get(&bucket.hook_id).await {
            Ok(Some(h)) => h,
            Ok(None) => return FlushOutcome::Drop,
            Err(e) => {
                tracing::error!(error = %e, "冲刷聚合桶时读取 hook 失败");
                return FlushOutcome::Requeue(Box::new(bucket));
            }
        };
        let channel = match self.deps.channels.get(&bucket.channel_id).await {
            Ok(Some(c)) => c,
            Ok(None) => return FlushOutcome::Drop,
            Err(e) => {
                tracing::error!(error = %e, "冲刷聚合桶时读取渠道失败");
                return FlushOutcome::Requeue(Box::new(bucket));
            }
        };
        // 被停用是**可恢复**的：管理员重新启用后应当补发，所以放回队列而不是丢弃。
        if !hook.enabled || !channel.enabled {
            return FlushOutcome::Requeue(Box::new(bucket));
        }
        let policy = match self.deps.policies.get(&channel.id).await {
            Ok(Some(p)) => p,
            _ => NotificationPolicy::default_for(channel.id.clone()),
        };

        let hash = event_hash(&hook.id, &bucket.event);
        let now = self.deps.now();
        // 需要保留事件本身以便失败后重试；聚合桶冲刷不是热路径（每个窗口至多一次），
        // 一次克隆的成本远小于后面那次飞书调用。
        let event = bucket.event.clone();
        match self
            .process_event(&hook, &channel, &policy, event, hash, now)
            .await
        {
            Ok(_) => FlushOutcome::Done,
            Err(e) => {
                tracing::warn!(error = %e, "冲刷聚合桶失败，将退避重试");
                FlushOutcome::Requeue(Box::new(bucket))
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

    /// 「忽略并 unpin」（R4c）：撤销该分组当前所有 Pin，并置 `pin_ignored`，使本次告警
    /// 事件周期内不再 Pin；重开（Resolved→Firing）时由 `process_event` 自动清除标志。
    ///
    /// 与投递流水线共用同一 `KeyedLocks`（按 `hook_id|group_key`），避免与 `process_event`
    /// 的读改写并发导致标志丢失。飞书侧取消 Pin 失败不影响标志落库，下一次事件会对账重试。
    pub async fn ignore_and_unpin(
        &self,
        group_id: &AlertGroupId,
        operator: Option<String>,
    ) -> AppResult<IgnoreOutcome> {
        let now = self.deps.now();
        // 先读一次拿到锁的 key（hook+group_key）；拿到锁后再重读，保证读到最新快照。
        let Some(probe) = self.deps.groups.get(group_id).await? else {
            return Ok(IgnoreOutcome {
                found: false,
                unpinned: 0,
            });
        };
        let lock_key = format!("{}|{}", probe.hook_id, probe.group_key);
        let _guard = self.group_locks.lock(&lock_key).await;

        let mut group = match self.deps.groups.get(group_id).await? {
            Some(g) => g,
            None => {
                return Ok(IgnoreOutcome {
                    found: false,
                    unpinned: 0,
                })
            }
        };
        group.ignore_pin();
        let policy = self
            .deps
            .policies
            .get(&group.channel_id)
            .await?
            .unwrap_or_else(|| NotificationPolicy::default_for(group.channel_id.clone()));
        let before = group.pinned_messages.len();
        // 复用唯一的撤销路径：flag 置位 ⇒ desired 为空 ⇒ 撤销全部 Pin（不再新增）。
        self.sender.reconcile_pins(&mut group, &policy, None).await;
        let unpinned = before.saturating_sub(group.pinned_messages.len());
        self.deps.groups.upsert(&group).await?;
        self.deps
            .metrics
            .incr(alkh_domain::ports::Counter::PinsIgnored, 1);

        let log = AuditLog::new(
            AuditActor::system("card_callback"),
            audit_action::PIN_IGNORED,
            Outcome::Ok,
            now,
        )
        .target(group.channel_id.to_string())
        .detail(json!({
            "alert_group_id": group.id.to_string(),
            "operator": operator,
            "unpinned": unpinned,
        }));
        self.write_audit(log).await;

        Ok(IgnoreOutcome {
            found: true,
            unpinned,
        })
    }

    /// 管理端「恢复 Pin」（反向出口）：清除 `pin_ignored` 并按策略重新对账，把当前活跃卡片
    /// 重新 Pin 上（若分组仍在 firing 且策略开启）。
    pub async fn unignore_pin(&self, group_id: &AlertGroupId) -> AppResult<IgnoreOutcome> {
        let Some(probe) = self.deps.groups.get(group_id).await? else {
            return Ok(IgnoreOutcome {
                found: false,
                unpinned: 0,
            });
        };
        let lock_key = format!("{}|{}", probe.hook_id, probe.group_key);
        let _guard = self.group_locks.lock(&lock_key).await;
        let mut group = match self.deps.groups.get(group_id).await? {
            Some(g) => g,
            None => {
                return Ok(IgnoreOutcome {
                    found: false,
                    unpinned: 0,
                })
            }
        };
        group.clear_pin_ignored();
        let policy = self
            .deps
            .policies
            .get(&group.channel_id)
            .await?
            .unwrap_or_else(|| NotificationPolicy::default_for(group.channel_id.clone()));
        // 恢复 Pin 时要重新 Pin 当前活跃卡片：取活跃投递的 message_id。
        let current_msg = match group.active_delivery_id.clone() {
            Some(did) => self
                .deps
                .deliveries
                .get(&did)
                .await?
                .and_then(|d| d.lark_message_id),
            None => None,
        };
        self.sender
            .reconcile_pins(&mut group, &policy, current_msg.as_deref())
            .await;
        self.deps.groups.upsert(&group).await?;
        Ok(IgnoreOutcome {
            found: true,
            unpinned: 0,
        })
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
        // 同一 `(hook, group_key)` 必须整体串行：下面的 `find → apply_event → upsert`
        // 是典型的读改写，两个并发事件会互相覆盖；发消息与状态推进也要在同一临界区，
        // 否则会各自判定「需要发新消息」而产生重复卡片。
        // 注意锁的粒度是「一个分组」，不同分组之间完全并行。
        let lock_key = format!("{}|{}", hook.id, event.group_key);
        let _group_guard = self.group_locks.lock(&lock_key).await;

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

        // R4c：「忽略并 unpin」只在本次告警事件周期内有效；分组彻底恢复后重开
        // （事件为 NewFiring 且历史状态为 Resolved）时清除标志，新告警重新按策略 Pin。
        if kind == EventKind::NewFiring
            && existing
                .as_ref()
                .map(|g| g.status == alkh_domain::alert::GroupStatus::Resolved)
                .unwrap_or(false)
        {
            group.clear_pin_ignored();
        }

        if policy.is_muted(now) {
            // R4：静默期不发消息，但 Pin 状态仍要与开关/分组状态对齐
            //（否则关闭开关后旧 Pin 会一直留在群里）。
            self.sender.reconcile_pins(&mut group, policy, None).await;
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
            // 本轮不发送（重复通知被抑制 / 内容变化被抑制）：现有 Pin 保持不变，
            // 但开关被关闭时要取消 Pin（R4）。
            self.sender.reconcile_pins(&mut group, policy, None).await;
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
        // R4：按策略与分组状态对齐 Pin 集合（多条累积；恢复时按开关批量取消或保留）。
        // 若本轮撤回了旧消息（「发送并删除旧告警」），其 Pin 记录应同步移除
        //（消息已不存在，否则会对一条已删除的消息记着 Pin）。
        if let Some(prev) = &result.previous {
            if prev.state == DeliveryState::Recalled {
                if let Some(mid) = prev.lark_message_id.as_deref() {
                    group.pinned_messages.retain(|p| p.message_id != mid);
                }
            }
        }
        let current_message_id = result.current.lark_message_id.clone();
        self.sender
            .reconcile_pins(&mut group, policy, current_message_id.as_deref())
            .await;
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
            "message_id": d.lark_message_id,
            "card_entity_id": d.card_entity_id,
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

    /// 同一个 key 必须互斥；不同 key 不能互相阻塞。
    #[tokio::test]
    async fn keyed_locks_serialize_same_key_only() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let locks = Arc::new(KeyedLocks::default());
        let concurrent = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        // 同一个 key 上并发 8 次，临界区内计数不应被叠加
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let locks = locks.clone();
            let concurrent = concurrent.clone();
            let max_seen = max_seen.clone();
            tasks.push(tokio::spawn(async move {
                let _guard = locks.lock("same").await;
                let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                concurrent.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(
            max_seen.load(Ordering::SeqCst),
            1,
            "同一 key 的临界区发生了重叠"
        );

        // 不同 key 可以并行
        let a = locks.lock("a").await;
        let b = locks.lock("b").await;
        drop((a, b));

        // 回收是**惰性**的：在下一次取锁时顺带清掉无人持有的条目，避免 key 空间
        // 无限增长（每个事件都会产生一个新 key）。
        let held = locks.lock("c").await;
        assert_eq!(
            locks.tracked(),
            1,
            "取锁时应顺带回收空闲 key，只留当前持有的"
        );
        drop(held);
    }

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
