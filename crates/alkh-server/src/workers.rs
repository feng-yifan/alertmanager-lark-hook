//! 后台任务：聚合桶冲刷、失败投递重试、数据保留清理。

use std::time::Duration;

use alkh_domain::alert::DeliveryState;
use alkh_domain::audit::{action as audit_action, AuditActor, AuditLog};
use tokio::time::MissedTickBehavior;

use crate::metrics::Metrics;
use crate::state::AppState;

pub fn spawn(state: AppState) {
    tokio::spawn(aggregator_loop(state.clone()));
    tokio::spawn(retry_loop(state.clone()));
    tokio::spawn(retention_loop(state));
}

/// R6：每 tick 冲刷到期的聚合桶。
async fn aggregator_loop(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_millis(state.config.worker_tick_ms));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let report = state.intake.flush_due().await;
        if report.handled > 0 {
            Metrics::add(
                &state.metrics.aggregator_flushed_total,
                report.handled as u64,
            );
            tracing::debug!(flushed = report.handled, "聚合桶已冲刷");
        }
        // 冲刷失败不再静默丢弃：放回队列退避重试，超限才放弃。这两个计数让
        // 「聚合层正在丢事件」这件事变得可见（此前完全没有信号）。
        if report.requeued > 0 {
            Metrics::add(
                &state.metrics.aggregator_requeued_total,
                report.requeued as u64,
            );
        }
        if report.dropped > 0 {
            Metrics::add(
                &state.metrics.aggregator_dropped_total,
                report.dropped as u64,
            );
            tracing::error!(dropped = report.dropped, "有聚合事件被放弃（已无法重试）");
        }
    }
}

/// R14：按 1s / 5s / 30s / 2m / 10m 的退避重试失败的投递。
async fn retry_loop(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(state.config.retry_interval_secs));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval.tick().await; // 跳过立即触发

    loop {
        interval.tick().await;
        let candidates = match state.deps.deliveries.list_retryable(50).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "读取待重试投递失败");
                continue;
            }
        };

        for delivery in candidates {
            if !backoff_elapsed(&delivery, state.deps.now()) {
                continue;
            }
            Metrics::inc(&state.metrics.delivery_retries_total);
            match state.sender().retry(&delivery).await {
                Ok(updated) => match updated.state {
                    DeliveryState::Failed => {
                        Metrics::inc(&state.metrics.deliveries_failed_total);
                        tracing::warn!(
                            delivery_id = %updated.id,
                            attempt = updated.attempt,
                            error = updated.last_error.as_deref().unwrap_or(""),
                            "投递重试耗尽"
                        );
                    }
                    DeliveryState::Active | DeliveryState::Resolved => {
                        Metrics::inc(&state.metrics.deliveries_sent_total);
                        tracing::info!(delivery_id = %updated.id, "投递重试成功");
                    }
                    _ => {}
                },
                Err(e) => tracing::warn!(delivery_id = %delivery.id, error = %e, "投递重试失败"),
            }
        }
    }
}

/// R14 的退避表。
fn backoff_for(attempt: u32) -> Duration {
    match attempt {
        0 => Duration::from_secs(1),
        1 => Duration::from_secs(5),
        2 => Duration::from_secs(30),
        3 => Duration::from_secs(120),
        _ => Duration::from_secs(600),
    }
}

fn backoff_elapsed(delivery: &alkh_domain::alert::Delivery, now: alkh_domain::Timestamp) -> bool {
    let waited = now.signed_duration_since(delivery.updated_at);
    waited >= chrono::Duration::from_std(backoff_for(delivery.attempt)).unwrap_or_default()
}

/// 数据保留清理。
async fn retention_loop(state: AppState) {
    let mut interval =
        tokio::time::interval(Duration::from_secs(state.config.retention_interval_secs));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval.tick().await;

    loop {
        interval.tick().await;
        let settings = match state.deps.settings.get().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "读取保留策略失败");
                continue;
            }
        };
        let now = state.deps.now();
        let r = settings.retention;

        let groups = state
            .deps
            .groups
            .delete_before(now - chrono::Duration::days(r.alert_group_days as i64))
            .await;
        let deliveries = state
            .deps
            .deliveries
            .delete_before(now - chrono::Duration::days(r.delivery_days as i64))
            .await;
        let audits = state
            .deps
            .audit
            .delete_before(now - chrono::Duration::days(r.audit_days as i64))
            .await;
        let dedup = state
            .deps
            .dedup
            .purge_before(now - chrono::Duration::hours(1))
            .await;
        let sessions = state.deps.sessions.purge_expired(now).await;

        // 清理失败以前被静默计成 0（`ok().unwrap_or(0)`），于是「某张表无限增长」
        // 这件事完全没有任何声音。现在逐个记录失败。
        fn count(name: &str, r: &Result<u64, alkh_domain::PortError>) -> u64 {
            match r {
                Ok(n) => *n,
                Err(e) => {
                    tracing::error!(table = name, error = %e, "保留清理失败，本次跳过该表");
                    0
                }
            }
        }
        let (g, d, a, dd, ss) = (
            count("alert_groups", &groups),
            count("deliveries", &deliveries),
            count("audit_logs", &audits),
            count("event_dedup", &dedup),
            count("sessions", &sessions),
        );
        let total = g + d + a + dd + ss;

        if total > 0 {
            tracing::info!(deleted = total, "数据保留清理完成");
            let log = AuditLog::ok(AuditActor::system("retention"), "retention.cleaned", now)
                .detail(serde_json::json!({
                    "alert_groups": g,
                    "deliveries": d,
                    "audit_logs": a,
                    "event_dedup": dd,
                    "sessions": ss,
                }));
            let _ = state.deps.audit.append(&log).await;
        }
    }
}

/// 供 handler 复用的动作名（避免拼写漂移）。
pub const RETENTION_ACTION: &str = audit_action::CHANNEL_SYNCED;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_schedule_matches_r14() {
        assert_eq!(backoff_for(0), Duration::from_secs(1));
        assert_eq!(backoff_for(1), Duration::from_secs(5));
        assert_eq!(backoff_for(2), Duration::from_secs(30));
        assert_eq!(backoff_for(3), Duration::from_secs(120));
        assert_eq!(backoff_for(4), Duration::from_secs(600));
        assert_eq!(backoff_for(9), Duration::from_secs(600));
    }

    #[test]
    fn backoff_gate_respects_elapsed_time() {
        use alkh_domain::alert::{Delivery, DeliveryKind};
        use alkh_domain::ids::{AlertGroupId, ChannelId};
        use alkh_domain::policy::MessageType;

        let now = chrono::Utc::now();
        let mut d = Delivery::new(
            AlertGroupId::new(),
            ChannelId::new(),
            DeliveryKind::FiringCreated,
            MessageType::CardMessage,
            None,
            "d",
            now,
        );
        // attempt 0 ⇒ 需要等 1 秒
        assert!(!backoff_elapsed(&d, now));
        assert!(backoff_elapsed(&d, now + chrono::Duration::seconds(2)));

        // attempt 3 ⇒ 需要等 2 分钟
        d.attempt = 3;
        assert!(!backoff_elapsed(&d, now + chrono::Duration::seconds(60)));
        assert!(backoff_elapsed(&d, now + chrono::Duration::seconds(130)));
    }
}
