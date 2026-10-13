//! 聚合窗口（R6）与最小发送间隔（R7）的进程内调度。
//!
//! 单实例部署下无需外部队列：`alkh-server` 每 200ms 调用一次
//! [`IntakeService::flush_due`](crate::intake::IntakeService::flush_due) 冲刷到期事件。

use std::time::Duration;

use alkh_domain::alert::NormalizedEvent;
use alkh_domain::ids::{ChannelId, HookId};
use dashmap::DashMap;
use tokio::time::Instant;

/// 默认去重窗口（R9）。
pub const DEFAULT_DEDUP_WINDOW_SECS: i64 = 60;

/// 一个聚合桶最多被重新排队多少次；超过后丢弃并记 error（此时已重试约 1 分钟）。
pub const MAX_BUCKET_ATTEMPTS: u32 = 10;

/// 一个等待冲刷的聚合桶。同一 `(hook, group_key)` 在窗口内只保留最后一个事件（R6）。
#[derive(Debug, Clone)]
pub struct PendingBucket {
    pub hook_id: HookId,
    pub channel_id: ChannelId,
    pub group_key: String,
    pub event: NormalizedEvent,
    pub deadline: Instant,
    /// 聚合原因，仅用于日志。
    pub reason: BucketReason,
    pub attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketReason {
    /// 聚合窗口内合并。
    Aggregation,
    /// 受最小发送间隔限制而延迟。
    RateLimited,
}

#[derive(Default)]
pub struct Aggregator {
    buckets: DashMap<String, PendingBucket>,
}

impl Aggregator {
    pub fn key(hook_id: &HookId, group_key: &str) -> String {
        format!("{hook_id}\u{1f}{group_key}")
    }

    /// 覆盖同键的待处理事件（窗口内取最后一个，R6）。
    pub fn submit(&self, bucket: PendingBucket) {
        let key = Self::key(&bucket.hook_id, &bucket.group_key);
        self.buckets.insert(key, bucket);
    }

    /// 推迟某个桶的到期时间（用于最小发送间隔）。
    pub fn defer(&self, key: &str, until: Instant) {
        if let Some(mut b) = self.buckets.get_mut(key) {
            b.deadline = until;
            b.reason = BucketReason::RateLimited;
            b.attempts += 1;
        }
    }

    /// 取出所有到期桶，按到期时间升序。
    pub fn take_due(&self, now: Instant) -> Vec<PendingBucket> {
        let due_keys: Vec<String> = self
            .buckets
            .iter()
            .filter(|e| e.value().deadline <= now)
            .map(|e| e.key().clone())
            .collect();

        let mut due: Vec<PendingBucket> = due_keys
            .into_iter()
            .filter_map(|k| self.buckets.remove(&k).map(|(_, v)| v))
            .collect();
        due.sort_by_key(|b| b.deadline);
        due
    }

    /// 冲刷失败后把桶放回队列，保留事件并增加尝试次数。
    ///
    /// 返回 `false` 表示尝试次数已超限、该事件被丢弃。
    ///
    /// 若期间同一 `(hook, group_key)` 已有**更新**的事件入桶，则保留更新者
    /// （R6 last-wins），本次的旧快照直接丢弃 —— 这样不会用旧状态覆盖新状态。
    pub fn requeue_after_failure(&self, bucket: PendingBucket, retry_at: Instant) -> bool {
        let key = Self::key(&bucket.hook_id, &bucket.group_key);
        if self.buckets.contains_key(&key) {
            return true;
        }
        let mut bucket = bucket;
        bucket.attempts += 1;
        bucket.deadline = retry_at;
        if bucket.attempts > MAX_BUCKET_ATTEMPTS {
            return false;
        }
        self.buckets.insert(key, bucket);
        true
    }

    /// 取出全部桶（不论是否到期），用于优雅停机时冲刷。
    pub fn take_all(&self) -> Vec<PendingBucket> {
        let keys: Vec<String> = self.buckets.iter().map(|e| e.key().clone()).collect();
        let mut all: Vec<PendingBucket> = keys
            .into_iter()
            .filter_map(|k| self.buckets.remove(&k).map(|(_, v)| v))
            .collect();
        all.sort_by_key(|b| b.deadline);
        all
    }

