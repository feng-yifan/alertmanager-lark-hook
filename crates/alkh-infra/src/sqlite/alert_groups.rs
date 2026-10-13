//! `AlertGroupRepository` 的 SQLite 实现。

use alkh_domain::alert::{AlertGroup, AlertInstance, GroupStatus};
use alkh_domain::error::PortResult;
use alkh_domain::ids::{AlertGroupId, ChannelId, DeliveryId, HookId};
use alkh_domain::ports::{AlertGroupFilter, AlertGroupRepository};
use alkh_domain::{Annotations, Labels, Page, Timestamp};
use async_trait::async_trait;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use super::common::{self, enum_to_str, to_json, ts, ts_opt, RowExt};

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// 把过滤条件追加到查询构造器上（`count` 与 `list` 共用，保证 WHERE 完全一致）。
fn push_filter(qb: &mut QueryBuilder<'_, Sqlite>, filter: &AlertGroupFilter) {
    qb.push(" WHERE 1=1");
    if let Some(status) = filter.status {
        qb.push(" AND status = ").push_bind(enum_to_str(&status));
    }
    if let Some(channel_id) = &filter.channel_id {
        qb.push(" AND channel_id = ")
            .push_bind(channel_id.as_str().to_string());
    }
    if let Some(severity) = filter
        .severity
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        qb.push(" AND severity = ").push_bind(severity.to_string());
    }
    if let Some(q) = filter.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        let pattern = format!("%{}%", escape_like(q));
        qb.push(" AND (group_key LIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE '\\' OR alertname LIKE ")
            .push_bind(pattern)
            .push(" ESCAPE '\\')");
    }
}

fn row_to_group(row: &SqliteRow) -> PortResult<AlertGroup> {
    Ok(AlertGroup {
        id: AlertGroupId::from_string(row.str_col("id")?),
        hook_id: HookId::from_string(row.str_col("hook_id")?),
        channel_id: ChannelId::from_string(row.str_col("channel_id")?),
        group_key: row.str_col("group_key")?,
        receiver: row.str_col("receiver")?,
        status: row.enum_col("status")?,
        alertname: row.opt_str_col("alertname")?,
        severity: row.opt_str_col("severity")?,
        group_labels: row.json_col::<Labels>("group_labels"),
        common_labels: row.json_col::<Labels>("common_labels"),
        common_annotations: row.json_col::<Annotations>("common_annotations"),
        alerts: row.json_col::<Vec<AlertInstance>>("alerts"),
        firing_count: row.u32_col("firing_count")?,
        resolved_count: row.u32_col("resolved_count")?,
        truncated_alerts: row.u64_col("truncated_alerts")?,
        event_count: row.u64_col("event_count")?,
        last_event_hash: row.opt_str_col("last_event_hash")?,
        external_url: row.opt_str_col("external_url")?,
        first_seen_at: row.ts_col("first_seen_at")?,
        last_seen_at: row.ts_col("last_seen_at")?,
        last_notified_at: row.opt_ts_col("last_notified_at")?,
        resolved_at: row.opt_ts_col("resolved_at")?,
        active_delivery_id: row
            .opt_str_col("active_delivery_id")?
            .map(DeliveryId::from_string),
        pinned_message_id: row.opt_str_col("pinned_message_id")?,
    })
}

/// 告警分组仓储。
#[derive(Debug, Clone)]
pub struct SqliteAlertGroupRepository {
    pool: SqlitePool,
}

impl SqliteAlertGroupRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AlertGroupRepository for SqliteAlertGroupRepository {
    async fn find(&self, hook_id: &HookId, group_key: &str) -> PortResult<Option<AlertGroup>> {
        let row = sqlx::query("SELECT * FROM alert_groups WHERE hook_id = ? AND group_key = ?")
            .bind(hook_id.as_str())
            .bind(group_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_group).transpose()
    }

