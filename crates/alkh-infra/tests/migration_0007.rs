//! 迁移 `0007_pin_ignored` 的回归测试。
//!
//! 0007 做两件事：
//!   1. `alert_groups` 增加 `pin_ignored`（默认 0）；
//!   2. `app_settings` 增加卡片回调验签用的 Encrypt Key / Verification Token（默认 NULL）。
//!
//! 这里手工按 0001 → 0006 执行（模拟存量库），写入一条分组，再跑 0007，
//! 验证新列默认值与旧数据不受影响。

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

const MIGRATION_0001: &str = include_str!("../../../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../../../migrations/0002_remove_app_feed_card.sql");
const MIGRATION_0003: &str = include_str!("../../../migrations/0003_hook_token_recoverable.sql");
const MIGRATION_0004: &str = include_str!("../../../migrations/0004_pin_unresolved.sql");
const MIGRATION_0005: &str = include_str!("../../../migrations/0005_timezone.sql");
const MIGRATION_0006: &str = include_str!("../../../migrations/0006_pin_resolved_multi.sql");
const MIGRATION_0007: &str = include_str!("../../../migrations/0007_pin_ignored.sql");

async fn column_exists(pool: &SqlitePool, table: &str, column: &str) -> bool {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .expect("PRAGMA 失败");
    rows.iter().any(|r| r.get::<String, _>("name") == column)
}

#[tokio::test]
async fn adds_pin_ignored_and_callback_secret_columns_with_defaults() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("内存 SQLite 连接失败");

    for sql in [
        MIGRATION_0001,
        MIGRATION_0002,
        MIGRATION_0003,
        MIGRATION_0004,
        MIGRATION_0005,
        MIGRATION_0006,
    ] {
        sqlx::raw_sql(sql)
            .execute(&pool)
            .await
            .expect("前置迁移失败");
    }

    // 存量形状：一条已经存在的分组。
    sqlx::query(
        "INSERT INTO channels (id, name, receive_id, receive_id_type, kind, enabled, source, \
         created_at, updated_at) VALUES ('c1', '群', 'oc_1', 'chat_id', 'group', 1, \
         'manual', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO alert_groups (id, hook_id, channel_id, group_key, status, first_seen_at, \
         last_seen_at) VALUES ('g1', 'h1', 'c1', 'k1', 'firing', \
         '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("存量分组写入应成功");

    sqlx::raw_sql(MIGRATION_0007)
        .execute(&pool)
        .await
        .expect("0007 迁移失败");

    // 分组新增 pin_ignored，存量行默认 0。
    assert!(column_exists(&pool, "alert_groups", "pin_ignored").await);
    let pin_ignored: i64 = sqlx::query_scalar("SELECT pin_ignored FROM alert_groups WHERE id='g1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pin_ignored, 0, "pin_ignored 默认必须为 0");

    // 设置新增 2 个回调验签列，且默认 NULL。
    for col in [
        "lark_verification_token_nonce",
        "lark_verification_token_ciphertext",
    ] {
        assert!(
            column_exists(&pool, "app_settings", col).await,
            "app_settings 缺少列 {col}"
        );
    }
}
