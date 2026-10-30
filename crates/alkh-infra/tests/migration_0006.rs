//! 迁移 `0006_pin_resolved_multi` 的回归测试。
//!
//! 0006 做两件事：
//!   1. 策略上新增 `unpin_on_resolve`（默认 1）与 `pin_resolved`（默认 0）；
//!   2. 分组的单值 `pinned_message_id` 升级为 JSON 数组 `pinned_messages`，并把旧值
//!      转成一条 `unresolved` 记录后删除旧列。
//!
//! 这里手工按 0001 → 0005 执行（模拟存量库），写入一条「旧的单 Pin」分组，再跑 0006，
//! 验证新列默认值、旧 Pin 迁移与旧列被删除。

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

const MIGRATION_0001: &str = include_str!("../../../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../../../migrations/0002_remove_app_feed_card.sql");
const MIGRATION_0003: &str = include_str!("../../../migrations/0003_hook_token_recoverable.sql");
const MIGRATION_0004: &str = include_str!("../../../migrations/0004_pin_unresolved.sql");
const MIGRATION_0005: &str = include_str!("../../../migrations/0005_timezone.sql");
const MIGRATION_0006: &str = include_str!("../../../migrations/0006_pin_resolved_multi.sql");

async fn column_exists(pool: &SqlitePool, table: &str, column: &str) -> bool {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .expect("PRAGMA 失败");
    rows.iter().any(|r| r.get::<String, _>("name") == column)
}

#[tokio::test]
async fn upgrade_moves_single_pin_to_multi_and_adds_switches() {
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
    ] {
        sqlx::raw_sql(sql)
            .execute(&pool)
            .await
            .expect("前置迁移失败");
    }

    // 存量形状：策略没有新开关，分组只有单值 pinned_message_id。
    sqlx::query(
        "INSERT INTO channels (id, name, receive_id, receive_id_type, kind, enabled, source, \
         created_at, updated_at) VALUES ('c1', '群', 'oc_1', 'chat_id', 'group', 1, \
         'manual', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO notification_policies (channel_id, card_template_id, pin_unresolved, \
         repeat_policy, resolved_policy, group_update_policy, label_matchers, \
         aggregation_window_secs, min_send_interval_secs, max_alerts_per_message, updated_at) \
         VALUES ('c1', NULL, 1, 'suppress', 'send', 'update_previous', '[]', 0, 0, 20, \
         '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("存量形状的策略写入应成功");
    sqlx::query(
        "INSERT INTO alert_groups (id, hook_id, channel_id, group_key, status, first_seen_at, \
         last_seen_at, pinned_message_id) VALUES ('g1', 'h1', 'c1', 'k1', 'firing', \
         '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'om_old')",
    )
    .execute(&pool)
    .await
    .expect("存量形状的分组写入应成功");

    sqlx::raw_sql(MIGRATION_0006)
        .execute(&pool)
        .await
        .expect("0006 迁移失败");

    // 策略新开关默认值：unpin_on_resolve 开、pin_resolved 关。
    let (unpin, pin_resolved): (i64, i64) = {
        let row = sqlx::query(
            "SELECT unpin_on_resolve, pin_resolved FROM notification_policies WHERE channel_id='c1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        (row.get("unpin_on_resolve"), row.get("pin_resolved"))
    };
    assert_eq!(unpin, 1, "unpin_on_resolve 默认必须开启");
    assert_eq!(pin_resolved, 0, "pin_resolved 默认必须关闭");

    // 分组列已升级：新列在、旧列没了。
    assert!(column_exists(&pool, "alert_groups", "pinned_messages").await);
    assert!(
        !column_exists(&pool, "alert_groups", "pinned_message_id").await,
        "0006 之后旧的单值列必须被删除"
    );

    // 旧 Pin 迁移成一条 unresolved 记录。
    let raw: String = sqlx::query_scalar("SELECT pinned_messages FROM alert_groups WHERE id='g1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).expect("pinned_messages 必须是合法 JSON");
    assert_eq!(parsed[0]["message_id"], "om_old");
    assert_eq!(parsed[0]["kind"], "unresolved");
}
