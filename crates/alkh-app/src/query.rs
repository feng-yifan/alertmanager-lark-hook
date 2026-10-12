//! 查询用例：概览、告警分组、投递、审计，以及预览用的样例数据。

use alkh_domain::alert::{AlertGroup, AlertManagerPayload, Delivery, NormalizedEvent};
use alkh_domain::audit::AuditLog;
use alkh_domain::channel::{Channel, ChannelSource, ReceiveIdType};
use alkh_domain::ids::AlertGroupId;
use alkh_domain::ports::{clamp_limit, AlertGroupFilter, AuditFilter, DeliveryFilter};
use alkh_domain::{Page, Timestamp};
use serde::Serialize;
use serde_json::json;

use crate::deps::Deps;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize)]
pub struct CountPair {
    pub total: u64,
    pub enabled: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertGroupCounts {
    pub firing: u64,
    pub resolved: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DeliveryStats {
    pub sent: u64,
    pub failed: u64,
    pub suppressed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub channels: CountPair,
    pub hooks: CountPair,
    pub alert_groups: AlertGroupCounts,
    pub deliveries_24h: DeliveryStats,
    pub recent_events: Vec<AuditLog>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertGroupDetail {
    #[serde(flatten)]
    pub group: AlertGroup,
    pub channel: Option<ChannelBrief>,
    pub hook: Option<HookBrief>,
    pub deliveries: Vec<Delivery>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelBrief {
    pub id: alkh_domain::ids::ChannelId,
    pub name: String,
    pub receive_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookBrief {
    pub id: alkh_domain::ids::HookId,
    pub name: String,
    pub token_prefix: String,
    pub enabled: bool,
}

pub struct QueryService {
    deps: Deps,
}

impl QueryService {
    pub fn new(deps: Deps) -> Self {
        Self { deps }
    }

    pub async fn overview(&self) -> AppResult<Overview> {
        let now = self.deps.now();
        let (channel_total, channel_enabled) = self.deps.channels.count().await?;
        let (hook_total, hook_enabled) = self.deps.hooks.count().await?;
        let (firing, resolved) = self.deps.groups.count_by_status().await?;
        let since = now - chrono::Duration::hours(24);

        let mut stats = DeliveryStats::default();
        for (state, count) in self.deps.deliveries.count_by_state_since(since).await? {
            use alkh_domain::alert::DeliveryState::*;
            match state {
                Failed => stats.failed += count,
                Active | Superseded | Recalled | RecallFailed | Resolved | Pending => {
                    stats.sent += count
                }
            }
        }
        // 「被抑制/跳过」不落投递表，从审计统计（重复、静默、过滤、策略抑制）。
        let skipped = self
            .deps
            .audit
            .list(AuditFilter {
                action: Some(alkh_domain::audit::action::HOOK_RECEIVED.to_string()),
                outcome: Some("skipped".to_string()),
                since: Some(since),
                limit: 1,
                offset: 0,
            })
            .await?;
        stats.suppressed = skipped.total;

        let recent_events = self.deps.audit.recent(10).await?;

        Ok(Overview {
            channels: CountPair {
                total: channel_total,
                enabled: channel_enabled,
            },
            hooks: CountPair {
                total: hook_total,
                enabled: hook_enabled,
            },
            alert_groups: AlertGroupCounts { firing, resolved },
            deliveries_24h: stats,
            recent_events,
        })
    }

    pub async fn alert_groups(&self, filter: AlertGroupFilter) -> AppResult<Page<AlertGroup>> {
        let mut filter = filter;
        filter.limit = clamp_limit(filter.limit, 50, 200);
        Ok(self.deps.groups.list(filter).await?)
    }

    pub async fn alert_group_detail(&self, id: &AlertGroupId) -> AppResult<AlertGroupDetail> {
        let group = self
            .deps
            .groups
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("告警分组不存在"))?;

        let channel = self
            .deps
            .channels
            .get(&group.channel_id)
            .await?
            .map(|c| ChannelBrief {
                id: c.id,
                name: c.name,
                receive_id: c.receive_id,
                enabled: c.enabled,
            });
        let hook = self
            .deps
            .hooks
            .get(&group.hook_id)
            .await?
            .map(|h| HookBrief {
                id: h.id,
                name: h.name,
                token_prefix: h.token_prefix,
                enabled: h.enabled,
            });
        let deliveries = self.deps.deliveries.list_by_group(id).await?;

        Ok(AlertGroupDetail {
            group,
            channel,
            hook,
            deliveries,
        })
    }

    pub async fn deliveries(&self, filter: DeliveryFilter) -> AppResult<Page<Delivery>> {
        let mut filter = filter;
        filter.limit = clamp_limit(filter.limit, 50, 200);
        Ok(self.deps.deliveries.list(filter).await?)
    }

    pub async fn audit_logs(&self, filter: AuditFilter) -> AppResult<Page<AuditLog>> {
        let mut filter = filter;
        filter.limit = clamp_limit(filter.limit, 100, 500);
        Ok(self.deps.audit.list(filter).await?)
    }

    pub async fn get_delivery(&self, id: &alkh_domain::ids::DeliveryId) -> AppResult<Delivery> {
        self.deps
            .deliveries
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("投递记录不存在"))
    }

    pub async fn delete_alert_group(&self, id: &AlertGroupId) -> AppResult<()> {
        if self.deps.groups.get(id).await?.is_none() {
            return Err(AppError::not_found("告警分组不存在"));
        }
        self.deps.groups.delete(id).await?;
        Ok(())
    }
}

/// 构造预览/测试用的样例分组。
pub fn sample_alert_group(
    channel: &Channel,
    hook_id: alkh_domain::ids::HookId,
    resolved: bool,
    now: Timestamp,
) -> AlertGroup {
    let alerts = if resolved {
        json!([{
            "status": "resolved",
            "labels": {"alertname": "HighCPUUsage", "instance": "10.0.0.1:9100", "job": "node-exporter"},
            "annotations": {"summary": "CPU 使用率已回落", "description": "当前 21%，低于阈值 80%", "runbook_url": "https://runbook.example.com/high-cpu"},
            "startsAt": (now - chrono::Duration::minutes(40)).to_rfc3339(),
            "endsAt": now.to_rfc3339(),
            "fingerprint": "sample-fp-1"
        }])
    } else {
        json!([
            {
                "status": "firing",
                "labels": {"alertname": "HighCPUUsage", "instance": "10.0.0.1:9100", "job": "node-exporter", "team": "sre"},
                "annotations": {"summary": "CPU 使用率 92% 超过阈值 80%", "description": "持续 5 分钟；runbook: https://runbook.example.com/high-cpu"},
                "startsAt": (now - chrono::Duration::minutes(5)).to_rfc3339(),
                "endsAt": "0001-01-01T00:00:00Z",
                "fingerprint": "sample-fp-1"
            },
            {
                "status": "firing",
                "labels": {"alertname": "HighMemoryUsage", "instance": "10.0.0.2:9100", "job": "node-exporter", "team": "sre"},
                "annotations": {"summary": "内存使用率 88%", "description": "持续 3 分钟"},
                "startsAt": (now - chrono::Duration::minutes(3)).to_rfc3339(),
                "endsAt": "0001-01-01T00:00:00Z",
                "fingerprint": "sample-fp-2"
            },
            {
                "status": "resolved",
                "labels": {"alertname": "DiskSpaceLow", "instance": "10.0.0.3:9100", "job": "node-exporter"},
                "annotations": {"summary": "磁盘空间已恢复"},
                "startsAt": (now - chrono::Duration::minutes(30)).to_rfc3339(),
                "endsAt": now.to_rfc3339(),
                "fingerprint": "sample-fp-3"
            }
        ])
    };

    let payload: AlertManagerPayload = serde_json::from_value(json!({
        "version": "4",
        "groupKey": "{}:{alertname=~\"High.*|DiskSpaceLow\"}",
        "truncatedAlerts": 0,
        "status": if resolved { "resolved" } else { "firing" },
        "receiver": "lark-alerts",
        "groupLabels": {"alertname": "HighCPUUsage"},
        "commonLabels": {"severity": if resolved { "info" } else { "critical" }, "team": "sre"},
        "commonAnnotations": {"summary": "样例告警（由管理后台测试触发）"},
        "externalURL": "http://alertmanager.example.com:9093",
        "alerts": alerts
    }))
    .expect("样例 payload 必须合法");

    let event = NormalizedEvent::from_payload(&payload).expect("样例事件必须可归一化");
    AlertGroup::new(hook_id, channel.id.clone(), &event, now)
}

/// 供单元测试构造一个最小渠道。
pub fn sample_channel() -> Channel {
    Channel::new(
        "样例渠道",
        "oc_sample",
        ReceiveIdType::ChatId,
        alkh_domain::channel::ChannelKind::Group,
        ChannelSource::Manual,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_group_is_firing_with_mixed_alerts() {
        let ch = sample_channel();
        let g = sample_alert_group(
            &ch,
            alkh_domain::ids::HookId::new(),
            false,
            chrono::Utc::now(),
        );
        assert!(g.invariants_hold());
        assert_eq!(g.alerts.len(), 3);
        assert_eq!(g.firing_count, 2);
        assert_eq!(g.resolved_count, 1);
        assert_eq!(g.status, alkh_domain::alert::GroupStatus::Firing);
        assert!(g.severity.is_some());
    }

    #[test]
    fn sample_group_resolved_variant() {
        let ch = sample_channel();
        let g = sample_alert_group(
            &ch,
            alkh_domain::ids::HookId::new(),
            true,
            chrono::Utc::now(),
        );
        assert!(g.invariants_hold());
        assert_eq!(g.firing_count, 0);
        assert_eq!(g.status, alkh_domain::alert::GroupStatus::Resolved);
        assert!(g.resolved_at.is_some());
    }
}
