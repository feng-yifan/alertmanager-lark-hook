//! `HookRepository` 的 SQLite 实现。

use alkh_domain::error::PortResult;
use alkh_domain::hook::Hook;
use alkh_domain::ids::{ChannelId, HookId};
use alkh_domain::ports::HookRepository;
use alkh_domain::settings::EncryptedSecret;
use alkh_domain::Timestamp;
use async_trait::async_trait;
use sqlx::sqlite::SqliteRow;
use sqlx::SqlitePool;

use super::common::{self, b, ts, ts_opt, RowExt};

/// 把 `token_nonce` / `token_ciphertext` 两列还原成密文；任一为空都视为「不可回显」。
fn token_enc(row: &SqliteRow) -> PortResult<Option<EncryptedSecret>> {
    let nonce = row.opt_str_col("token_nonce")?;
    let ciphertext = row.opt_str_col("token_ciphertext")?;
    Ok(match (nonce, ciphertext) {
        (Some(nonce), Some(ciphertext)) => Some(EncryptedSecret { nonce, ciphertext }),
        _ => None,
    })
}

fn row_to_hook(row: &SqliteRow) -> PortResult<Hook> {
    Ok(Hook {
        id: HookId::from_string(row.str_col("id")?),
        channel_id: ChannelId::from_string(row.str_col("channel_id")?),
        name: row.str_col("name")?,
        token_hash: row.str_col("token_hash")?,
        token_prefix: row.str_col("token_prefix")?,
        token_enc: token_enc(row)?,
        enabled: row.bool_col("enabled")?,
        last_called_at: row.opt_ts_col("last_called_at")?,
        call_count: row.u64_col("call_count")?,
        created_at: row.ts_col("created_at")?,
        rotated_at: row.opt_ts_col("rotated_at")?,
    })
}

/// Hook 仓储。
#[derive(Debug, Clone)]
pub struct SqliteHookRepository {
    pool: SqlitePool,
}

impl SqliteHookRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl HookRepository for SqliteHookRepository {
    async fn list(&self, channel_id: Option<&ChannelId>) -> PortResult<Vec<Hook>> {
        let rows = match channel_id {
            Some(id) => {
                sqlx::query("SELECT * FROM hooks WHERE channel_id = ? ORDER BY created_at ASC")
                    .bind(id.as_str())
                    .fetch_all(&self.pool)
                    .await
            }
            None => {
                sqlx::query("SELECT * FROM hooks ORDER BY created_at ASC")
                    .fetch_all(&self.pool)
                    .await
            }
        }
        .map_err(common::map_sql)?;
        rows.iter().map(row_to_hook).collect()
    }

    async fn get(&self, id: &HookId) -> PortResult<Option<Hook>> {
        let row = sqlx::query("SELECT * FROM hooks WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_hook).transpose()
    }

    async fn find_by_token_hash(&self, token_hash: &str) -> PortResult<Option<Hook>> {
        let row = sqlx::query("SELECT * FROM hooks WHERE token_hash = ?")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_hook).transpose()
    }

