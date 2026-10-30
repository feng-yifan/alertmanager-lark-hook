//! `NotificationPolicyRepository` 的 SQLite 实现。

use alkh_domain::error::PortResult;
use alkh_domain::ids::{CardTemplateId, ChannelId};
use alkh_domain::policy::{LabelMatcher, NotificationPolicy};
use alkh_domain::ports::NotificationPolicyRepository;
use async_trait::async_trait;
use sqlx::sqlite::SqliteRow;
use sqlx::SqlitePool;

use super::common::{self, enum_to_str, to_json, ts, ts_opt, RowExt};

fn row_to_policy(row: &SqliteRow) -> PortResult<NotificationPolicy> {
    Ok(NotificationPolicy {
        channel_id: ChannelId::from_string(row.str_col("channel_id")?),
        card_template_id: row
            .opt_str_col("card_template_id")?
            .map(CardTemplateId::from_string),
        pin_unresolved: row.bool_col("pin_unresolved")?,
        unpin_on_resolve: row.bool_col("unpin_on_resolve")?,
        pin_resolved: row.bool_col("pin_resolved")?,
        repeat_policy: row.enum_col("repeat_policy")?,
        resolved_policy: row.enum_col("resolved_policy")?,
        group_update_policy: row.enum_col("group_update_policy")?,
        label_matchers: row.json_col::<Vec<LabelMatcher>>("label_matchers"),
        aggregation_window_secs: row.u32_col("aggregation_window_secs")?,
        min_send_interval_secs: row.u32_col("min_send_interval_secs")?,
        max_alerts_per_message: row.u32_col("max_alerts_per_message")?,
        mute_until: row.opt_ts_col("mute_until")?,
        updated_at: row.ts_col("updated_at")?,
    })
}

/// 通知策略仓储。
#[derive(Debug, Clone)]
pub struct SqliteNotificationPolicyRepository {
    pool: SqlitePool,
}

impl SqliteNotificationPolicyRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl NotificationPolicyRepository for SqliteNotificationPolicyRepository {
    async fn get(&self, channel_id: &ChannelId) -> PortResult<Option<NotificationPolicy>> {
        let row = sqlx::query("SELECT * FROM notification_policies WHERE channel_id = ?")
            .bind(channel_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_policy).transpose()
    }

