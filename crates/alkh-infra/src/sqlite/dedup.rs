//! `EventDedup` 的 SQLite 实现：投递幂等去重窗口（R9）。

use alkh_domain::error::PortResult;
use alkh_domain::ids::HookId;
use alkh_domain::ports::EventDedup;
use alkh_domain::Timestamp;
use async_trait::async_trait;
use chrono::Duration;
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
    async fn claim(
        &self,
        hook_id: &HookId,
        hash: &str,
        at: Timestamp,
        window_secs: i64,
    ) -> PortResult<bool> {
        // 单条语句完成「检查 + 写入」，因此对并发请求是原子的：
        //   - 没有记录            → INSERT，changes()=1  ⇒ 认领成功
        //   - 有记录且在窗口内    → DO UPDATE 的 WHERE 不成立，不更新，changes()=0 ⇒ 重复
        //   - 有记录但已过窗口    → DO UPDATE 刷新 seen_at，changes()=1 ⇒ 重新认领
        //
        // 注意 cutoff 由传入的 `at` 推导而不是 `Utc::now()`，这样时钟走
        // `Clock` 端口，测试与校时场景下行为一致。
        let cutoff = at - Duration::seconds(window_secs.max(0));
        let result = sqlx::query(
            "INSERT INTO event_dedup (hook_id, event_hash, seen_at) VALUES (?, ?, ?) \
             ON CONFLICT(hook_id, event_hash) DO UPDATE SET seen_at = excluded.seen_at \
             WHERE event_dedup.seen_at < ?",
        )
        .bind(hook_id.as_str())
        .bind(hash)
        .bind(ts(at))
        .bind(ts(cutoff))
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(result.rows_affected() == 1)
    }

    async fn release(&self, hook_id: &HookId, hash: &str) -> PortResult<()> {
        sqlx::query("DELETE FROM event_dedup WHERE hook_id = ? AND event_hash = ?")
            .bind(hook_id.as_str())
            .bind(hash)
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
    use chrono::Utc;

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
    async fn claim_is_window_aware_and_scoped_per_hook() {
        let dedup = SqliteEventDedup::new(pool().await);
        let hook = HookId::new();
        let now = Utc::now();

        assert!(
            dedup.claim(&hook, "h1", now, 60).await.unwrap(),
            "首次应认领"
        );
        assert!(
            !dedup.claim(&hook, "h1", now, 60).await.unwrap(),
            "窗口内的重复不应认领"
        );

        // 其他 hook / 其他 hash 互不影响。
        assert!(dedup.claim(&HookId::new(), "h1", now, 60).await.unwrap());
        assert!(dedup.claim(&hook, "h2", now, 60).await.unwrap());

        // 窗口滑过后可以重新认领，且是更新而非插入新行（避免表无限增长）。
        let later = now + Duration::seconds(61);
        assert!(
            dedup.claim(&hook, "h1", later, 60).await.unwrap(),
            "窗口外应可重新认领"
        );
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM event_dedup")
            .fetch_one(&dedup.pool)
            .await
            .unwrap();
        assert_eq!(rows, 3, "重新认领应更新既有行而不是新增一行");
    }

    /// 回归（P0-1）：处理失败归还认领后，AlertManager 的重试必须能被处理。
    ///
    /// 修复前的实现是「先 record 记录去重键、再处理事件」，且失败时不回滚。
    /// 于是首次处理失败返回 5xx 后，AM 在秒级重试会命中 60s 去重窗口，
    /// 被判为 `duplicate` 并以 **200** 应答 —— AM 认为投递成功，该告警永久丢失。
    #[tokio::test]
    async fn released_claim_allows_retry_after_failure() {
        let dedup = SqliteEventDedup::new(pool().await);
        let hook = HookId::new();
        let now = Utc::now();

        assert!(
            dedup.claim(&hook, "h", now, 60).await.unwrap(),
            "首次应认领"
        );

        // 模拟处理失败：归还认领。
        dedup.release(&hook, "h").await.unwrap();

        assert!(
            dedup.claim(&hook, "h", now, 60).await.unwrap(),
            "失败归还后的认领必须允许重试，否则重试被当作重复并返回 200，告警静默丢失"
        );
    }

    #[tokio::test]
    async fn purge_before_deletes_only_old_rows() {
        let dedup = SqliteEventDedup::new(pool().await);
        let hook = HookId::new();
        let now = Utc::now();

        dedup
            .claim(&hook, "old", now - Duration::days(2), 60)
            .await
            .unwrap();
        dedup.claim(&hook, "new", now, 60).await.unwrap();

        assert_eq!(
            dedup.purge_before(now - Duration::days(1)).await.unwrap(),
            1
        );
        assert!(
            dedup.claim(&hook, "old", now, 60).await.unwrap(),
            "旧行已被清理，应可重新认领"
        );
        assert!(!dedup.claim(&hook, "new", now, 60).await.unwrap());
        assert_eq!(
            dedup.purge_before(now + Duration::days(1)).await.unwrap(),
            2
        );
    }
}
