//! 管理后台鉴权用例：引导管理员、登录、会话、改密。

use alkh_domain::audit::{action as audit_action, AuditActor, AuditLog};
use alkh_domain::auth::{AdminUser, Session};
use alkh_domain::ids::AdminUserId;
use serde_json::json;

use crate::deps::Deps;
use crate::error::{AppError, AppResult};

/// 管理员引导结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapOutcome {
    /// 已创建管理员。`generated_password` 为随机生成的初始密码，需要打印一次。
    Created { generated_password: Option<String> },
    /// 已存在管理员，未做任何变更。
    AlreadyExists,
}

pub struct AuthService {
    deps: Deps,
}

impl AuthService {
    pub fn new(deps: Deps) -> Self {
        Self { deps }
    }

    /// 首次启动时创建管理员。
    ///
    /// - 若已存在管理员：不做任何事，返回 [`BootstrapOutcome::AlreadyExists`]。
    /// - 否则创建；`password` 为 `None` 时随机生成，并通过返回值交回调用方打印到日志。
    pub async fn ensure_bootstrap_admin(
        &self,
        username: &str,
        password: Option<String>,
    ) -> AppResult<BootstrapOutcome> {
        if self.deps.users.count().await? > 0 {
            return Ok(BootstrapOutcome::AlreadyExists);
        }
        AdminUser::validate_username(username)?;
        let (generated, password) = match password {
            Some(p) if !p.trim().is_empty() => (false, p),
            _ => (true, random_password()),
        };
        AdminUser::validate_password(&password)?;

        let user = AdminUser {
            id: AdminUserId::new(),
            username: username.trim().to_string(),
            password_hash: self.deps.hasher.hash(&password)?,
            must_change_password: generated,
            created_at: self.deps.now(),
            last_login_at: None,
        };
        self.deps.users.insert(&user).await?;

        let log = AuditLog::ok(
            AuditActor::system("bootstrap"),
            "admin.created",
            self.deps.now(),
        )
        .target(user.username.clone())
        .detail(json!({"generated_password": generated}));
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, "审计写入失败");
        }
        Ok(BootstrapOutcome::Created {
            generated_password: generated.then_some(password),
        })
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
        user_agent: Option<String>,
        ip: Option<String>,
    ) -> AppResult<(AdminUser, String)> {
        let user = self
            .deps
            .users
            .find_by_username(username.trim())
            .await?
            .ok_or(AppError::Unauthenticated)?;

        if !self.deps.hasher.verify(password, &user.password_hash)? {
            let log = AuditLog::new(
                AuditActor::admin(username),
                audit_action::AUTH_LOGIN,
                alkh_domain::audit::Outcome::failed("密码错误"),
                self.deps.now(),
            );
            let _ = self.deps.audit.append(&log).await;
            return Err(AppError::Unauthenticated);
        }

        let now = self.deps.now();
        let (session, token) = Session::issue(user.id.clone(), now, user_agent, ip);
        self.deps.sessions.insert(&session).await?;
        self.deps.sessions.purge_expired(now).await?;

        let mut updated = user.clone();
        updated.last_login_at = Some(now);
        self.deps.users.update(&updated).await?;

        let log = AuditLog::ok(
            AuditActor::admin(updated.username.clone()),
            audit_action::AUTH_LOGIN,
            now,
        )
        .target(updated.username.clone());
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, "审计写入失败");
        }
        Ok((updated, token))
    }

    pub async fn logout(&self, token: &str, actor: &AuditActor) -> AppResult<()> {
        let hash = alkh_domain::auth::hash_session_token(token);
        if let Some(session) = self.deps.sessions.find_by_token_hash(&hash).await? {
            self.deps.sessions.delete(&session.id).await?;
        }
        let log = AuditLog::ok(actor.clone(), audit_action::AUTH_LOGOUT, self.deps.now());
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, "审计写入失败");
        }
        Ok(())
    }

    /// 解析会话 token → 管理员。
    pub async fn session_user(&self, token: &str) -> AppResult<AdminUser> {
        let hash = alkh_domain::auth::hash_session_token(token);
        let session = self
            .deps
            .sessions
            .find_by_token_hash(&hash)
            .await?
            .ok_or(AppError::Unauthenticated)?;

        let now = self.deps.now();
        if !session.is_valid_at(now) {
            let _ = self.deps.sessions.delete(&session.id).await;
            return Err(AppError::Unauthenticated);
        }
        self.deps
            .users
            .get(&session.user_id)
            .await?
            .ok_or(AppError::Unauthenticated)
    }

    pub async fn change_password(
        &self,
        user_id: &AdminUserId,
        current: &str,
        new: &str,
        actor: &AuditActor,
    ) -> AppResult<()> {
        let mut user = self
            .deps
            .users
            .get(user_id)
            .await?
            .ok_or(AppError::Unauthenticated)?;
        if !self.deps.hasher.verify(current, &user.password_hash)? {
            return Err(AppError::Forbidden("当前密码不正确".into()));
        }
        if current == new {
            return Err(AppError::validation("新密码不能与当前密码相同"));
        }
        AdminUser::validate_password(new)?;

        user.password_hash = self.deps.hasher.hash(new)?;
        user.must_change_password = false;
        self.deps.users.update(&user).await?;

        // 改密后使其它会话失效
        self.deps.sessions.delete_for_user(&user.id).await?;

        let log = AuditLog::ok(
            actor.clone(),
            audit_action::AUTH_PASSWORD_CHANGED,
            self.deps.now(),
        )
        .target(user.username.clone());
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, "审计写入失败");
        }
        Ok(())
    }
}

fn random_password() -> String {
    use rand::distributions::Alphanumeric;
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..20)
        .map(|_| char::from(rng.sample(Alphanumeric)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_password_is_strong_enough() {
        let p = random_password();
        assert_eq!(p.len(), 20);
        assert!(AdminUser::validate_password(&p).is_ok());
        assert_ne!(p, random_password());
    }
}