    async fn upsert(&self, policy: &NotificationPolicy) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO notification_policies \
             (channel_id, card_template_id, pin_unresolved, unpin_on_resolve, pin_resolved, \
              repeat_policy, resolved_policy, group_update_policy, label_matchers, \
              aggregation_window_secs, min_send_interval_secs, max_alerts_per_message, \
              mute_until, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(channel_id) DO UPDATE SET \
              card_template_id = excluded.card_template_id, \
              pin_unresolved = excluded.pin_unresolved, \
              unpin_on_resolve = excluded.unpin_on_resolve, \
              pin_resolved = excluded.pin_resolved, \
              repeat_policy = excluded.repeat_policy, \
              resolved_policy = excluded.resolved_policy, \
              group_update_policy = excluded.group_update_policy, \
              label_matchers = excluded.label_matchers, \
              aggregation_window_secs = excluded.aggregation_window_secs, \
              min_send_interval_secs = excluded.min_send_interval_secs, \
              max_alerts_per_message = excluded.max_alerts_per_message, \
              mute_until = excluded.mute_until, updated_at = excluded.updated_at",
        )
        .bind(policy.channel_id.as_str())
        .bind(policy.card_template_id.as_ref().map(|t| t.as_str()))
        .bind(policy.pin_unresolved)
        .bind(policy.unpin_on_resolve)
        .bind(policy.pin_resolved)
        .bind(enum_to_str(&policy.repeat_policy))
        .bind(enum_to_str(&policy.resolved_policy))
        .bind(enum_to_str(&policy.group_update_policy))
        .bind(to_json(&policy.label_matchers))
        .bind(i64::from(policy.aggregation_window_secs))
        .bind(i64::from(policy.min_send_interval_secs))
        .bind(i64::from(policy.max_alerts_per_message))
        .bind(ts_opt(policy.mute_until))
        .bind(ts(policy.updated_at))
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn count_by_template(&self, id: &CardTemplateId) -> PortResult<u64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_policies WHERE card_template_id = ?",
        )
        .bind(id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(count.max(0) as u64)
    }

    async fn delete_for_channel(&self, channel_id: &ChannelId) -> PortResult<()> {
        sqlx::query("DELETE FROM notification_policies WHERE channel_id = ?")
            .bind(channel_id.as_str())
            .execute(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::channels::SqliteChannelRepository;
    use alkh_domain::channel::Channel;
    use alkh_domain::policy::{GroupUpdatePolicy, MatchOp, RepeatPolicy, ResolvedPolicy};
    use alkh_domain::ports::ChannelRepository;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    async fn seed_channel(pool: &SqlitePool) -> ChannelId {
        let repo = SqliteChannelRepository::new(pool.clone());
        let c = Channel::from_chat("oc_1", "ops", None, false);
        repo.upsert(&c).await.unwrap();
        c.id
    }

    #[tokio::test]
    async fn insert_get_roundtrip() {
        let pool = pool().await;
        let channel_id = seed_channel(&pool).await;
        let repo = SqliteNotificationPolicyRepository::new(pool);

        let mut p = NotificationPolicy::default_for(channel_id.clone());
        p.card_template_id = Some(CardTemplateId::new());
        p.pin_unresolved = true;
        p.unpin_on_resolve = false;
        p.pin_resolved = true;
        p.repeat_policy = RepeatPolicy::SendAndRevokePrevious;
        p.resolved_policy = ResolvedPolicy::UpdatePrevious;
        p.group_update_policy = GroupUpdatePolicy::Send;
        p.label_matchers = vec![LabelMatcher::new("severity", MatchOp::Eq, "critical")];
        p.aggregation_window_secs = 30;
        p.min_send_interval_secs = 5;
        p.max_alerts_per_message = 42;
        repo.upsert(&p).await.unwrap();

        let got = repo.get(&channel_id).await.unwrap().unwrap();
        assert_eq!(got.card_template_id, p.card_template_id);
        assert!(got.pin_unresolved, "pin_unresolved 必须落库并回读");
        assert!(!got.unpin_on_resolve, "unpin_on_resolve 必须落库并回读");
        assert!(got.pin_resolved, "pin_resolved 必须落库并回读");
        assert_eq!(got.repeat_policy, RepeatPolicy::SendAndRevokePrevious);
        assert_eq!(got.resolved_policy, ResolvedPolicy::UpdatePrevious);
        assert_eq!(got.group_update_policy, GroupUpdatePolicy::Send);
        assert_eq!(got.label_matchers, p.label_matchers);
        assert_eq!(got.aggregation_window_secs, 30);
        assert_eq!(got.min_send_interval_secs, 5);
        assert_eq!(got.max_alerts_per_message, 42);
        assert_eq!(got.mute_until, None);
    }

    #[tokio::test]
    async fn upsert_twice_updates_in_place() {
        let pool = pool().await;
        let channel_id = seed_channel(&pool).await;
        let repo = SqliteNotificationPolicyRepository::new(pool);

        let mut p = NotificationPolicy::default_for(channel_id.clone());
        repo.upsert(&p).await.unwrap();
        let template = CardTemplateId::new();
        p.card_template_id = Some(template.clone());
        p.repeat_policy = RepeatPolicy::Send;
        p.max_alerts_per_message = 3;
        repo.upsert(&p).await.unwrap();

        // 同 channel_id 只有一行。
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notification_policies")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(repo.count_by_template(&template).await.unwrap(), 1);

        let got = repo.get(&channel_id).await.unwrap().unwrap();
        assert_eq!(got.repeat_policy, RepeatPolicy::Send);
        assert_eq!(got.max_alerts_per_message, 3);
    }

    #[tokio::test]
    async fn count_by_template_and_delete_for_channel() {
        let pool = pool().await;
        let repo = SqliteNotificationPolicyRepository::new(pool.clone());

        let mut ids = Vec::new();
        for i in 0..3u32 {
            let c = Channel::from_chat(format!("oc_{i}"), format!("team-{i}"), None, false);
            SqliteChannelRepository::new(pool.clone())
                .upsert(&c)
                .await
                .unwrap();
            ids.push(c.id);
        }
        let template = CardTemplateId::new();
        for (i, id) in ids.iter().enumerate() {
            let mut p = NotificationPolicy::default_for(id.clone());
            if i < 2 {
                p.card_template_id = Some(template.clone());
            }
            repo.upsert(&p).await.unwrap();
        }

        assert_eq!(repo.count_by_template(&template).await.unwrap(), 2);
        assert_eq!(
            repo.count_by_template(&CardTemplateId::new())
                .await
                .unwrap(),
            0
        );

        repo.delete_for_channel(&ids[0]).await.unwrap();
        assert!(repo.get(&ids[0]).await.unwrap().is_none());
        assert_eq!(repo.count_by_template(&template).await.unwrap(), 1);
    }
}
