//! 迁移 `0002_remove_app_feed_card` 的回归测试。
//!
//! 这是**破坏性**迁移（删模板行 + DROP COLUMN），而它要跑在已有数据的库上：
//! 一旦数据处置写错，升级后要么策略指向不存在的模板、要么直接迁移失败起不来。
//! 这里用 0001 手工建表 → 灌入 app_feed 数据 → 执行 0002 → 断言结果。
//!
//! 不用 `sqlx::migrate!` 是因为它会把 0001 与 0002 连着跑完，
//! 没法在中间插入「升级前」的数据。

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

const MIGRATION_0001: &str = include_str!("../../../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../../../migrations/0002_remove_app_feed_card.sql");

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
async fn upgrade_from_a_database_with_app_feed_data() {
    let pool = pool_with_0001().await;

    // --- 升级前的数据：一个 app_feed 模板 + 引用它的策略 + 一条 app_feed 投递 ---
    sqlx::query(
        "INSERT INTO channels (id, name, receive_id, receive_id_type, kind, enabled, source, \
         created_at, updated_at) VALUES ('c1', '单人', 'ou_1', 'open_id', 'p2p_user', 1, \
         'manual', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO card_templates (id, kind, name, card_message, app_feed_spec, is_default, \
         created_at, updated_at) VALUES ('t_feed', 'app_feed_card', '默认应用消息流卡片', NULL, \
         '{\"title\":\"x\"}', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO card_templates (id, kind, name, card_message, app_feed_spec, is_default, \
         created_at, updated_at) VALUES ('t_card', 'card_message', '默认告警卡片', \
         '{\"source\":\"builder_json\"}', NULL, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO alert_groups (id, hook_id, channel_id, group_key, status, first_seen_at, \
         last_seen_at) VALUES ('g1', 'h1', 'c1', 'k1', 'firing', '2026-01-01T00:00:00Z', \
         '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO notification_policies (channel_id, message_type, card_template_id, \
         pin_unresolved, repeat_policy, resolved_policy, group_update_policy, label_matchers, \
         aggregation_window_secs, min_send_interval_secs, max_alerts_per_message, updated_at) \
         VALUES ('c1', 'app_feed_card', 't_feed', 1, 'suppress', 'send', 'update_previous', \
         '[]', 0, 0, 20, '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO deliveries (id, alert_group_id, channel_id, kind, message_type, state, \
         app_feed_biz_id, card_sequence, rendered_digest, attempt, created_at, updated_at) \
         VALUES ('d1', 'g1', 'c1', 'firing_created', 'app_feed_card', 'active', 'alkh-g1', 0, \
         'digest', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    // --- 执行 0002 ---
    sqlx::raw_sql(MIGRATION_0002)
        .execute(&pool)
        .await
        .expect("0002 迁移失败");

    // 1. app_feed 模板行被删除，卡片模板保留
    let names: Vec<String> = sqlx::query("SELECT name FROM card_templates ORDER BY name")
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<String, _>("name"))
        .collect();
    assert_eq!(names, vec!["默认告警卡片".to_string()]);

    // 2. 引用被删模板的策略被置空（否则运行时是悬空引用）
    let row =
        sqlx::query("SELECT card_template_id FROM notification_policies WHERE channel_id='c1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        row.get::<Option<String>, _>("card_template_id").is_none(),
        "引用被删模板的策略必须回退到默认模板（card_template_id = NULL）"
    );

    // 3. 列被删除
    for (table, column) in [
        ("card_templates", "kind"),
        ("card_templates", "app_feed_spec"),
        ("notification_policies", "message_type"),
        ("notification_policies", "pin_unresolved"),
        ("deliveries", "message_type"),
        ("deliveries", "app_feed_biz_id"),
        ("app_settings", "default_app_feed_link"),
    ] {
        assert!(
            !column_exists(&pool, table, column).await,
            "{table}.{column} 应已被删除"
        );
    }

    // 4. 历史投递行保留（审计数据不能因为删列而丢）
    let deliveries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM deliveries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(deliveries, 1, "历史投递必须保留");

    // 5. 迁移后的表能正常写入（列数与新代码一致）
    sqlx::query(
        "INSERT INTO card_templates (id, name, card_message, is_default, created_at, updated_at) \
         VALUES ('t_new', '新卡片', '{\"source\":\"builder_json\"}', 0, '2026-01-01T00:00:00Z', \
         '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("迁移后应能正常插入卡片模板");
    sqlx::query(
        "INSERT INTO deliveries (id, alert_group_id, channel_id, kind, state, card_sequence, \
         rendered_digest, attempt, created_at, updated_at) VALUES ('d2', 'g1', 'c1', \
         'firing_created', 'pending', 0, 'x', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("迁移后应能正常插入投递");
}