    async fn insert(&self, hook: &Hook) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO hooks \
             (id, channel_id, name, token_hash, token_prefix, token_nonce, token_ciphertext, \
              enabled, last_called_at, call_count, created_at, rotated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(hook.id.as_str())
        .bind(hook.channel_id.as_str())
        .bind(&hook.name)
        .bind(&hook.token_hash)
        .bind(&hook.token_prefix)
        .bind(hook.token_enc.as_ref().map(|e| e.nonce.as_str()))
        .bind(hook.token_enc.as_ref().map(|e| e.ciphertext.as_str()))
        .bind(b(hook.enabled))
        .bind(ts_opt(hook.last_called_at))
        .bind(hook.call_count as i64)
        .bind(ts(hook.created_at))
        .bind(ts_opt(hook.rotated_at))
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn update(&self, hook: &Hook) -> PortResult<()> {
        sqlx::query(
            "UPDATE hooks SET channel_id = ?, name = ?, token_hash = ?, token_prefix = ?, \
             token_nonce = ?, token_ciphertext = ?, \
             enabled = ?, last_called_at = ?, call_count = ?, rotated_at = ? WHERE id = ?",
        )
        .bind(hook.channel_id.as_str())
        .bind(&hook.name)
        .bind(&hook.token_hash)
        .bind(&hook.token_prefix)
        .bind(hook.token_enc.as_ref().map(|e| e.nonce.as_str()))
        .bind(hook.token_enc.as_ref().map(|e| e.ciphertext.as_str()))
        .bind(b(hook.enabled))
        .bind(ts_opt(hook.last_called_at))
        .bind(hook.call_count as i64)
        .bind(ts_opt(hook.rotated_at))
        .bind(hook.id.as_str())
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn delete(&self, id: &HookId) -> PortResult<()> {
        sqlx::query("DELETE FROM hooks WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(())
    }

    async fn record_call(&self, id: &HookId, at: Timestamp) -> PortResult<()> {
        sqlx::query(
            "UPDATE hooks SET call_count = call_count + 1, last_called_at = ? WHERE id = ?",
        )
        .bind(ts(at))
        .bind(id.as_str())
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn count(&self) -> PortResult<(u64, u64)> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM hooks")
            .fetch_one(&self.pool)
            .await
            .map_err(common::map_sql)?;
        let enabled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM hooks WHERE enabled = 1")
            .fetch_one(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok((total.max(0) as u64, enabled.max(0) as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::super::channels::SqliteChannelRepository;
    use super::*;
    use alkh_domain::channel::Channel;
    use alkh_domain::ports::ChannelRepository;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    /// 插入一个渠道作为外键父行，返回其 ID。
    async fn seed_channel(pool: &SqlitePool) -> ChannelId {
        let repo = SqliteChannelRepository::new(pool.clone());
        let c = Channel::from_chat("oc_1", "ops", None, false);
        repo.upsert(&c).await.unwrap();
        c.id
    }

    fn sample(channel_id: ChannelId, name: &str) -> (Hook, String) {
        Hook::issue(channel_id, name)
    }

    #[tokio::test]
    async fn insert_get_and_token_lookup() {
        let pool = pool().await;
        let channel_id = seed_channel(&pool).await;
        let repo = SqliteHookRepository::new(pool);

        let (mut hook, token) = sample(channel_id.clone(), "receiver-a");
        hook.last_called_at = Some(hook.created_at);
        hook.call_count = 7;
        repo.insert(&hook).await.unwrap();

        let got = repo.get(&hook.id).await.unwrap().unwrap();
        assert_eq!(got.name, "receiver-a");
        assert_eq!(got.channel_id, channel_id);
        assert_eq!(got.call_count, 7);
        assert_eq!(got.token_hash, hook.token_hash);
        assert!(got.last_called_at.is_some());

        let by_token = repo
            .find_by_token_hash(&hook.token_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_token.id, hook.id);
        assert!(by_token.matches_token(&token));
    }

    #[tokio::test]
    async fn record_call_increments_counter() {
        let pool = pool().await;
        let channel_id = seed_channel(&pool).await;
        let repo = SqliteHookRepository::new(pool);

        let (hook, _) = sample(channel_id, "receiver-a");
        repo.insert(&hook).await.unwrap();
        assert!(repo
            .get(&hook.id)
            .await
            .unwrap()
            .unwrap()
            .last_called_at
            .is_none());

        let at = alkh_domain::now();
        repo.record_call(&hook.id, at).await.unwrap();
        repo.record_call(&hook.id, at).await.unwrap();

        let got = repo.get(&hook.id).await.unwrap().unwrap();
        assert_eq!(got.call_count, 2);
        assert_eq!(got.last_called_at, Some(at));
    }

    #[tokio::test]
    async fn encrypted_token_roundtrips_and_is_cleared_by_rotation() {
        let pool = pool().await;
        let channel_id = seed_channel(&pool).await;
        let repo = SqliteHookRepository::new(pool);

        let (mut hook, _) = sample(channel_id, "receiver-a");
        let enc = EncryptedSecret {
            nonce: "bm9uY2U=".into(),
            ciphertext: "Y2lwaGVydGV4dA==".into(),
        };
        hook.set_encrypted_token(enc.clone());
        repo.insert(&hook).await.unwrap();

        let got = repo.get(&hook.id).await.unwrap().unwrap();
        assert_eq!(
            got.token_enc,
            Some(enc),
            "密文必须能原样读回，供后台回显地址"
        );

        // 轮换会清空密文；旧密文绝不能继续被当成新地址展示。
        let _ = hook.rotate();
        repo.update(&hook).await.unwrap();
        let got = repo.get(&hook.id).await.unwrap().unwrap();
        assert!(got.token_enc.is_none());
    }

    #[tokio::test]
    async fn update_list_count_and_delete() {
        let pool = pool().await;
        let channel_id = seed_channel(&pool).await;
        let repo = SqliteHookRepository::new(pool);

        let (mut hook, _) = sample(channel_id.clone(), "a");
        repo.insert(&hook).await.unwrap();
        let (other, _) = sample(channel_id.clone(), "b");
        repo.insert(&other).await.unwrap();

        hook.name = "a-renamed".into();
        hook.enabled = false;
        hook.call_count = 3;
        repo.update(&hook).await.unwrap();

        assert_eq!(repo.list(Some(&channel_id)).await.unwrap().len(), 2);
        assert_eq!(repo.list(None).await.unwrap().len(), 2);
        let got = repo.get(&hook.id).await.unwrap().unwrap();
        assert_eq!(got.name, "a-renamed");
        assert!(!got.enabled);
        assert_eq!(got.call_count, 3);

        let (total, enabled) = repo.count().await.unwrap();
        assert_eq!((total, enabled), (2, 1));

        repo.delete(&other.id).await.unwrap();
        assert_eq!(repo.list(None).await.unwrap().len(), 1);
        assert!(repo.get(&other.id).await.unwrap().is_none());
    }
}
