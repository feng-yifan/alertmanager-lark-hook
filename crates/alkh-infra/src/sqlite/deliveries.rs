//! `DeliveryRepository` 的 SQLite 实现。

use alkh_domain::alert::{Delivery, DeliveryState};
use alkh_domain::error::PortResult;
use alkh_domain::ids::{AlertGroupId, CardTemplateId, ChannelId, DeliveryId};
use alkh_domain::ports::{DeliveryFilter, DeliveryRepository};
use alkh_domain::{Page, Timestamp};
use async_trait::async_trait;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use super::common::{self, enum_to_str, ts, RowExt};

/// 把过滤条件追加到查询构造器上（`count` 与 `list` 共用，保证 WHERE 完全一致）。
fn push_filter(qb: &mut QueryBuilder<'_, Sqlite>, filter: &DeliveryFilter) {
    qb.push(" WHERE 1=1");
    if let Some(group_id) = &filter.alert_group_id {
        qb.push(" AND alert_group_id = ")
            .push_bind(group_id.as_str().to_string());
    }
    if let Some(channel_id) = &filter.channel_id {
        qb.push(" AND channel_id = ")
            .push_bind(channel_id.as_str().to_string());
    }
    if let Some(state) = filter.state {
        qb.push(" AND state = ").push_bind(enum_to_str(&state));
    }
}

fn row_to_delivery(row: &SqliteRow) -> PortResult<Delivery> {
    Ok(Delivery {
        id: DeliveryId::from_string(row.str_col("id")?),
        alert_group_id: AlertGroupId::from_string(row.str_col("alert_group_id")?),
        channel_id: ChannelId::from_string(row.str_col("channel_id")?),
        kind: row.enum_col("kind")?,
        message_type: row.enum_col("message_type")?,
        state: row.enum_col("state")?,
        card_template_id: row
            .opt_str_col("card_template_id")?
            .map(CardTemplateId::from_string),
        lark_message_id: row.opt_str_col("lark_message_id")?,
        card_entity_id: row.opt_str_col("card_entity_id")?,
        app_feed_biz_id: row.opt_str_col("app_feed_biz_id")?,
        card_sequence: row.i64_col("card_sequence")?,
        rendered_digest: row.str_col("rendered_digest")?,
        attempt: row.u32_col("attempt")?,
        last_error: row.opt_str_col("last_error")?,
        created_at: row.ts_col("created_at")?,
        updated_at: row.ts_col("updated_at")?,
    })
}

/// 投递仓储。
#[derive(Debug, Clone)]
pub struct SqliteDeliveryRepository {
    pool: SqlitePool,
}

