//! `ChannelRepository` 的 SQLite 实现。

use alkh_domain::channel::{Channel, ReceiveIdType};
use alkh_domain::error::PortResult;
use alkh_domain::ids::ChannelId;
use alkh_domain::ports::{ChannelFilter, ChannelRepository};
use alkh_domain::Page;
use async_trait::async_trait;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use super::common::{self, b, enum_to_str, ts, RowExt};

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// 把过滤条件追加到查询构造器上（`count` 与 `list` 共用，保证 WHERE 完全一致）。
fn push_filter(qb: &mut QueryBuilder<'_, Sqlite>, filter: &ChannelFilter) {
    qb.push(" WHERE 1=1");
    if let Some(enabled) = filter.enabled {
        qb.push(" AND enabled = ").push_bind(b(enabled));
    }
    if let Some(q) = filter.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        qb.push(" AND name LIKE ")
            .push_bind(format!("%{}%", escape_like(q)))
            .push(" ESCAPE '\\'");
    }
}

fn row_to_channel(row: &SqliteRow) -> PortResult<Channel> {
    Ok(Channel {
        id: ChannelId::from_string(row.str_col("id")?),
        name: row.str_col("name")?,
        receive_id: row.str_col("receive_id")?,
        receive_id_type: row.enum_col("receive_id_type")?,
        kind: row.enum_col("kind")?,
        avatar_url: row.opt_str_col("avatar_url")?,
        external: row.bool_col("external")?,
        bot_can_send: row.bool_col("bot_can_send")?,
        enabled: row.bool_col("enabled")?,
        source: row.enum_col("source")?,
        created_at: row.ts_col("created_at")?,
        updated_at: row.ts_col("updated_at")?,
    })
}

/// 渠道仓储。
#[derive(Debug, Clone)]
pub struct SqliteChannelRepository {
    pool: SqlitePool,
}

impl SqliteChannelRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChannelRepository for SqliteChannelRepository {
    async fn list(&self, filter: ChannelFilter) -> PortResult<Page<Channel>> {
        let limit = if filter.limit == 0 { 50 } else { filter.limit };
        let offset = filter.offset;

        let mut count_qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM channels");
        push_filter(&mut count_qb, &filter);
        let total: i64 = count_qb
            .build_query_scalar()
            .fetch_one(&self.pool)
            .await
            .map_err(common::map_sql)?;

        let mut qb = QueryBuilder::<Sqlite>::new("SELECT * FROM channels");
        push_filter(&mut qb, &filter);
        qb.push(" ORDER BY created_at ASC LIMIT ")
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
            .map(row_to_channel)
            .collect::<PortResult<Vec<_>>>()?;

        Ok(Page::new(items, total.max(0) as u64, limit, offset))
    }

