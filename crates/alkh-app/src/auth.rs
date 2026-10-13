//! 管理后台鉴权用例：引导管理员、登录、会话、改密。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

/// 登录失败节流器：按 `(用户名, 来源 IP)` 计数并做指数退避。
///
/// 之所以放在进程内存里：本项目**明确不支持多副本**（嵌入式 SQLite 单写者，
/// 见 README 与 deploy/kubernetes.yaml 的 `replicas: 1` 说明），单实例内存态足够；
/// 重启会清空计数，属可接受的降级（重启本身已需要运维权限）。
#[derive(Debug, Default)]
struct LoginThrottle {
    entries: Mutex<HashMap<String, ThrottleEntry>>,
}

#[derive(Debug)]
struct ThrottleEntry {
    consecutive_failures: u32,
    blocked_until: Option<Instant>,
    lock: Duration,
}

/// 连续失败多少次后开始锁定。
const THROTTLE_FREE_ATTEMPTS: u32 = 5;
const THROTTLE_BASE_LOCK: Duration = Duration::from_secs(30);
const THROTTLE_MAX_LOCK: Duration = Duration::from_secs(15 * 60);

impl LoginThrottle {
    fn key(username: &str, ip: Option<&str>) -> String {
        format!(
            "{}|{}",
            username.trim().to_ascii_lowercase(),
            ip.unwrap_or("-")
        )
    }

    /// 若该组合正处于锁定期，返回建议等待的秒数。
    fn retry_after(&self, key: &str, now: Instant) -> Option<u64> {
        let guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let entry = guard.get(key)?;
        let until = entry.blocked_until?;
        (until > now).then(|| (until - now).as_secs().max(1))
    }

    fn record_failure(&self, key: &str, now: Instant) {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let entry = guard.entry(key.to_string()).or_insert(ThrottleEntry {
            consecutive_failures: 0,
            blocked_until: None,
            lock: THROTTLE_BASE_LOCK,
        });
        entry.consecutive_failures += 1;
        if entry.consecutive_failures >= THROTTLE_FREE_ATTEMPTS {
            let lock = entry.lock.min(THROTTLE_MAX_LOCK);
            entry.blocked_until = Some(now + lock);
            // 指数退避，下次失败锁更久。
            entry.lock = (lock * 2).min(THROTTLE_MAX_LOCK);
        }
    }

    fn record_success(&self, key: &str) {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(key);
    }

    /// 顺带清理过期条目，避免长期运行后 map 无限增长。
    fn prune(&self, now: Instant) {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|_, e| e.blocked_until.map(|u| u > now).unwrap_or(true));
    }
}

pub struct AuthService {
    deps: Deps,
    throttle: LoginThrottle,
}

