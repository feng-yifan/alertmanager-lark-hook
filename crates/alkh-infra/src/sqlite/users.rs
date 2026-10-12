//! `AdminUserRepository` 与 `SessionRepository` 的 SQLite 实现。

use alkh_domain::auth::{AdminUser, Session};
use alkh_domain::error::PortResult;
use alkh_domain::ids::{AdminUserId, SessionId};
use alkh_domain::ports::{AdminUserRepository, SessionRepository};
use alkh_domain::Timestamp;
use async_trait::async_trait;
use sqlx::sqlite::SqliteRow;
use sqlx::SqlitePool;

use super::common::{self, b, ts, ts_opt, RowExt};

fn row_to_user(row: &SqliteRow) -> PortResult<AdminUser> {
    Ok(AdminUser {
        id: AdminUserId::from_string(row.str_col("id")?),
        username: row.str_col("username")?,
        password_hash: row.str_col("password_hash")?,
        must_change_password: row.bool_col("must_change_password")?,
        created_at: row.ts_col("created_at")?,
        last_login_at: row.opt_ts_col("last_login_at")?,
    })
}

/// 管理后台用户仓储。
#[derive(Debug, Clone)]
pub struct SqliteAdminUserRepository {
    pool: SqlitePool,
}

impl SqliteAdminUserRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AdminUserRepository for SqliteAdminUserRepository {
    async fn get(&self, id: &AdminUserId) -> PortResult<Option<AdminUser>> {
        let row = sqlx::query("SELECT * FROM admin_users WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_user).transpose()
    }

    async fn find_by_username(&self, username: &str) -> PortResult<Option<AdminUser>> {
        let row = sqlx::query("SELECT * FROM admin_users WHERE username = ?")
            .bind(username)
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_user).transpose()
    }

