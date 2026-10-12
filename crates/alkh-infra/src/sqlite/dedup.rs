//! `EventDedup` 的 SQLite 实现：投递幂等去重窗口（R9）。

use alkh_domain::error::PortResult;
use alkh_domain::ids::HookId;
use alkh_domain::ports::EventDedup;
use alkh_domain::Timestamp;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use sqlx::SqlitePool;

use super::common::{self, ts};

/// 幂等去重表。主键为 `(hook_id, event_hash)`。
#[derive(Debug, Clone)]
pub struct SqliteEventDedup {
    pool: SqlitePool,
}

impl SqliteEventDedup {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EventDedup for SqliteEventDedup {
    async fn seen_recently(
        &self,
        hook_id: &HookId,
        hash: &str,
        window_secs: i64,
    ) -> PortResult<bool> {
        let cutoff = Utc::now() - Duration::seconds(window_secs.max(0));
        let row = sqlx::query(
            "SELECT 1 FROM event_dedup WHERE hook_id = ? AND event_hash = ? AND seen_at >= ?",
        )
        .bind(hook_id.as_str())
        .bind(hash)
        .bind(ts(cutoff))
        .fetch_optional(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(row.is_some())
    }

    async fn record(&self, hook_id: &HookId, hash: &str, at: Timestamp) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO event_dedup (hook_id, event_hash, seen_at) VALUES (?, ?, ?) \
             ON CONFLICT(hook_id, event_hash) DO UPDATE SET seen_at = excluded.seen_at",
        )
        .bind(hook_id.as_str())
        .bind(hash)
        .bind(ts(at))
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn purge_before(&self, before: Timestamp) -> PortResult<u64> {
        let result = sqlx::query("DELETE FROM event_dedup WHERE seen_at < ?")
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

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn window_is_respected_for_old_and_recent_records() {
        let dedup = SqliteEventDedup::new(pool().await);
        let hook = HookId::new();
        let now = Utc::now();

        assert!(!dedup.seen_recently(&hook, "h1", 300).await.unwrap());

        // 100 秒前的事件：60 秒窗口内不再命中，1 小时窗口内仍命中。
        dedup
            .record(&hook, "h1", now - Duration::seconds(100))
            .await
            .unwrap();
        assert!(!dedup.seen_recently(&hook, "h1", 60).await.unwrap());
        assert!(dedup.seen_recently(&hook, "h1", 3600).await.unwrap());

        // 其他 hook / 其他 hash 互不影响。
        assert!(!dedup
            .seen_recently(&HookId::new(), "h1", 3600)
            .await
            .unwrap());
        assert!(!dedup.seen_recently(&hook, "h2", 3600).await.unwrap());
    }

    #[tokio::test]
    async fn record_is_an_upsert_updating_seen_at() {
        let dedup = SqliteEventDedup::new(pool().await);
        let hook = HookId::new();
        let now = Utc::now();

        dedup
            .record(&hook, "h", now - Duration::seconds(100))
            .await
            .unwrap();
        dedup.record(&hook, "h", now).await.unwrap();

        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM event_dedup")
            .fetch_one(&dedup.pool)
            .await
            .unwrap();
        assert_eq!(rows, 1);
        let seen_at: String = sqlx::query_scalar("SELECT seen_at FROM event_dedup")
            .fetch_one(&dedup.pool)
            .await
            .unwrap();
        assert_eq!(seen_at, ts(now));
        assert!(dedup.seen_recently(&hook, "h", 10).await.unwrap());
    }

    #[tokio::test]
    async fn purge_before_deletes_only_old_rows() {
        let dedup = SqliteEventDedup::new(pool().await);
        let hook = HookId::new();
        let now = Utc::now();

        dedup
            .record(&hook, "old", now - Duration::days(2))
            .await
            .unwrap();
        dedup.record(&hook, "new", now).await.unwrap();

        assert_eq!(
            dedup.purge_before(now - Duration::days(1)).await.unwrap(),
            1
        );
        assert!(!dedup.seen_recently(&hook, "old", 3600).await.unwrap());
        assert!(dedup.seen_recently(&hook, "new", 3600).await.unwrap());
        assert_eq!(
            dedup.purge_before(now + Duration::days(1)).await.unwrap(),
            1
        );
    }
}
