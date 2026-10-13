//! `CardTemplateRepository` 的 SQLite 实现。

use alkh_domain::error::PortResult;
use alkh_domain::ids::CardTemplateId;
use alkh_domain::ports::CardTemplateRepository;
use alkh_domain::template::{CardMessageSpec, CardTemplate};
use async_trait::async_trait;
use sqlx::sqlite::SqliteRow;
use sqlx::SqlitePool;

use super::common::{self, b, to_json, ts, RowExt};

fn row_to_template(row: &SqliteRow) -> PortResult<CardTemplate> {
    let card_message: Option<CardMessageSpec> = row.json_opt_col::<CardMessageSpec>("card_message");
    Ok(CardTemplate {
        id: CardTemplateId::from_string(row.str_col("id")?),
        name: row.str_col("name")?,
        card_message: card_message.ok_or_else(|| {
            alkh_domain::error::PortError::Internal("card_templates.card_message 为空".into())
        })?,
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
    async fn list(&self) -> PortResult<Vec<CardTemplate>> {
        let rows = sqlx::query("SELECT * FROM card_templates ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await
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
             (id, name, card_message, is_default, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(template.id.as_str())
        .bind(&template.name)
        .bind(to_json(&template.card_message))
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
            "UPDATE card_templates SET name = ?, card_message = ?, is_default = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&template.name)
        .bind(to_json(&template.card_message))
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

    async fn find_default(&self) -> PortResult<Option<CardTemplate>> {
        let row = sqlx::query(
            "SELECT * FROM card_templates WHERE is_default = 1 ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(common::map_sql)?;
        row.as_ref().map(row_to_template).transpose()
    }

    async fn clear_default(&self) -> PortResult<()> {
        sqlx::query("UPDATE card_templates SET is_default = 0")
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
    async fn insert_get_roundtrip() {
        let repo = SqliteCardTemplateRepository::new(pool().await);

        let cm = card_message("卡片");
        repo.insert(&cm).await.unwrap();
        let got = repo.get(&cm.id).await.unwrap().unwrap();
        assert_eq!(got.name, "卡片");
        assert_eq!(got.card_message.template_id, None);
        assert!(!got.is_default);
    }

    #[tokio::test]
    async fn default_is_unique() {
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
        assert_eq!(repo.find_default().await.unwrap().unwrap().id, a.id);

        repo.clear_default().await.unwrap();
        assert!(repo.find_default().await.unwrap().is_none());

        // 两个默认标记同时存在时，返回创建时间最新的一个。
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
        assert_eq!(repo.find_default().await.unwrap().unwrap().id, b.id);
    }

    #[tokio::test]
    async fn list_and_delete() {
        let repo = SqliteCardTemplateRepository::new(pool().await);
        repo.insert(&card_message("a")).await.unwrap();
        repo.insert(&card_message("b")).await.unwrap();

        assert_eq!(repo.list().await.unwrap().len(), 2);

        let all = repo.list().await.unwrap();
        repo.delete(&all[0].id).await.unwrap();
        assert_eq!(repo.list().await.unwrap().len(), 1);
    }
}
