//! 迁移 `0004_pin_unresolved` 的回归测试。
//!
//! 这条迁移要跑在**已经跑过 0002 的库**上：0002 把 `notification_policies.pin_unresolved`
//! 整列删掉了（当时判定 R4 无官方能力），0004 只是把开关加回来，并新增
//! `alert_groups.pinned_message_id`。若顺序或语句写错，升级会直接失败起不来。
//!
//! 这里手工按 0001 → 0002 → 0003 → 0004 执行，模拟 0.2.0 存量库的升级路径。

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

const MIGRATION_0001: &str = include_str!("../../../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../../../migrations/0002_remove_app_feed_card.sql");
const MIGRATION_0003: &str = include_str!("../../../migrations/0003_hook_token_recoverable.sql");
const MIGRATION_0004: &str = include_str!("../../../migrations/0004_pin_unresolved.sql");

async fn column_exists(pool: &SqlitePool, table: &str, column: &str) -> bool {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .expect("PRAGMA 失败");
    rows.iter().any(|r| r.get::<String, _>("name") == column)
}

#[tokio::test]
async fn upgrade_from_a_database_without_the_pin_columns() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("内存 SQLite 连接失败");

    for sql in [MIGRATION_0001, MIGRATION_0002, MIGRATION_0003] {
        sqlx::raw_sql(sql)
            .execute(&pool)
            .await
            .expect("前置迁移失败");
    }
    assert!(
        !column_exists(&pool, "notification_policies", "pin_unresolved").await,
        "0002 之后该列必须不存在（否则本测试没有覆盖「重新加回来」的路径）"
    );

    sqlx::raw_sql(MIGRATION_0004)
        .execute(&pool)
        .await
        .expect("0004 迁移失败");

    assert!(column_exists(&pool, "notification_policies", "pin_unresolved").await);
    assert!(column_exists(&pool, "alert_groups", "pinned_message_id").await);

    // 存量策略升级后默认关闭；新代码按老列形状写入也能拿到默认值。
    sqlx::query(
        "INSERT INTO channels (id, name, receive_id, receive_id_type, kind, enabled, source, \
         created_at, updated_at) VALUES ('c1', '群', 'oc_1', 'chat_id', 'group', 1, \
         'manual', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO notification_policies (channel_id, card_template_id, repeat_policy, \
         resolved_policy, group_update_policy, label_matchers, aggregation_window_secs, \
         min_send_interval_secs, max_alerts_per_message, updated_at) \
         VALUES ('c1', NULL, 'suppress', 'send', 'update_previous', '[]', 0, 0, 20, \
         '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("存量形状的策略写入应成功（新列有默认值）");
    let pin: i64 = sqlx::query_scalar(
        "SELECT pin_unresolved FROM notification_policies WHERE channel_id='c1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pin, 0, "新开关必须默认关闭");

    // 分组可以记录 / 清空 Pin 消息。
    sqlx::query(
        "INSERT INTO alert_groups (id, hook_id, channel_id, group_key, status, first_seen_at, \
         last_seen_at, pinned_message_id) VALUES ('g1', 'h1', 'c1', 'k1', 'firing', \
         '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'om_1')",
    )
    .execute(&pool)
    .await
    .expect("Pin 消息应能写入");
    sqlx::query("UPDATE alert_groups SET pinned_message_id = NULL WHERE id = 'g1'")
        .execute(&pool)
        .await
        .expect("Pin 消息应能清空");
}
