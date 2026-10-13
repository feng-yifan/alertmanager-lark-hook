//! 迁移 `0003_hook_token_recoverable` 的回归测试。
//!
//! 0003 在已有的 `hooks` 表上追加两列（token 的 AES-256-GCM 密文），用于在管理后台
//! 随时回显 / 复制完整 Hook 地址。它必须能安全跑在有历史数据的库上：
//! 旧行两列为 NULL（前端提示轮换一次即可恢复），新行写入密文后仍能用 `token_hash` 查找。
//!
//! 不用 `sqlx::migrate!` 是因为它会把 0001→0003 连着跑完，没法插「升级前」的数据。

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

const MIGRATION_0001: &str = include_str!("../../../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../../../migrations/0002_remove_app_feed_card.sql");
const MIGRATION_0003: &str = include_str!("../../../migrations/0003_hook_token_recoverable.sql");

async fn pool_with_0001() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("内存 SQLite 连接失败");
    sqlx::raw_sql(MIGRATION_0001)
        .execute(&pool)
        .await
        .expect("0001 迁移失败");
    pool
}

async fn column_exists(pool: &SqlitePool, table: &str, column: &str) -> bool {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .expect("PRAGMA 失败");
    rows.iter().any(|r| r.get::<String, _>("name") == column)
}

#[tokio::test]
async fn upgrade_from_a_database_with_existing_hooks() {
    let pool = pool_with_0001().await;

    sqlx::query(
        "INSERT INTO channels (id, name, receive_id, receive_id_type, kind, enabled, source, \
         created_at, updated_at) VALUES ('c1', '运维告警群', 'oc_1', 'chat_id', 'group', 1, \
         'manual', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    // 升级前签发的 Hook：只有哈希与前缀，没有密文列
    sqlx::query(
        "INSERT INTO hooks (id, channel_id, name, token_hash, token_prefix, enabled, call_count, \
         created_at) VALUES ('h1', 'c1', 'ops', 'deadbeef', 'alkh_a1b2', 1, 3, \
         '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(MIGRATION_0002)
        .execute(&pool)
        .await
        .expect("0002 迁移失败");
    sqlx::raw_sql(MIGRATION_0003)
        .execute(&pool)
        .await
        .expect("0003 迁移失败");

    // 1. 新列已追加
    for column in ["token_nonce", "token_ciphertext"] {
        assert!(
            column_exists(&pool, "hooks", column).await,
            "hooks.{column} 应已被追加"
        );
    }

    // 2. 历史 Hook 保留，且两列为 NULL（地址暂不可回显，需轮换）
    let row = sqlx::query(
        "SELECT token_hash, call_count, token_nonce, token_ciphertext FROM hooks WHERE id = 'h1'",
    )
    .fetch_one(&pool)
    .await
    .expect("历史 Hook 必须保留");
    assert_eq!(row.get::<String, _>("token_hash"), "deadbeef");
    assert_eq!(row.get::<i64, _>("call_count"), 3);
    assert!(row.get::<Option<String>, _>("token_nonce").is_none());
    assert!(row.get::<Option<String>, _>("token_ciphertext").is_none());

    // 3. 迁移后的表能写入密文，并且 token_hash 查找仍然可用
    sqlx::query(
        "INSERT INTO hooks (id, channel_id, name, token_hash, token_prefix, token_nonce, \
         token_ciphertext, enabled, call_count, created_at) \
         VALUES ('h2', 'c1', 'ops2', 'cafebabe', 'alkh_c3d4', 'bm9uY2U=', 'Y2lwaGVy', 1, 0, \
         '2026-01-02T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("迁移后应能写入密文列");

    let found: String = sqlx::query_scalar("SELECT id FROM hooks WHERE token_hash = 'cafebabe'")
        .fetch_one(&pool)
        .await
        .expect("token_hash 查找必须仍可用");
    assert_eq!(found, "h2");
}