    /// 最近一个到期时间，供调度循环决定 sleep 时长。
    pub fn next_deadline(&self) -> Option<Instant> {
        self.buckets.iter().map(|e| e.value().deadline).min()
    }

    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    pub fn clear(&self) {
        self.buckets.clear();
    }
}

/// 每渠道的最小发送间隔跟踪（R7）。
#[derive(Default)]
pub struct RateLimiter {
    last_sent: DashMap<ChannelId, Instant>,
}

impl RateLimiter {
    /// 若当前未满足最小间隔，返回应该延迟到的时刻。
    pub fn ready_at(&self, channel_id: &ChannelId, min_interval: Duration) -> Option<Instant> {
        if min_interval.is_zero() {
            return None;
        }
        let last = self.last_sent.get(channel_id)?;
        let ready = *last + min_interval;
        let now = Instant::now();
        if ready > now {
            Some(ready)
        } else {
            None
        }
    }

    pub fn record(&self, channel_id: &ChannelId) {
        self.last_sent.insert(channel_id.clone(), Instant::now());
    }

    pub fn forget(&self, channel_id: &ChannelId) {
        self.last_sent.remove(channel_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alkh_domain::alert::{AlertGroup, NormalizedEvent};
    use serde_json::json;

    fn event() -> NormalizedEvent {
        let payload: alkh_domain::alert::AlertManagerPayload = serde_json::from_value(json!({
            "version":"4","groupKey":"g","status":"firing",
            "alerts":[{"status":"firing","labels":{"alertname":"X"},"fingerprint":"f1"}]
        }))
        .unwrap();
        NormalizedEvent::from_payload(&payload).unwrap()
    }

    fn bucket(deadline: Instant, reason: BucketReason) -> PendingBucket {
        PendingBucket {
            hook_id: HookId::new(),
            channel_id: ChannelId::new(),
            group_key: "g".into(),
            event: event(),
            deadline,
            reason,
            attempts: 0,
        }
    }

    /// 指定 hook 与 group_key 的桶，便于构造同键 / 跨键场景。
    fn bucket_for(hook_id: &HookId, group_key: &str, deadline: Instant) -> PendingBucket {
        PendingBucket {
            hook_id: hook_id.clone(),
            group_key: group_key.to_string(),
            ..bucket(deadline, BucketReason::Aggregation)
        }
    }

    /// 冲刷失败必须**保留事件**并增加尝试次数；超过上限才允许丢弃。
    /// 这条路径以前是直接丢掉，等于静默吃掉一条 AM 认为已经投递成功的告警。
    #[test]
    fn requeue_after_failure_keeps_event_until_attempt_cap() {
        let agg = Aggregator::default();
        let hook = HookId::new();
        let now = Instant::now();
        agg.submit(bucket_for(&hook, "g", now));

        let taken = agg.take_due(now + Duration::from_secs(1));
        assert_eq!(taken.len(), 1);
        assert!(agg.is_empty());

        let retry_at = now + Duration::from_secs(2);
        assert!(agg.requeue_after_failure(taken[0].clone(), retry_at));
        assert_eq!(agg.len(), 1, "事件必须还在队列里");
        assert_eq!(agg.next_deadline(), Some(retry_at), "应按新的重试时间排队");

        // 未到期不会被取出
        assert!(agg.take_due(now).is_empty());
        let again = agg.take_due(retry_at);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].attempts, 1, "尝试次数应递增");

        // 到达上限后拒绝再排队
        let mut capped = again[0].clone();
        capped.attempts = MAX_BUCKET_ATTEMPTS;
        assert!(!agg.requeue_after_failure(capped, retry_at));
        assert!(agg.is_empty());
    }

