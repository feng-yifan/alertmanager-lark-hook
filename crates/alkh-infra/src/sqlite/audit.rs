//! `AuditLogRepository` 的 SQLite 实现。审计日志只追加。

use alkh_domain::audit::{AuditActor, AuditLog, Outcome};
use alkh_domain::error::PortResult;
use alkh_domain::ports::{AuditFilter, AuditLogRepository};
use alkh_domain::{Page, Timestamp};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use super::common::{self, from_json, to_json, ts, RowExt};

/// 把过滤条件追加到查询构造器上（`count` 与 `list` 共用，保证 WHERE 完全一致）。
fn push_filter(qb: &mut QueryBuilder<'_, Sqlite>, filter: &AuditFilter) {
    qb.push(" WHERE 1=1");
    if let Some(action) = filter
        .action
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        qb.push(" AND action = ").push_bind(action.to_string());
    }
    // `outcome` 列以带 tag 的 JSON 存储，按类型名过滤。
    if let Some(outcome) = filter
        .outcome
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        qb.push(" AND json_extract(outcome, '$.type') = ")
            .push_bind(outcome.to_string());
    }
    if let Some(since) = filter.since {
        qb.push(" AND at >= ").push_bind(ts(since));
    }
}

fn row_to_audit(row: &SqliteRow) -> PortResult<AuditLog> {
    Ok(AuditLog {
        id: row.i64_col("id")?,
        at: row.ts_col("at")?,
        actor: from_json::<AuditActor>(&row.str_col("actor")?)?,
        action: row.str_col("action")?,
        target: row.opt_str_col("target")?,
        outcome: from_json::<Outcome>(&row.str_col("outcome")?)?,
        detail: row.json_col::<Value>("detail"),
    })
}

