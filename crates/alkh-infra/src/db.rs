//! SQLite 连接池与迁移。

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

/// 打开（必要时创建）SQLite 数据库。
///
/// - WAL 日志模式：读写并发（管理后台读 + 后台任务写）
/// - `busy_timeout`：写锁竞争时等待而不是立刻 `SQLITE_BUSY`
/// - 外键约束：保证级联删除生效
pub async fn connect(database_url: &str, max_connections: u32) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(15))
        .foreign_keys(true)
        .pragma("temp_store", "MEMORY");

    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections.max(1))
        .acquire_timeout(Duration::from_secs(15))
        .connect_with(options)
        .await?;

    Ok(pool)
}

/// 运行嵌入式迁移（编译期打包进二进制）。
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}

/// 就绪探针：确认数据库可读写。
pub async fn health_check(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

/// 优雅停机前把 WAL 落盘（best-effort）。
pub async fn checkpoint(pool: &SqlitePool) {
    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await;
}
