//! `CardTemplateRepository` 的 SQLite 实现。

use alkh_domain::error::PortResult;
use alkh_domain::ids::CardTemplateId;
use alkh_domain::ports::CardTemplateRepository;
use alkh_domain::template::{AppFeedCardSpec, CardMessageSpec, CardTemplate, TemplateKind};
use async_trait::async_trait;
use sqlx::sqlite::SqliteRow;
use sqlx::SqlitePool;

use super::common::{self, b, enum_to_str, to_json, ts, RowExt};

fn row_to_template(row: &SqliteRow) -> PortResult<CardTemplate> {
    Ok(CardTemplate {
        id: CardTemplateId::from_string(row.str_col("id")?),
        name: row.str_col("name")?,
        kind: row.enum_col("kind")?,
        card_message: row.json_opt_col::<CardMessageSpec>("card_message"),
        app_feed_spec: row.json_opt_col::<AppFeedCardSpec>("app_feed_spec"),
        is_default: row.bool_col("is_default")?,
        created_at: row.ts_col("created_at")?,
        updated_at: row.ts_col("updated_at")?,
    })
}

/// 卡片模板仓储。
#[derive(Debug, Clone)]
pub struct SqliteCardTemplateRepository {
    pool: SqlitePool,
}

impl SqliteCardTemplateRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CardTemplateRepository for SqliteCardTemplateRepository {
    async fn list(&self, kind: Option<TemplateKind>) -> PortResult<Vec<CardTemplate>> {
        let rows = match kind {
            Some(kind) => {
                sqlx::query("SELECT * FROM card_templates WHERE kind = ? ORDER BY created_at ASC")
                    .bind(enum_to_str(&kind))
                    .fetch_all(&self.pool)
                    .await
            }
            None => {
                sqlx::query("SELECT * FROM card_templates ORDER BY created_at ASC")
                    .fetch_all(&self.pool)
                    .await
            }
        }
        .map_err(common::map_sql)?;
        rows.iter().map(row_to_template).collect()
    }

    async fn get(&self, id: &CardTemplateId) -> PortResult<Option<CardTemplate>> {
        let row = sqlx::query("SELECT * FROM card_templates WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_template).transpose()
    }

    async fn insert(&self, template: &CardTemplate) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO card_templates \
             (id, kind, name, card_message, app_feed_spec, is_default, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(template.id.as_str())
        .bind(enum_to_str(&template.kind))
        .bind(&template.name)
        .bind(template.card_message.as_ref().map(to_json))
        .bind(template.app_feed_spec.as_ref().map(to_json))
        .bind(b(template.is_default))
        .bind(ts(template.created_at))
        .bind(ts(template.updated_at))
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn update(&self, template: &CardTemplate) -> PortResult<()> {
        sqlx::query(
            "UPDATE card_templates SET kind = ?, name = ?, card_message = ?, app_feed_spec = ?, \
             is_default = ?, updated_at = ? WHERE id = ?",
        )
        .bind(enum_to_str(&template.kind))
        .bind(&template.name)
        .bind(template.card_message.as_ref().map(to_json))
        .bind(template.app_feed_spec.as_ref().map(to_json))
        .bind(b(template.is_default))
        .bind(ts(template.updated_at))
        .bind(template.id.as_str())
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn delete(&self, id: &CardTemplateId) -> PortResult<()> {
        sqlx::query("DELETE FROM card_templates WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(())
    }

    async fn find_default(&self, kind: TemplateKind) -> PortResult<Option<CardTemplate>> {
        let row = sqlx::query(
            "SELECT * FROM card_templates WHERE kind = ? AND is_default = 1 \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(enum_to_str(&kind))
        .fetch_optional(&self.pool)
        .await
        .map_err(common::map_sql)?;
        row.as_ref().map(row_to_template).transpose()
    }

    async fn clear_default(&self, kind: TemplateKind) -> PortResult<()> {
        sqlx::query("UPDATE card_templates SET is_default = 0 WHERE kind = ?")
            .bind(enum_to_str(&kind))
            .execute(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn card_message(name: &str) -> CardTemplate {
        CardTemplate::new_card_message(
            name,
            CardMessageSpec::builder_json(json!({"schema": "2.0", "body": {"elements": []}})),
        )
    }

    #[tokio::test]
    async fn insert_get_roundtrip_for_both_kinds() {
        let repo = SqliteCardTemplateRepository::new(pool().await);

        let cm = card_message("卡片");
        repo.insert(&cm).await.unwrap();
        let got = repo.get(&cm.id).await.unwrap().unwrap();
        assert_eq!(got.name, "卡片");
        assert_eq!(got.kind, TemplateKind::CardMessage);
        assert_eq!(got.card_message.as_ref().unwrap().template_id, None);
        assert!(got.app_feed_spec.is_none());
        assert!(!got.is_default);

        let mut feed = CardTemplate::new_app_feed_card("feed", AppFeedCardSpec::default());
        feed.is_default = true;
        repo.insert(&feed).await.unwrap();
        let got = repo.get(&feed.id).await.unwrap().unwrap();
        assert_eq!(got.kind, TemplateKind::AppFeedCard);
        assert!(got.app_feed_spec.is_some());
        assert!(got.is_default);
    }

    #[tokio::test]
    async fn default_is_unique_per_kind() {
        let repo = SqliteCardTemplateRepository::new(pool().await);
        let base = alkh_domain::now();
        let mut a = card_message("a");
        a.created_at = base;
        let mut b = card_message("b");
        b.created_at = base + chrono::Duration::seconds(1);
        repo.insert(&a).await.unwrap();
        repo.insert(&b).await.unwrap();

        repo.update(&CardTemplate {
            is_default: true,
            ..a.clone()
        })
        .await
        .unwrap();
        assert_eq!(
            repo.find_default(TemplateKind::CardMessage)
                .await
                .unwrap()
                .unwrap()
                .id,
            a.id
        );

        repo.clear_default(TemplateKind::CardMessage).await.unwrap();
        assert!(repo
            .find_default(TemplateKind::CardMessage)
            .await
            .unwrap()
            .is_none());

        // 两个同类默认标记同时存在时，返回创建时间最新的一个。
        repo.update(&CardTemplate {
            is_default: true,
            ..a.clone()
        })
        .await
        .unwrap();
        repo.update(&CardTemplate {
            is_default: true,
            ..b.clone()
        })
        .await
        .unwrap();
        assert_eq!(
            repo.find_default(TemplateKind::CardMessage)
                .await
                .unwrap()
                .unwrap()
                .id,
            b.id
        );

        // 其他 kind 不受影响。
        assert!(repo
            .find_default(TemplateKind::AppFeedCard)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn list_by_kind_and_delete() {
        let repo = SqliteCardTemplateRepository::new(pool().await);
        repo.insert(&card_message("a")).await.unwrap();
        repo.insert(&card_message("b")).await.unwrap();
        repo.insert(&CardTemplate::new_app_feed_card(
            "f",
            AppFeedCardSpec::default(),
        ))
        .await
        .unwrap();

        assert_eq!(repo.list(None).await.unwrap().len(), 3);
        assert_eq!(
            repo.list(Some(TemplateKind::CardMessage))
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            repo.list(Some(TemplateKind::AppFeedCard))
                .await
                .unwrap()
                .len(),
            1
        );

        let all = repo.list(None).await.unwrap();
        repo.delete(&all[0].id).await.unwrap();
        assert_eq!(repo.list(None).await.unwrap().len(), 2);
    }
}