    async fn insert(&self, user: &AdminUser) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO admin_users \
             (id, username, password_hash, must_change_password, created_at, last_login_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user.id.as_str())
        .bind(&user.username)
        .bind(&user.password_hash)
        .bind(b(user.must_change_password))
        .bind(ts(user.created_at))
        .bind(ts_opt(user.last_login_at))
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn update(&self, user: &AdminUser) -> PortResult<()> {
        sqlx::query(
            "UPDATE admin_users SET username = ?, password_hash = ?, must_change_password = ?, \
             last_login_at = ? WHERE id = ?",
        )
        .bind(&user.username)
        .bind(&user.password_hash)
        .bind(b(user.must_change_password))
        .bind(ts_opt(user.last_login_at))
        .bind(user.id.as_str())
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn count(&self) -> PortResult<u64> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_users")
            .fetch_one(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(count.max(0) as u64)
    }
}

fn row_to_session(row: &SqliteRow) -> PortResult<Session> {
    Ok(Session {
        id: SessionId::from_string(row.str_col("id")?),
        user_id: AdminUserId::from_string(row.str_col("user_id")?),
        token_hash: row.str_col("token_hash")?,
        expires_at: row.ts_col("expires_at")?,
        created_at: row.ts_col("created_at")?,
        user_agent: row.opt_str_col("user_agent")?,
        ip: row.opt_str_col("ip")?,
    })
}

/// 登录会话仓储。
#[derive(Debug, Clone)]
pub struct SqliteSessionRepository {
    pool: SqlitePool,
}

impl SqliteSessionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionRepository for SqliteSessionRepository {
    async fn insert(&self, session: &Session) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO sessions \
             (id, user_id, token_hash, expires_at, created_at, user_agent, ip) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session.id.as_str())
        .bind(session.user_id.as_str())
        .bind(&session.token_hash)
        .bind(ts(session.expires_at))
        .bind(ts(session.created_at))
        .bind(session.user_agent.as_deref())
        .bind(session.ip.as_deref())
        .execute(&self.pool)
        .await
        .map_err(common::map_sql)?;
        Ok(())
    }

    async fn find_by_token_hash(&self, token_hash: &str) -> PortResult<Option<Session>> {
        let row = sqlx::query("SELECT * FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(common::map_sql)?;
        row.as_ref().map(row_to_session).transpose()
    }

    async fn delete(&self, id: &SessionId) -> PortResult<()> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(())
    }

    async fn delete_for_user(&self, user_id: &AdminUserId) -> PortResult<()> {
        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(user_id.as_str())
            .execute(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(())
    }

    async fn purge_expired(&self, now: Timestamp) -> PortResult<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
            .bind(ts(now))
            .execute(&self.pool)
            .await
            .map_err(common::map_sql)?;
        Ok(result.rows_affected())
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

    fn sample_user(username: &str) -> AdminUser {
        AdminUser {
            id: AdminUserId::new(),
            username: username.into(),
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
            must_change_password: true,
            created_at: alkh_domain::now(),
            last_login_at: None,
        }
    }

    #[tokio::test]
    async fn admin_user_roundtrip_lookup_and_update() {
        let repo = SqliteAdminUserRepository::new(pool().await);
        let mut user = sample_user("admin");
        repo.insert(&user).await.unwrap();

        let got = repo.get(&user.id).await.unwrap().unwrap();
        assert_eq!(got.username, "admin");
        assert_eq!(got.password_hash, user.password_hash);
        assert!(got.must_change_password);
        assert!(got.last_login_at.is_none());

        let by_name = repo.find_by_username("admin").await.unwrap().unwrap();
        assert_eq!(by_name.id, user.id);
        assert!(repo.find_by_username("nobody").await.unwrap().is_none());
        assert_eq!(repo.count().await.unwrap(), 1);

        user.must_change_password = false;
        user.last_login_at = Some(alkh_domain::now());
        repo.update(&user).await.unwrap();
        let got = repo.get(&user.id).await.unwrap().unwrap();
        assert!(!got.must_change_password);
        assert!(got.last_login_at.is_some());
        assert_eq!(got.created_at, user.created_at);
    }

    #[tokio::test]
    async fn session_roundtrip_and_delete() {
        let pool = pool().await;
        let users = SqliteAdminUserRepository::new(pool.clone());
        let repos = SqliteSessionRepository::new(pool);
        let user = sample_user("admin");
        users.insert(&user).await.unwrap();

        let now = alkh_domain::now();
        let (session, token) = Session::issue(
            user.id.clone(),
            now,
            Some("curl/8".into()),
            Some("127.0.0.1".into()),
        );
        repos.insert(&session).await.unwrap();

        let got = repos
            .find_by_token_hash(&session.token_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.id, session.id);
        assert_eq!(got.user_id, user.id);
        assert_eq!(
            got.token_hash,
            alkh_domain::auth::hash_session_token(&token)
        );
        assert_eq!(got.user_agent.as_deref(), Some("curl/8"));
        assert_eq!(got.ip.as_deref(), Some("127.0.0.1"));
        assert!(got.is_valid_at(now));

        repos.delete(&session.id).await.unwrap();
        assert!(repos
            .find_by_token_hash(&session.token_hash)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn delete_for_user_and_purge_expired() {
        let pool = pool().await;
        let users = SqliteAdminUserRepository::new(pool.clone());
        let repos = SqliteSessionRepository::new(pool);
        let user = sample_user("admin");
        users.insert(&user).await.unwrap();
        let other = sample_user("other");
        users.insert(&other).await.unwrap();

        let now = alkh_domain::now();
        for (owner, offset) in [(&user.id, -1i64), (&user.id, 1), (&other.id, 1)] {
            let mut s = Session::issue(owner.clone(), now, None, None).0;
            s.expires_at = now + chrono::Duration::hours(offset);
            repos.insert(&s).await.unwrap();
        }

        // 过期的 1 条被清理（另一个用户的过期会话不会被 user 维度删除）。
        assert_eq!(repos.purge_expired(now).await.unwrap(), 1);

        repos.delete_for_user(&user.id).await.unwrap();
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(&repos.pool)
            .await
            .unwrap();
        assert_eq!(remaining, 1);
    }
}