impl SqliteDeliveryRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DeliveryRepository for SqliteDeliveryRepository {
    async fn insert(&self, delivery: &Delivery) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO deliveries \
             (id, alert_group_id, channel_id, kind, message_type, state, card_template_id, \
              lark_message_id, card_entity_id, app_feed_biz_id, card_sequence, rendered_digest, \
              attempt, last_error, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(delivery.id.as_str())
        .bind(delivery.alert_group_id.as_str())
        .bind(delivery.channel_id.as_str())
        .bind(enum_to_str(&delivery.kind))
        .bind(enum_to_str(&delivery.message_type))
        .bind(enum_to_str(&delivery.state))
        .bind(delivery.card_template_id.as_ref().map(|t| t.as_str()))
        .bind(delivery.lark_message_id.as_deref())
        .bind(delivery.card_entity_id.as_deref())
        .bind(delivery.app_feed_biz_id.as_deref())
        .bind(delivery.card_sequence)
        .bind(&delivery.rendered_digest)
        .bind(i64::from(delivery.attempt))
        .bind(delivery.last_error.as_deref())
        .bind(ts(delivery.created_at))
        .bind(ts(delivery.updated_at))
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn update(&self, delivery: &Delivery) -> PortResult<()> {
        sqlx::query(
            "UPDATE deliveries SET alert_group_id = ?, channel_id = ?, kind = ?, message_type = ?, \
             state = ?, card_template_id = ?, lark_message_id = ?, card_entity_id = ?, \
             app_feed_biz_id = ?, card_sequence = ?, rendered_digest = ?, attempt = ?, \
             last_error = ?, updated_at = ? WHERE id = ?",
        )
        .bind(delivery.alert_group_id.as_str())
        .bind(delivery.channel_id.as_str())
        .bind(enum_to_str(&delivery.kind))
        .bind(enum_to_str(&delivery.message_type))
        .bind(enum_to_str(&delivery.state))
        .bind(delivery.card_template_id.as_ref().map(|t| t.as_str()))
        .bind(delivery.lark_message_id.as_deref())
        .bind(delivery.card_entity_id.as_deref())
        .bind(delivery.app_feed_biz_id.as_deref())
        .bind(delivery.card_sequence)
        .bind(&delivery.rendered_digest)
        .bind(i64::from(delivery.attempt))
        .bind(delivery.last_error.as_deref())
        .bind(ts(delivery.updated_at))
        .bind(delivery.id.as_str())
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn get(&self, id: &DeliveryId) -> PortResult<Option<Delivery>> {
        let row = sqlx::query("SELECT * FROM deliveries WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_delivery).transpose()
    }

    async fn list(&self, filter: DeliveryFilter) -> PortResult<Page<Delivery>> {
        let limit = if filter.limit == 0 { 50 } else { filter.limit };
        let offset = filter.offset;

        let mut count_qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM deliveries");
        push_filter(&mut count_qb, &filter);
        let total: i64 = count_qb
            .build_query_scalar()
            .fetch_one(&self.pool)
            .await
            .map_err(common::map_sql)?;

        let mut qb = QueryBuilder::<Sqlite>::new("SELECT * FROM deliveries");
        push_filter(&mut qb, &filter);
        qb.push(" ORDER BY created_at DESC LIMIT ")
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
            .map(row_to_delivery)
            .collect::<PortResult<Vec<_>>>()?;

        Ok(Page::new(items, total.max(0) as u64, limit, offset))
    }

    async fn list_by_group(&self, alert_group_id: &AlertGroupId) -> PortResult<Vec<Delivery>> {
        let rows = sqlx::query(
            "SELECT * FROM deliveries WHERE alert_group_id = ? ORDER BY created_at DESC",
        )
        .bind(alert_group_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(common::map_sql)?;
        rows.iter().map(row_to_delivery).collect()
    }

    async fn find_active(&self, alert_group_id: &AlertGroupId) -> PortResult<Option<Delivery>> {
        let row = sqlx::query(
            "SELECT * FROM deliveries WHERE alert_group_id = ? AND state = ? \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(alert_group_id.as_str())
        .bind(enum_to_str(&DeliveryState::Active))
        .fetch_optional(&self.pool)
        .await
        .map_err(common::map_sql)?;
        row.as_ref().map(row_to_delivery).transpose()
    }

    async fn list_retryable(&self, limit: u32) -> PortResult<Vec<Delivery>> {
        let limit = if limit == 0 { 50 } else { limit };
        let rows = sqlx::query(
            "SELECT * FROM deliveries WHERE state = ? AND attempt < 5 \
             ORDER BY updated_at ASC LIMIT ?",
        )
        .bind(enum_to_str(&DeliveryState::Pending))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(common::map_sql)?;
        rows.iter().map(row_to_delivery).collect()
    }

    async fn count_by_state_since(
        &self,
        since: Timestamp,
    ) -> PortResult<Vec<(DeliveryState, u64)>> {
        let rows = sqlx::query(
            "SELECT state, COUNT(*) AS cnt FROM deliveries WHERE updated_at >= ? GROUP BY state",
        )
        .bind(ts(since))
        .fetch_all(&self.pool)
        .await
        .map_err(common::map_sql)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            out.push((row.enum_col::<DeliveryState>("state")?, row.u64_col("cnt")?));
        }
        Ok(out)
    }

    async fn delete_before(&self, before: Timestamp) -> PortResult<u64> {
        // ⚠️ 必须排除仍被 `alert_groups.active_delivery_id` 引用的行。
        //
        // 长期 firing 且内容不变的分组会命中 R3「内容未变化则跳过更新」提前返回，
        // 因而不会刷新 deliveries.updated_at，投递行会先于分组被清理掉，
        // `active_delivery_id` 就成了悬空指针 —— 之后告警恢复时 previous=None，
        // `decide` 判成 Suppress，恢复通知静默不发，群里那张卡片永远停在「未解决」。
        let result = sqlx::query(
            "DELETE FROM deliveries WHERE updated_at < ? AND id NOT IN (\
                 SELECT active_delivery_id FROM alert_groups \
                 WHERE active_delivery_id IS NOT NULL\
             )",
        )
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
    use alkh_domain::alert::DeliveryKind;
    use alkh_domain::policy::MessageType;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    fn sample(group: &AlertGroupId, channel: &ChannelId, now: Timestamp) -> Delivery {
        Delivery::new(
            group.clone(),
            channel.clone(),
            DeliveryKind::FiringCreated,
            MessageType::CardMessage,
            None,
            "digest",
            now,
        )
    }

    #[tokio::test]
    async fn insert_get_roundtrip() {
        let repo = SqliteDeliveryRepository::new(pool().await);
        let now = alkh_domain::now();
        let mut d = sample(&AlertGroupId::new(), &ChannelId::new(), now);
        d.card_template_id = Some(CardTemplateId::new());
        d.lark_message_id = Some("om_1".into());
        d.card_entity_id = Some("card_1".into());
        d.app_feed_biz_id = Some("biz_1".into());
        d.card_sequence = 3;
        d.attempt = 2;
        d.last_error = Some("boom".into());
        repo.insert(&d).await.unwrap();

        let got = repo.get(&d.id).await.unwrap().unwrap();
        assert_eq!(got.kind, DeliveryKind::FiringCreated);
        assert_eq!(got.message_type, MessageType::CardMessage);
        assert_eq!(got.state, DeliveryState::Pending);
        assert_eq!(got.card_template_id, d.card_template_id);
        assert_eq!(got.lark_message_id.as_deref(), Some("om_1"));
        assert_eq!(got.card_entity_id.as_deref(), Some("card_1"));
        assert_eq!(got.app_feed_biz_id.as_deref(), Some("biz_1"));
        assert_eq!(got.card_sequence, 3);
        assert_eq!(got.rendered_digest, "digest");
        assert_eq!(got.attempt, 2);
        assert_eq!(got.last_error.as_deref(), Some("boom"));
        assert_eq!(got.created_at, now);
    }

    #[tokio::test]
    async fn update_and_find_active() {
        let repo = SqliteDeliveryRepository::new(pool().await);
        let now = alkh_domain::now();
        let group = AlertGroupId::new();
        let mut old = sample(&group, &ChannelId::new(), now);
        old.mark_active(now);
        old.lark_message_id = Some("om_old".into());
        repo.insert(&old).await.unwrap();

        let mut newest = sample(
            &group,
            &ChannelId::new(),
            now + chrono::Duration::seconds(1),
        );
        newest.mark_active(now + chrono::Duration::seconds(1));
        repo.insert(&newest).await.unwrap();

        let other = sample(&AlertGroupId::new(), &ChannelId::new(), now);
        repo.insert(&other).await.unwrap();

        let active = repo.find_active(&group).await.unwrap().unwrap();
        assert_eq!(active.id, newest.id);
        assert_eq!(active.state, DeliveryState::Active);

        // 更新为新状态后不再被视为 active。
        newest.mark_recalled(now);
        newest.lark_message_id = None;
        repo.update(&newest).await.unwrap();
        let got = repo.get(&newest.id).await.unwrap().unwrap();
        assert_eq!(got.state, DeliveryState::Recalled);
        assert!(got.lark_message_id.is_none());

        let active = repo.find_active(&group).await.unwrap().unwrap();
        assert_eq!(active.id, old.id);
        assert!(repo
            .find_active(&AlertGroupId::new())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn list_filters_pagination_and_ordering() {
        let repo = SqliteDeliveryRepository::new(pool().await);
        let group = AlertGroupId::new();
        let channel = ChannelId::new();
        let base = alkh_domain::now();

        for i in 0..5i64 {
            let mut d = sample(&group, &channel, base + chrono::Duration::seconds(i));
            if i % 2 == 0 {
                d.mark_failed("x", base + chrono::Duration::seconds(i));
            }
            repo.insert(&d).await.unwrap();
        }
        let mut other = sample(&AlertGroupId::new(), &channel, base);
        other.mark_failed("y", base);
        repo.insert(&other).await.unwrap();

        let page = repo
            .list(DeliveryFilter {
                alert_group_id: Some(group.clone()),
                limit: 2,
                offset: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 5);
        assert_eq!(page.items.len(), 2);
        // created_at DESC。
        assert_eq!(
            page.items[0].created_at,
            base + chrono::Duration::seconds(4)
        );

        let failed = repo
            .list(DeliveryFilter {
                state: Some(DeliveryState::Failed),
                limit: 50,
                offset: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(failed.total, 4);

        let by_channel = repo
            .list(DeliveryFilter {
                channel_id: Some(channel),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(by_channel.total, 6);

        let by_group = repo.list_by_group(&group).await.unwrap();
        assert_eq!(by_group.len(), 5);
    }

    #[tokio::test]
    async fn retryable_state_counts_and_delete_before() {
        let repo = SqliteDeliveryRepository::new(pool().await);
        let group = AlertGroupId::new();
        let channel = ChannelId::new();
        let base = alkh_domain::now();

        // pending + attempt=1 → 可重试。
        let mut retry = sample(&group, &channel, base);
        retry.record_attempt(Some("err".into()), base);
        repo.insert(&retry).await.unwrap();

        // pending 但 attempt=5 → 不可重试。
        let mut exhausted = sample(&group, &channel, base + chrono::Duration::seconds(1));
        exhausted.attempt = 5;
        exhausted.updated_at = base + chrono::Duration::seconds(1);
        repo.insert(&exhausted).await.unwrap();

        // active → 不在重试集合。
        let mut active = sample(&group, &channel, base + chrono::Duration::seconds(2));
        active.mark_active(base + chrono::Duration::seconds(2));
        repo.insert(&active).await.unwrap();

        let retryable = repo.list_retryable(10).await.unwrap();
        assert_eq!(retryable.len(), 1);
        assert_eq!(retryable[0].id, retry.id);
        assert!(repo.list_retryable(0).await.unwrap().len() <= 50);

        let counts = repo.count_by_state_since(base).await.unwrap();
        let pending = counts
            .iter()
            .find(|(s, _)| *s == DeliveryState::Pending)
            .unwrap()
            .1;
        let active_count = counts
            .iter()
            .find(|(s, _)| *s == DeliveryState::Active)
            .unwrap()
            .1;
        assert_eq!(pending, 2);
        assert_eq!(active_count, 1);

        let after = repo
            .count_by_state_since(base + chrono::Duration::days(1))
            .await
            .unwrap();
        assert!(after.is_empty());

        assert_eq!(
            repo.delete_before(base + chrono::Duration::seconds(1))
                .await
                .unwrap(),
            1
        );
        assert!(repo.get(&retry.id).await.unwrap().is_none());
        assert!(repo.get(&active.id).await.unwrap().is_some());
    }
    /// 回归（P0-6）：保留清理不得删除仍被 `alert_groups.active_delivery_id` 引用的投递。
    ///
    /// 长期 firing 且内容不变的分组会命中 R3「内容未变化则跳过更新」提前返回，
    /// 因而不会刷新 `deliveries.updated_at`：投递行会先于分组被清理掉，
    /// `active_delivery_id` 变成悬空指针 → 告警恢复时 `previous = None` →
    /// `decide` 判成 Suppress，恢复通知静默丢失、群里卡片永远停在「未解决」。
    #[tokio::test]
    async fn delete_before_keeps_deliveries_referenced_by_alert_groups() {
        let repo = SqliteDeliveryRepository::new(pool().await);
        let now = alkh_domain::now();
        let old = now - chrono::Duration::days(60);
        let group = AlertGroupId::new();
        let channel = ChannelId::new();

        // 一条被分组引用为活跃消息，一条无人引用
        let referenced = sample(&group, &channel, old);
        let orphan = sample(&AlertGroupId::new(), &channel, old);
        repo.insert(&referenced).await.unwrap();
        repo.insert(&orphan).await.unwrap();

        sqlx::query(
            "INSERT INTO alert_groups (id, hook_id, channel_id, group_key, status, \
             first_seen_at, last_seen_at, active_delivery_id) \
             VALUES (?, 'h1', ?, 'g1', 'firing', ?, ?, ?)",
        )
        .bind(group.as_str())
        .bind(channel.as_str())
        .bind(ts(now))
        .bind(ts(now))
        .bind(referenced.id.as_str())
        .execute(&repo.pool)
        .await
        .unwrap();

        let deleted = repo.delete_before(now).await.unwrap();
        assert_eq!(deleted, 1, "只应删除没有被引用的那条");
        assert!(
            repo.get(&referenced.id).await.unwrap().is_some(),
            "被 alert_groups.active_delivery_id 引用的投递必须保留，否则指针悬空、恢复通知会静默丢失"
        );
        assert!(
            repo.get(&orphan.id).await.unwrap().is_none(),
            "无人引用的旧投递应被清理"
        );
    }
}
