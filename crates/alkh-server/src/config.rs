//! 环境变量配置（12-factor）。

use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub http_addr: SocketAddr,
    pub data_dir: PathBuf,
    pub database_url: String,
    pub db_max_connections: u32,
    pub master_key: Option<String>,
    pub admin_username: String,
    pub admin_password: Option<String>,
    pub public_base_url: Option<String>,
    pub lark_base_url: String,
    pub log_level: String,
    pub log_json: bool,
    pub static_dir: Option<PathBuf>,
    /// 会话 Cookie 是否加 `Secure`（HTTPS 部署时应为 true）。
    pub cookie_secure: bool,
    /// 聚合冲刷循环的 tick。
    pub worker_tick_ms: u64,
    /// 失败投递重试间隔。
    pub retry_interval_secs: u64,
    /// 数据保留清理间隔。
    pub retention_interval_secs: u64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let data_dir = PathBuf::from(env_str("ALKH_DATA_DIR", "/data"));
        let database_url = match env_opt("ALKH_DATABASE_URL") {
            Some(url) => url,
            None => format!("sqlite://{}/alkh.db", data_dir.display()),
        };
        let public_base_url = env_opt("ALKH_PUBLIC_BASE_URL");
        // 显式设置优先；未设置时按对外基址的协议推断，默认 HTTPS 部署开启 Secure。
        let cookie_secure = resolve_cookie_secure(
            public_base_url.as_deref(),
            env_opt("ALKH_COOKIE_SECURE").as_deref(),
        );

        Ok(Self {
            http_addr: env_str("ALKH_HTTP_ADDR", "0.0.0.0:8080")
                .parse()
                .map_err(|e| anyhow::anyhow!("ALKH_HTTP_ADDR 非法: {e}"))?,
            data_dir,
            database_url,
            db_max_connections: env_u64("ALKH_DB_MAX_CONNECTIONS", 8).max(1) as u32,
            master_key: env_opt("ALKH_MASTER_KEY"),
            admin_username: env_str("ALKH_ADMIN_USERNAME", "admin"),
            admin_password: env_opt("ALKH_ADMIN_PASSWORD"),
            public_base_url,
            lark_base_url: env_str("ALKH_LARK_BASE_URL", "https://open.feishu.cn"),
            log_level: env_str("ALKH_LOG_LEVEL", "info"),
            log_json: env_bool("ALKH_LOG_JSON", false),
            static_dir: env_opt("ALKH_STATIC_DIR").map(PathBuf::from),
            cookie_secure,
            worker_tick_ms: env_u64("ALKH_WORKER_TICK_MS", 200).clamp(50, 5000),
            // 默认 5s：R14 的第一档退避是 1s，tick 太粗会让短档位实际被拉平到 tick 长度。
            // 一次 tick 只是一条走索引的查询，开销可忽略。
            retry_interval_secs: env_u64("ALKH_RETRY_INTERVAL_SECS", 5).max(1),
            retention_interval_secs: env_u64("ALKH_RETENTION_INTERVAL_SECS", 3600).max(60),
        })
    }

    /// 主密钥文件路径（未提供 `ALKH_MASTER_KEY` 时自动生成）。
    pub fn master_key_path(&self) -> PathBuf {
        self.data_dir.join("master.key")
    }

    pub fn session_cookie_name() -> &'static str {
        "alkh_session"
    }
}

/// 会话 Cookie 是否加 `Secure`。
///
/// 显式设置 `ALKH_COOKIE_SECURE` 时以它为准；**未设置时按 `ALKH_PUBLIC_BASE_URL`
/// 的协议推断**。这个开关过去默认 `false`，而项目推荐的部署形态正是「公网 + HTTPS
/// 反代」（README 与 k8s ConfigMap 都这么建议）—— 运维漏配时，12 小时有效的管理员
/// 会话 Cookie 会在任意 `http://` 请求中明文外发，可被中间人直接劫持。
///
/// 无 `ALKH_PUBLIC_BASE_URL` 时仍取 `false`：本地开发与内网纯 HTTP 场景不能被误伤。
fn resolve_cookie_secure(public_base_url: Option<&str>, explicit: Option<&str>) -> bool {
    if let Some(v) = explicit {
        return matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        );
    }
    public_base_url
        .map(|u| u.trim().to_ascii_lowercase().starts_with("https://"))
        .unwrap_or(false)
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_str(key: &str, default: &str) -> String {
    env_opt(key).unwrap_or_else(|| default.to_string())
}

fn env_u64(key: &str, default: u64) -> u64 {
    env_opt(key)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    match env_opt(key).map(|v| v.to_ascii_lowercase()) {
        Some(v) => matches!(v.as_str(), "1" | "true" | "yes" | "on"),
        None => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 会话 Cookie 的 `Secure` 必须默认跟随对外协议的推断：
    /// 漏配是真实踩过的坑（HTTPS 反代 + 默认 false ⇒ 会话 Cookie 明文外发）。
    #[test]
    fn cookie_secure_follows_public_base_url_unless_explicit() {
        // 未显式设置：按对外基址协议推断
        assert!(resolve_cookie_secure(
            Some("https://alkh.example.com"),
            None
        ));
        assert!(resolve_cookie_secure(Some("HTTPS://A.example.com"), None));
        assert!(!resolve_cookie_secure(Some("http://10.0.0.1:8080"), None));
        // 没有对外基址时保持 false，不误伤本地/内网 HTTP
        assert!(!resolve_cookie_secure(None, None));

        // 显式设置优先（两个方向都要能被覆盖）
        assert!(!resolve_cookie_secure(
            Some("https://a.example.com"),
            Some("false")
        ));
        assert!(resolve_cookie_secure(
            Some("http://a.example.com"),
            Some("true")
        ));
        assert!(resolve_cookie_secure(None, Some(" ON ")));
        // 显式给了无法识别的值 ⇒ 视为 false（与 env_bool 一致）
        assert!(!resolve_cookie_secure(
            Some("https://a.example.com"),
            Some("enable")
        ));
    }

    #[test]
    fn default_database_url_is_under_data_dir() {
        let dir = std::env::temp_dir();
        let url = format!("sqlite://{}/alkh.db", dir.display());
        assert!(url.starts_with("sqlite:///"));
        assert!(url.ends_with("/alkh.db"));
    }

    #[test]
    fn master_key_path_is_in_data_dir() {
        let cfg = Config {
            http_addr: "0.0.0.0:8080".parse().unwrap(),
            data_dir: PathBuf::from("/data"),
            database_url: "sqlite:///data/alkh.db".into(),
            db_max_connections: 8,
            master_key: None,
            admin_username: "admin".into(),
            admin_password: None,
            public_base_url: None,
            lark_base_url: "https://open.feishu.cn".into(),
            log_level: "info".into(),
            log_json: false,
            static_dir: None,
            cookie_secure: false,
            worker_tick_ms: 200,
            retry_interval_secs: 30,
            retention_interval_secs: 3600,
        };
        assert_eq!(cfg.master_key_path(), PathBuf::from("/data/master.key"));
    }
}