    async fn get(&self, id: &AlertGroupId) -> PortResult<Option<AlertGroup>> {
        let row = sqlx::query("SELECT * FROM alert_groups WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_group).transpose()
    }

    async fn upsert(&self, group: &AlertGroup) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO alert_groups \
             (id, hook_id, channel_id, group_key, receiver, status, alertname, severity, \
              group_labels, common_labels, common_annotations, alerts, firing_count, \
              resolved_count, truncated_alerts, event_count, last_event_hash, external_url, \
              first_seen_at, last_seen_at, last_notified_at, resolved_at, active_delivery_id, \
              pinned_message_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
              hook_id = excluded.hook_id, channel_id = excluded.channel_id, \
              group_key = excluded.group_key, receiver = excluded.receiver, \
              status = excluded.status, alertname = excluded.alertname, \
              severity = excluded.severity, group_labels = excluded.group_labels, \
              common_labels = excluded.common_labels, \
              common_annotations = excluded.common_annotations, alerts = excluded.alerts, \
              firing_count = excluded.firing_count, resolved_count = excluded.resolved_count, \
              truncated_alerts = excluded.truncated_alerts, event_count = excluded.event_count, \
              last_event_hash = excluded.last_event_hash, external_url = excluded.external_url, \
              first_seen_at = excluded.first_seen_at, last_seen_at = excluded.last_seen_at, \
              last_notified_at = excluded.last_notified_at, resolved_at = excluded.resolved_at, \
              active_delivery_id = excluded.active_delivery_id, \
              pinned_message_id = excluded.pinned_message_id",
        )
        .bind(group.id.as_str())
        .bind(group.hook_id.as_str())
        .bind(group.channel_id.as_str())
        .bind(&group.group_key)
        .bind(&group.receiver)
        .bind(enum_to_str(&group.status))
        .bind(group.alertname.as_deref())
        .bind(group.severity.as_deref())
        .bind(to_json(&group.group_labels))
        .bind(to_json(&group.common_labels))
        .bind(to_json(&group.common_annotations))
        .bind(to_json(&group.alerts))
        .bind(i64::from(group.firing_count))
        .bind(i64::from(group.resolved_count))
        .bind(group.truncated_alerts as i64)
        .bind(group.event_count as i64)
        .bind(group.last_event_hash.as_deref())
        .bind(group.external_url.as_deref())
        .bind(ts(group.first_seen_at))
        .bind(ts(group.last_seen_at))
        .bind(ts_opt(group.last_notified_at))
        .bind(ts_opt(group.resolved_at))
        .bind(group.active_delivery_id.as_ref().map(|d| d.as_str()))
        .bind(group.pinned_message_id.as_deref())
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn set_active_delivery(
        &self,
        id: &AlertGroupId,
        delivery_id: Option<&DeliveryId>,
        notified_at: Option<Timestamp>,
    ) -> PortResult<()> {
        match notified_at {
            Some(at) => {
                sqlx::query(
                    "UPDATE alert_groups SET active_delivery_id = ?, last_notified_at = ? WHERE id = ?",
                )
                .bind(delivery_id.map(|d| d.as_str()))
                .bind(ts(at))
                .bind(id.as_str())
                .execute(&self.pool)
                .await
                .map_err(common::map_sql)?;
            }
            None => {
                sqlx::query("UPDATE alert_groups SET active_delivery_id = ? WHERE id = ?")
                    .bind(delivery_id.map(|d| d.as_str()))
                    .bind(id.as_str())
                    .execute(&self.pool)
                    .await
                    .map_err(common::map_sql)?;
            }
        }
        Ok(())
    }

    async fn list(&self, filter: AlertGroupFilter) -> PortResult<Page<AlertGroup>> {
        let limit = if filter.limit == 0 { 50 } else { filter.limit };
        let offset = filter.offset;

        let mut count_qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM alert_groups");
        push_filter(&mut count_qb, &filter);
        let total: i64 = count_qb
            .build_query_scalar()
            .fetch_one(&self.pool)
            .await
            .map_err(common::map_sql)?;

        let mut qb = QueryBuilder::<Sqlite>::new("SELECT * FROM alert_groups");
        push_filter(&mut qb, &filter);
        qb.push(" ORDER BY last_seen_at DESC LIMIT ")
            .push_bind(i64::from(limit))
            .push(" OFFSET ")
            .push_bind(i64::from(offset));
        let rows = qb
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(common::map_sql)?;
        let items = rows
            .iter()
            .map(row_to_group)
            .collect::<PortResult<Vec<_>>>()?;

        Ok(Page::new(items, total.max(0) as u64, limit, offset))
    }

    async fn delete(&self, id: &AlertGroupId) -> PortResult<()> {
        sqlx::query("DELETE FROM alert_groups WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(())
    }

    async fn count_by_status(&self) -> PortResult<(u64, u64)> {
        let firing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alert_groups WHERE status = ?")
            .bind(enum_to_str(&GroupStatus::Firing))
            .fetch_one(&self.pool)
            .await
            .map_err(common::map_sql)?;
        let resolved: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM alert_groups WHERE status = ?")
                .bind(enum_to_str(&GroupStatus::Resolved))
                .fetch_one(&self.pool)
                .await
                .map_err(common::map_sql)?;
        Ok((firing.max(0) as u64, resolved.max(0) as u64))
    }

    async fn delete_before(&self, before: Timestamp) -> PortResult<u64> {
        let result = sqlx::query("DELETE FROM alert_groups WHERE last_seen_at < ?")
            .bind(ts(before))
            .execute(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alkh_domain::alert::{AlertManagerPayload, NormalizedEvent};

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    fn sample_group(
        hook_id: HookId,
        channel_id: ChannelId,
        alertname: &str,
        group_key: &str,
    ) -> AlertGroup {
        let payload: AlertManagerPayload = serde_json::from_value(serde_json::json!({
            "version": "4",
            "groupKey": group_key,
            "status": "firing",
            "receiver": "lark",
            "commonLabels": {"alertname": alertname, "severity": "critical"},
            "commonAnnotations": {"summary": "cpu high"},
            "alerts": [{
                "status": "firing",
                "labels": {"alertname": alertname, "instance": "a:9100"},
                "fingerprint": "fp1"
            }]
        }))
        .unwrap();
        let event = NormalizedEvent::from_payload(&payload).unwrap();
        AlertGroup::new(hook_id, channel_id, &event, alkh_domain::now())
    }

    #[tokio::test]
    async fn upsert_get_and_natural_key_lookup() {
        let repo = SqliteAlertGroupRepository::new(pool().await);
        let hook = HookId::new();
        let channel = ChannelId::new();
        let g = {
            let mut g = sample_group(
                hook.clone(),
                channel.clone(),
                "HighCPU",
                "{alertname=\"HighCPU\"}",
            );
            g.pinned_message_id = Some("om_pinned".into());
            g
        };
        repo.upsert(&g).await.unwrap();

        let got = repo.get(&g.id).await.unwrap().unwrap();
        assert_eq!(got.group_key, g.group_key);
        assert_eq!(got.pinned_message_id.as_deref(), Some("om_pinned"));
        assert_eq!(got.status, GroupStatus::Firing);
        assert_eq!(got.alertname.as_deref(), Some("HighCPU"));
        assert_eq!(got.severity.as_deref(), Some("critical"));
        assert_eq!(got.firing_count, 1);
        assert_eq!(got.event_count, 1);
        assert_eq!(got.alerts.len(), 1);
        assert_eq!(
            got.common_annotations.get("summary").map(String::as_str),
            Some("cpu high")
        );

        let found = repo.find(&hook, &g.group_key).await.unwrap().unwrap();
        assert_eq!(found.id, g.id);
        assert!(repo.find(&hook, "{other}").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_twice_updates_and_natural_key_conflict_is_error() {
        let repo = SqliteAlertGroupRepository::new(pool().await);
        let hook = HookId::new();
        let channel = ChannelId::new();
        let mut g = sample_group(hook.clone(), channel, "HighCPU", "{a=\"1\"}");
        repo.upsert(&g).await.unwrap();

        g.status = GroupStatus::Resolved;
        g.alerts.clear();
        g.firing_count = 0;
        g.resolved_count = 1;
        g.event_count = 2;
        g.resolved_at = Some(alkh_domain::now());
        repo.upsert(&g).await.unwrap();

        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alert_groups")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(total, 1);
        let got = repo.get(&g.id).await.unwrap().unwrap();
        assert_eq!(got.status, GroupStatus::Resolved);
        assert_eq!(got.event_count, 2);
        assert!(got.resolved_at.is_some());

        // 同一自然键 (hook_id, group_key) 的另一条记录必须报冲突，而不是静默吞掉。
        let other = sample_group(hook, got.channel_id.clone(), "HighCPU", "{a=\"1\"}");
        let err = repo.upsert(&other).await.unwrap_err();
        assert!(
            matches!(err, alkh_domain::PortError::Conflict(_)),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn list_filters_pagination_and_ordering() {
        let repo = SqliteAlertGroupRepository::new(pool().await);
        let hook = HookId::new();
        let channel = ChannelId::new();
        let base = alkh_domain::now();

        for i in 0..5i64 {
            let mut g = sample_group(
                hook.clone(),
                channel.clone(),
                &format!("Alert{i}"),
                &format!("{{k=\"{i}\"}}"),
            );
            g.last_seen_at = base + chrono::Duration::seconds(i);
            if i == 4 {
                g.status = GroupStatus::Resolved;
            }
            repo.upsert(&g).await.unwrap();
        }

        let page = repo
            .list(AlertGroupFilter {
                q: Some("Alert".into()),
                limit: 2,
                offset: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 5);
        assert_eq!(page.items.len(), 2);
        // last_seen_at DESC：最新的是 Alert4。
        assert_eq!(page.items[0].alertname.as_deref(), Some("Alert4"));
        assert_eq!(page.items[1].alertname.as_deref(), Some("Alert3"));

        let fired = repo
            .list(AlertGroupFilter {
                status: Some(GroupStatus::Firing),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(fired.total, 4);

        let by_channel = repo
            .list(AlertGroupFilter {
                channel_id: Some(channel),
                severity: Some("critical".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(by_channel.total, 5);

        // 搜索同时覆盖 group_key。
        let by_key = repo
            .list(AlertGroupFilter {
                q: Some("k=\"3\"".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(by_key.total, 1);
        assert_eq!(by_key.items[0].alertname.as_deref(), Some("Alert3"));
    }

    #[tokio::test]
    async fn active_delivery_status_count_and_delete_before() {
        let repo = SqliteAlertGroupRepository::new(pool().await);
        let hook = HookId::new();
        let channel = ChannelId::new();
        let base = alkh_domain::now();

        let mut g = sample_group(hook.clone(), channel, "HighCPU", "{a=\"1\"}");
        g.last_seen_at = base - chrono::Duration::days(40);
        repo.upsert(&g).await.unwrap();
        let mut recent = sample_group(hook, g.channel_id.clone(), "Recent", "{a=\"2\"}");
        recent.status = GroupStatus::Resolved;
        recent.resolved_at = Some(base);
        repo.upsert(&recent).await.unwrap();

        let delivery = DeliveryId::new();
        let notified = base + chrono::Duration::seconds(5);
        repo.set_active_delivery(&g.id, Some(&delivery), Some(notified))
            .await
            .unwrap();
        let got = repo.get(&g.id).await.unwrap().unwrap();
        assert_eq!(got.active_delivery_id.as_ref(), Some(&delivery));
        assert_eq!(got.last_notified_at, Some(notified));

        // 不带 notified_at 时保留上次通知时间，仅清除活跃引用。
        repo.set_active_delivery(&g.id, None, None).await.unwrap();
        let got = repo.get(&g.id).await.unwrap().unwrap();
        assert_eq!(got.active_delivery_id, None);
        assert_eq!(got.last_notified_at, Some(notified));

        assert_eq!(repo.count_by_status().await.unwrap(), (1, 1));

        assert_eq!(
            repo.delete_before(base - chrono::Duration::days(7))
                .await
                .unwrap(),
            1
        );
        assert!(repo.get(&g.id).await.unwrap().is_none());
        assert!(repo.get(&recent.id).await.unwrap().is_some());
    }
}