async fn insert_one(tx: &mut sqlx::Transaction<'_, Sqlite>, log: &AuditLog) -> PortResult<()> {
    sqlx::query(
        "INSERT INTO audit_logs (at, actor, action, target, outcome, detail) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(ts(log.at))
    .bind(to_json(&log.actor))
    .bind(log.action.as_str())
    .bind(log.target.as_deref())
    .bind(to_json(&log.outcome))
    .bind(to_json(&log.detail))
    .execute(&mut **tx)
    .await
    .map_err(common::map_sql)?;
    Ok(())
}

/// 审计日志仓储。
#[derive(Debug, Clone)]
pub struct SqliteAuditLogRepository {
    pool: SqlitePool,
}

impl SqliteAuditLogRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AuditLogRepository for SqliteAuditLogRepository {
    async fn append(&self, log: &AuditLog) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO audit_logs (at, actor, action, target, outcome, detail) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(ts(log.at))
        .bind(to_json(&log.actor))
        .bind(log.action.as_str())
        .bind(log.target.as_deref())
        .bind(to_json(&log.outcome))
        .bind(to_json(&log.detail))
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn append_many(&self, logs: &[AuditLog]) -> PortResult<()> {
        let mut tx = self.pool.begin().await.map_err(common::map_sql)?;
        for log in logs {
            insert_one(&mut tx, log).await?;
        }
        tx.commit().await.map_err(common::map_sql)?;
        Ok(())
    }

    async fn list(&self, filter: AuditFilter) -> PortResult<Page<AuditLog>> {
        let limit = if filter.limit == 0 { 50 } else { filter.limit };
        let offset = filter.offset;

        let mut count_qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM audit_logs");
        push_filter(&mut count_qb, &filter);
        let total: i64 = count_qb
            .build_query_scalar()
            .fetch_one(&self.pool)
            .await
            .map_err(common::map_sql)?;

        let mut qb = QueryBuilder::<Sqlite>::new("SELECT * FROM audit_logs");
        push_filter(&mut qb, &filter);
        qb.push(" ORDER BY at DESC, id DESC LIMIT ")
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
            .map(row_to_audit)
            .collect::<PortResult<Vec<_>>>()?;

        Ok(Page::new(items, total.max(0) as u64, limit, offset))
    }

    async fn recent(&self, limit: u32) -> PortResult<Vec<AuditLog>> {
        let limit = if limit == 0 { 50 } else { limit };
        let rows = sqlx::query("SELECT * FROM audit_logs ORDER BY at DESC, id DESC LIMIT ?")
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(common::map_sql)?;
        rows.iter().map(row_to_audit).collect()
    }

    async fn delete_before(&self, before: Timestamp) -> PortResult<u64> {
        let result = sqlx::query("DELETE FROM audit_logs WHERE at < ?")
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
    use alkh_domain::audit::action;
    use serde_json::json;

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
    async fn append_and_recent_ordering() {
        let repo = SqliteAuditLogRepository::new(pool().await);
        let base = alkh_domain::now();

        let older = AuditLog::ok(AuditActor::admin("admin"), action::HOOK_CREATED, base)
            .target("hook-1")
            .detail(json!({"n": 1}));
        repo.append(&older).await.unwrap();

        // 同一时间戳的两条：id 更大的排在前面。
        let same_a = AuditLog::skipped(
            AuditActor::alertmanager("hook-1"),
            action::HOOK_RECEIVED,
            "duplicate",
            base + chrono::Duration::seconds(1),
        );
        let same_b = AuditLog::failed(
            AuditActor::system("retention"),
            action::DELIVERY_FAILED,
            "boom",
            base + chrono::Duration::seconds(1),
        );
        repo.append(&same_a).await.unwrap();
        repo.append(&same_b).await.unwrap();

        let recent = repo.recent(10).await.unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].action, action::DELIVERY_FAILED);
        assert_eq!(recent[1].action, action::HOOK_RECEIVED);
        assert_eq!(recent[2].action, action::HOOK_CREATED);

        let top = repo.recent(1).await.unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].action, action::DELIVERY_FAILED);

        let got = &recent[2];
        assert_eq!(got.actor, AuditActor::admin("admin"));
        assert_eq!(got.target.as_deref(), Some("hook-1"));
        assert_eq!(got.detail, json!({"n": 1}));
        assert_eq!(got.outcome, Outcome::Ok);
        assert!(recent[0].id > recent[1].id);
    }

    #[tokio::test]
    async fn list_filters_pagination_and_total() {
        let repo = SqliteAuditLogRepository::new(pool().await);
        let base = alkh_domain::now();

        for i in 0..5i64 {
            let at = base + chrono::Duration::seconds(i);
            let log = if i % 2 == 0 {
                AuditLog::skipped(
                    AuditActor::alertmanager("hook-1"),
                    action::HOOK_RECEIVED,
                    "duplicate",
                    at,
                )
            } else {
                AuditLog::ok(AuditActor::admin("admin"), action::MESSAGE_SENT, at)
            };
            repo.append(&log).await.unwrap();
        }

        let page = repo
            .list(AuditFilter {
                limit: 2,
                offset: 1,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 5);
        assert_eq!(page.items.len(), 2);
        assert_eq!((page.limit, page.offset), (2, 1));

        let received = repo
            .list(AuditFilter {
                action: Some(action::HOOK_RECEIVED.into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(received.total, 3);

        let skipped = repo
            .list(AuditFilter {
                outcome: Some("skipped".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(skipped.total, 3);
        assert!(skipped
            .items
            .iter()
            .all(|l| l.outcome.as_str() == "skipped"));

        let failed = repo
            .list(AuditFilter {
                outcome: Some("failed".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(failed.total, 0);

        let since = repo
            .list(AuditFilter {
                since: Some(base + chrono::Duration::seconds(3)),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(since.total, 2);
    }

    #[tokio::test]
    async fn append_many_is_transactional_and_delete_before_prunes() {
        let repo = SqliteAuditLogRepository::new(pool().await);
        let base = alkh_domain::now();

        let logs = vec![
            AuditLog::ok(
                AuditActor::system("a"),
                action::CHANNEL_SYNCED,
                base - chrono::Duration::days(30),
            ),
            AuditLog::ok(AuditActor::system("b"), action::CHANNEL_SYNCED, base),
            AuditLog::ok(AuditActor::system("c"), action::CHANNEL_SYNCED, base),
        ];
        repo.append_many(&logs).await.unwrap();
        assert_eq!(repo.recent(10).await.unwrap().len(), 3);

        // 空批次也是合法的。
        repo.append_many(&[]).await.unwrap();

        assert_eq!(
            repo.delete_before(base - chrono::Duration::days(1))
                .await
                .unwrap(),
            1
        );
        assert_eq!(repo.recent(10).await.unwrap().len(), 2);
    }
}
