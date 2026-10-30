//! `SettingsRepository` 的 SQLite 实现（单例行 `id = 1`）。

use alkh_domain::error::PortResult;
use alkh_domain::ports::SettingsRepository;
use alkh_domain::settings::{AppSettings, EncryptedSecret, RetentionPolicy};
use async_trait::async_trait;
use sqlx::sqlite::SqliteRow;
use sqlx::SqlitePool;

use super::common::{self, ts, RowExt};

fn row_to_settings(row: &SqliteRow) -> PortResult<AppSettings> {
    let secret = match (
        row.opt_str_col("lark_app_secret_nonce")?,
        row.opt_str_col("lark_app_secret_ciphertext")?,
    ) {
        (Some(nonce), Some(ciphertext)) => Some(EncryptedSecret { nonce, ciphertext }),
        _ => None,
    };
    let verification_token = match (
        row.opt_str_col("lark_verification_token_nonce")?,
        row.opt_str_col("lark_verification_token_ciphertext")?,
    ) {
        (Some(nonce), Some(ciphertext)) => Some(EncryptedSecret { nonce, ciphertext }),
        _ => None,
    };

    Ok(AppSettings {
        lark_app_id: row.opt_str_col("lark_app_id")?,
        lark_app_secret_enc: secret,
        lark_verification_token_enc: verification_token,
        lark_base_url: row.str_col("lark_base_url")?,
        public_base_url: row.opt_str_col("public_base_url")?,
        timezone: row.str_col("timezone")?,
        retention: RetentionPolicy {
            alert_group_days: row.u32_col("retention_alert_group_days")?,
            delivery_days: row.u32_col("retention_delivery_days")?,
            audit_days: row.u32_col("retention_audit_days")?,
        },
        updated_at: row.ts_col("updated_at")?,
    })
}

/// 应用设置仓储。
#[derive(Debug, Clone)]
pub struct SqliteSettingsRepository {
    pool: SqlitePool,
}

impl SqliteSettingsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SettingsRepository for SqliteSettingsRepository {
    async fn get(&self) -> PortResult<AppSettings> {
        let row = sqlx::query("SELECT * FROM app_settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        match row {
            Some(row) => row_to_settings(&row),
            None => Ok(AppSettings::default()),
        }
    }

    async fn save(&self, settings: &AppSettings) -> PortResult<()> {
        let (nonce, ciphertext) = match &settings.lark_app_secret_enc {
            Some(secret) => (
                Some(secret.nonce.as_str()),
                Some(secret.ciphertext.as_str()),
            ),
            None => (None, None),
        };
        let (verif_nonce, verif_cipher) = match &settings.lark_verification_token_enc {
            Some(secret) => (
                Some(secret.nonce.as_str()),
                Some(secret.ciphertext.as_str()),
            ),
            None => (None, None),
        };
        sqlx::query(
            "INSERT INTO app_settings \
             (id, lark_app_id, lark_app_secret_nonce, lark_app_secret_ciphertext, \
              lark_verification_token_nonce, lark_verification_token_ciphertext, \
              lark_base_url, public_base_url, timezone, retention_alert_group_days, \
              retention_delivery_days, retention_audit_days, updated_at) \
             VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
              lark_app_id = excluded.lark_app_id, \
              lark_app_secret_nonce = excluded.lark_app_secret_nonce, \
              lark_app_secret_ciphertext = excluded.lark_app_secret_ciphertext, \
              lark_verification_token_nonce = excluded.lark_verification_token_nonce, \
              lark_verification_token_ciphertext = excluded.lark_verification_token_ciphertext, \
              lark_base_url = excluded.lark_base_url, \
              public_base_url = excluded.public_base_url, \
              timezone = excluded.timezone, \
              retention_alert_group_days = excluded.retention_alert_group_days, \
              retention_delivery_days = excluded.retention_delivery_days, \
              retention_audit_days = excluded.retention_audit_days, \
              updated_at = excluded.updated_at",
        )
        .bind(settings.lark_app_id.as_deref())
        .bind(nonce)
        .bind(ciphertext)
        .bind(verif_nonce)
        .bind(verif_cipher)
        .bind(&settings.lark_base_url)
        .bind(settings.public_base_url.as_deref())
        .bind(&settings.timezone)
        .bind(i64::from(settings.retention.alert_group_days))
        .bind(i64::from(settings.retention.delivery_days))
        .bind(i64::from(settings.retention.audit_days))
        .bind(ts(settings.updated_at))
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn absent_singleton_returns_default() {
        let repo = SqliteSettingsRepository::new(pool().await);
        let got = repo.get().await.unwrap();
        assert_eq!(got.lark_base_url, "https://open.feishu.cn");
        assert!(got.lark_app_id.is_none());
        assert!(got.lark_app_secret_enc.is_none());
        assert_eq!(got.retention, RetentionPolicy::default());
        assert_eq!(got.timezone, "UTC");
        assert!(!got.is_configured());
    }

    #[tokio::test]
    async fn save_and_load_including_encrypted_secret() {
        let repo = SqliteSettingsRepository::new(pool().await);
        let s = AppSettings {
            lark_app_id: Some("cli_app".into()),
            lark_app_secret_enc: Some(EncryptedSecret {
                nonce: "bm9uY2U=".into(),
                ciphertext: "Y2lwaGVy".into(),
            }),
            public_base_url: Some("https://alkh.example.com".into()),
            timezone: "Asia/Shanghai".into(),
            retention: RetentionPolicy {
                alert_group_days: 7,
                delivery_days: 8,
                audit_days: 9,
            },
            ..Default::default()
        };
        repo.save(&s).await.unwrap();

        let got = repo.get().await.unwrap();
        assert_eq!(got.lark_app_id.as_deref(), Some("cli_app"));
        assert_eq!(
            got.lark_app_secret_enc,
            Some(EncryptedSecret {
                nonce: "bm9uY2U=".into(),
                ciphertext: "Y2lwaGVy".into()
            })
        );
        assert_eq!(
            got.public_base_url.as_deref(),
            Some("https://alkh.example.com")
        );
        assert_eq!(got.timezone, "Asia/Shanghai");
        assert_eq!(
            got.retention,
            RetentionPolicy {
                alert_group_days: 7,
                delivery_days: 8,
                audit_days: 9
            }
        );
        assert!(got.is_configured());
    }

    #[tokio::test]
    async fn save_twice_upserts_and_can_clear_secret() {
        let repo = SqliteSettingsRepository::new(pool().await);
        let mut s = AppSettings {
            lark_app_id: Some("cli_app".into()),
            lark_app_secret_enc: Some(EncryptedSecret {
                nonce: "n".into(),
                ciphertext: "c".into(),
            }),
            ..Default::default()
        };
        repo.save(&s).await.unwrap();

        s.lark_base_url = "https://open.larksuite.com".into();
        s.lark_app_secret_enc = None;
        s.retention.audit_days = 30;
        repo.save(&s).await.unwrap();

        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_settings")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(rows, 1);

        let got = repo.get().await.unwrap();
        assert_eq!(got.lark_base_url, "https://open.larksuite.com");
        assert!(got.lark_app_secret_enc.is_none());
        assert_eq!(got.retention.audit_days, 30);
        assert!(!got.is_configured());
    }
}