    /// 失败重排期间若有更新的事件入桶，必须保留更新者（R6 last-wins），
    /// 不能用旧快照把新状态覆盖回去。
    #[test]
    fn requeue_does_not_clobber_a_newer_event() {
        let agg = Aggregator::default();
        let hook = HookId::new();
        let now = Instant::now();
        agg.submit(bucket_for(&hook, "g", now));
        let taken = agg.take_due(now + Duration::from_secs(1));

        // 期间来了一个更新的事件
        let mut newer = bucket_for(&hook, "g", now + Duration::from_secs(5));
        newer.event.common_labels.insert("v".into(), "2".into());
        agg.submit(newer.clone());

        assert!(agg.requeue_after_failure(taken[0].clone(), now));
        assert_eq!(agg.len(), 1);
        // 保留的是更新者
        let kept = agg.take_all();
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0].event.common_labels.get("v").map(String::as_str),
            Some("2")
        );
    }

    /// 停机冲刷必须取出**全部**桶（含未到期），否则关停时照样丢事件。
    #[test]
    fn take_all_returns_undue_buckets_too() {
        let agg = Aggregator::default();
        let hook = HookId::new();
        let now = Instant::now();
        agg.submit(bucket_for(&hook, "g1", now + Duration::from_secs(600)));
        agg.submit(bucket_for(&hook, "g2", now + Duration::from_secs(900)));
        assert!(agg.take_due(now).is_empty(), "都还没到期");
        assert_eq!(agg.take_all().len(), 2, "停机时应全部取出");
        assert!(agg.is_empty());
    }

    #[test]
    fn same_key_is_coalesced_last_wins() {
        let agg = Aggregator::default();
        let hook = HookId::new();
        let mut a = bucket(Instant::now(), BucketReason::Aggregation);
        a.hook_id = hook.clone();
        a.event.group_key = "k1".into();
        agg.submit(a);

        let mut b = bucket(Instant::now(), BucketReason::Aggregation);
        b.hook_id = hook.clone();
        b.event.group_key = "k1".into();
        b.event.receiver = "second".into();
        agg.submit(b);

        assert_eq!(agg.len(), 1);
        let item = agg.buckets.iter().next().unwrap();
        assert_eq!(item.value().event.receiver, "second");
    }

    #[test]
    fn take_due_respects_deadline() {
        let agg = Aggregator::default();
        let now = Instant::now();
        agg.submit(bucket(
            now - Duration::from_secs(1),
            BucketReason::Aggregation,
        ));
        let mut not_due = bucket(now + Duration::from_secs(60), BucketReason::Aggregation);
        not_due.group_key = "other".into();
        agg.submit(not_due);

        let due = agg.take_due(now);
        assert_eq!(due.len(), 1);
        assert_eq!(agg.len(), 1);
        assert!(agg.next_deadline().is_some());
    }

    #[test]
    fn defer_moves_deadline_and_bumps_attempts() {
        let agg = Aggregator::default();
        let now = Instant::now();
        let b = bucket(now, BucketReason::Aggregation);
        let hook = b.hook_id.clone();
        let key = Aggregator::key(&hook, &b.group_key);
        agg.submit(b);
        let later = now + Duration::from_secs(30);
        agg.defer(&key, later);
        let got = agg.take_due(now);
        assert!(got.is_empty());
        let got = agg.take_due(later);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].attempts, 1);
        assert_eq!(got[0].reason, BucketReason::RateLimited);
    }

    #[test]
    fn rate_limiter_gates_then_releases() {
        let rl = RateLimiter::default();
        let ch = ChannelId::new();
        // 未发送过 ⇒ 立即可发
        assert!(rl.ready_at(&ch, Duration::from_secs(10)).is_none());
        rl.record(&ch);
        assert!(rl.ready_at(&ch, Duration::from_secs(60)).is_some());
        // 间隔为 0 ⇒ 不受限
        assert!(rl.ready_at(&ch, Duration::ZERO).is_none());
    }

    #[test]
    fn alert_group_clone_is_used_for_bucket() {
        // 保证桶内可携带完整分组（编译期契约）
        let payload: alkh_domain::alert::AlertManagerPayload = serde_json::from_value(json!({
            "version":"4","groupKey":"g","status":"firing",
            "alerts":[{"status":"firing","labels":{},"fingerprint":"f"}]
        }))
        .unwrap();
        let ev = NormalizedEvent::from_payload(&payload).unwrap();
        let g = AlertGroup::new(HookId::new(), ChannelId::new(), &ev, chrono::Utc::now());
        assert!(g.invariants_hold());
    }
}