    async fn get(&self, id: &ChannelId) -> PortResult<Option<Channel>> {
        let row = sqlx::query("SELECT * FROM channels WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_channel).transpose()
    }

    async fn find_by_receive_id(
        &self,
        receive_id_type: ReceiveIdType,
        receive_id: &str,
    ) -> PortResult<Option<Channel>> {
        let row =
            sqlx::query("SELECT * FROM channels WHERE receive_id_type = ? AND receive_id = ?")
                .bind(enum_to_str(&receive_id_type))
                .bind(receive_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(common::map_sql)?;
        row.as_ref().map(row_to_channel).transpose()
    }

    async fn upsert(&self, channel: &Channel) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO channels \
             (id, name, receive_id, receive_id_type, kind, avatar_url, external, bot_can_send, \
              enabled, source, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
              name = excluded.name, receive_id = excluded.receive_id, \
              receive_id_type = excluded.receive_id_type, kind = excluded.kind, \
              avatar_url = excluded.avatar_url, external = excluded.external, \
              bot_can_send = excluded.bot_can_send, enabled = excluded.enabled, \
              source = excluded.source, updated_at = excluded.updated_at",
        )
        .bind(channel.id.as_str())
        .bind(&channel.name)
        .bind(&channel.receive_id)
        .bind(enum_to_str(&channel.receive_id_type))
        .bind(enum_to_str(&channel.kind))
        .bind(channel.avatar_url.as_deref())
        .bind(b(channel.external))
        .bind(b(channel.bot_can_send))
        .bind(b(channel.enabled))
        .bind(enum_to_str(&channel.source))
        .bind(ts(channel.created_at))
        .bind(ts(channel.updated_at))
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn delete(&self, id: &ChannelId) -> PortResult<()> {
        sqlx::query("DELETE FROM channels WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(())
    }

    async fn count(&self) -> PortResult<(u64, u64)> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels")
            .fetch_one(&self.pool)
            .await
            .map_err(common::map_sql)?;
        let enabled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE enabled = 1")
            .fetch_one(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok((total.max(0) as u64, enabled.max(0) as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alkh_domain::channel::{ChannelKind, ChannelSource};

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    fn sample(name: &str, receive_id: &str) -> Channel {
        Channel::from_chat(receive_id, name, None, false)
    }

    #[tokio::test]
    async fn insert_get_roundtrip() {
        let repo = SqliteChannelRepository::new(pool().await);
        let c = sample("运维群", "oc_1");
        repo.upsert(&c).await.unwrap();

        let got = repo.get(&c.id).await.unwrap().unwrap();
        assert_eq!(got.name, "运维群");
        assert_eq!(got.receive_id, "oc_1");
        assert_eq!(got.receive_id_type, ReceiveIdType::ChatId);
        assert_eq!(got.kind, ChannelKind::Group);
        assert_eq!(got.source, ChannelSource::FeishuSync);
        assert!(got.enabled && got.bot_can_send && !got.external);
        assert_eq!(got.created_at, c.created_at);

        let by_receive = repo
            .find_by_receive_id(ReceiveIdType::ChatId, "oc_1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_receive.id, c.id);
        assert!(repo
            .find_by_receive_id(ReceiveIdType::ChatId, "oc_missing")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn upsert_twice_updates_without_duplicating() {
        let repo = SqliteChannelRepository::new(pool().await);
        let mut c = sample("ops", "oc_1");
        repo.upsert(&c).await.unwrap();
        c.name = "ops-renamed".into();
        c.enabled = false;
        repo.upsert(&c).await.unwrap();

        let (total, enabled) = repo.count().await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(enabled, 0);
        let got = repo.get(&c.id).await.unwrap().unwrap();
        assert_eq!(got.name, "ops-renamed");
        assert!(!got.enabled);
    }

    #[tokio::test]
    async fn list_filters_and_reports_total() {
        let repo = SqliteChannelRepository::new(pool().await);
        for i in 0..5u32 {
            let mut c = sample(&format!("team-{i}"), &format!("oc_{i}"));
            c.enabled = i % 2 == 0;
            repo.upsert(&c).await.unwrap();
        }

        let page = repo
            .list(ChannelFilter {
                q: Some("team-".into()),
                limit: 2,
                offset: 1,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 5);
        assert_eq!(page.items.len(), 2);
        assert_eq!((page.limit, page.offset), (2, 1));

        let enabled = repo
            .list(ChannelFilter {
                enabled: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(enabled.total, 3);

        // LIKE 通配符被转义：`%` 不再匹配任意串。
        let literal = repo
            .list(ChannelFilter {
                q: Some("%".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(literal.total, 0);

        let missing = repo
            .list(ChannelFilter {
                q: Some("nope".into()),
                limit: 0,
                offset: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(missing.total, 0);
        assert_eq!(missing.limit, 50);
        assert!(missing.items.is_empty());

        repo.delete(&page.items[0].id).await.unwrap();
        assert_eq!(repo.count().await.unwrap().0, 4);
    }
}