impl AuthService {
    pub fn new(deps: Deps) -> Self {
        Self {
            deps,
            throttle: LoginThrottle::default(),
        }
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
        let throttle_key = LoginThrottle::key(username, ip.as_deref());
        let monotonic = Instant::now();
        if let Some(retry_after) = self.throttle.retry_after(&throttle_key, monotonic) {
            let log = AuditLog::new(
                AuditActor::admin(username),
                audit_action::AUTH_LOGIN,
                alkh_domain::audit::Outcome::failed("登录失败次数过多，已临时拒绝"),
                self.deps.now(),
            );
            let _ = self.deps.audit.append(&log).await;
            return Err(AppError::rate_limited(retry_after));
        }
        self.throttle.prune(monotonic);

        let user = match self.deps.users.find_by_username(username.trim()).await? {
            Some(user) => user,
            None => {
                // 用户不存在时**也要**付出一次 Argon2 校验的代价：否则响应时间会
                // 明显短于「密码错误」，可被用来枚举用户名。
                if let Some(dummy) = self.dummy_password_hash() {
                    let _ = self.deps.hasher.verify(password, &dummy);
                }
                self.throttle.record_failure(&throttle_key, monotonic);
                let log = AuditLog::new(
                    AuditActor::admin(username),
                    audit_action::AUTH_LOGIN,
                    alkh_domain::audit::Outcome::failed("用户名不存在"),
                    self.deps.now(),
                );
                let _ = self.deps.audit.append(&log).await;
                return Err(AppError::Unauthenticated);
            }
        };

        if !self.deps.hasher.verify(password, &user.password_hash)? {
            self.throttle.record_failure(&throttle_key, monotonic);
            let log = AuditLog::new(
                AuditActor::admin(username),
                audit_action::AUTH_LOGIN,
                alkh_domain::audit::Outcome::failed("密码错误"),
                self.deps.now(),
            );
            let _ = self.deps.audit.append(&log).await;
            return Err(AppError::Unauthenticated);
        }
        self.throttle.record_success(&throttle_key);

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

    /// 「用户不存在」分支用的假哈希（进程内只算一次）。
    ///
    /// 目的是让「用户不存在」与「密码错误」两条路径的耗时可比，消除用户名枚举的
    /// 时间侧信道；返回值只为付出代价，never 用于比较结果。
    fn dummy_password_hash(&self) -> Option<String> {
        static DUMMY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        DUMMY
            .get_or_init(|| {
                self.deps
                    .hasher
                    .hash("alkh-timing-equalizer-placeholder")
                    .map_err(|e| {
                        tracing::warn!(error = %e, "生成时间均衡用假哈希失败");
                        e
                    })
                    .ok()
            })
            .clone()
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

    /// 节流：阈值内不锁，达到阈值后锁定并指数退避，成功登录会清零。
    #[test]
    fn login_throttle_locks_after_threshold_and_backs_off() {
        let throttle = LoginThrottle::default();
        let key = LoginThrottle::key("admin", Some("10.0.0.1"));
        let t0 = Instant::now();

        // 阈值 - 1 次失败仍不锁
        for _ in 0..THROTTLE_FREE_ATTEMPTS - 1 {
            throttle.record_failure(&key, t0);
        }
        assert_eq!(throttle.retry_after(&key, t0), None, "未达阈值不应锁定");

        // 第 THROTTLE_FREE_ATTEMPTS 次失败开始锁
        throttle.record_failure(&key, t0);
        let first = throttle.retry_after(&key, t0).expect("达到阈值后应锁定");
        assert!(first >= 1);
        assert!(first <= THROTTLE_BASE_LOCK.as_secs());

        // 锁定期过后仍处于「还差一次就再次锁定」的状态，下一次失败锁更久（指数退避）
        let after_lock = t0 + THROTTLE_BASE_LOCK + Duration::from_secs(1);
        assert_eq!(throttle.retry_after(&key, after_lock), None);
        throttle.record_failure(&key, after_lock);
        let second = throttle.retry_after(&key, after_lock).expect("应再次锁定");
        assert!(second > first, "第二次锁定时长应更长: {second} vs {first}");

        // 成功登录清零
        throttle.record_success(&key);
        assert_eq!(throttle.retry_after(&key, after_lock), None);
    }

    /// 不同用户名 / 不同来源 IP 互不影响，避免一个被爆破就全站登录不了。
    #[test]
    fn login_throttle_is_scoped_per_username_and_ip() {
        let throttle = LoginThrottle::default();
        let victim = LoginThrottle::key("admin", Some("1.1.1.1"));
        for _ in 0..=THROTTLE_FREE_ATTEMPTS {
            throttle.record_failure(&victim, Instant::now());
        }
        assert!(throttle.retry_after(&victim, Instant::now()).is_some());

        for other in [
            LoginThrottle::key("admin", Some("2.2.2.2")),
            LoginThrottle::key("someone-else", Some("1.1.1.1")),
        ] {
            assert_eq!(
                throttle.retry_after(&other, Instant::now()),
                None,
                "不应牵连其他用户名/来源"
            );
        }
    }

    /// 清理不能把仍在锁定期的条目丢掉，否则节流可被大量新 IP 冲掉。
    #[test]
    fn login_throttle_prune_keeps_locked_entries() {
        let throttle = LoginThrottle::default();
        let locked = LoginThrottle::key("admin", Some("1.1.1.1"));
        let idle = LoginThrottle::key("admin", Some("3.3.3.3"));
        let t0 = Instant::now();
        for _ in 0..=THROTTLE_FREE_ATTEMPTS {
            throttle.record_failure(&locked, t0);
        }
        throttle.record_failure(&idle, t0); // 只有一次失败，没锁
        throttle.prune(t0);
        assert!(throttle.retry_after(&locked, t0).is_some());
        assert_eq!(throttle.retry_after(&idle, t0), None);
    }

    #[test]
    fn generated_password_is_strong_enough() {
        let p = random_password();
        assert_eq!(p.len(), 20);
        assert!(AdminUser::validate_password(&p).is_ok());
        assert_ne!(p, random_password());
    }
}
