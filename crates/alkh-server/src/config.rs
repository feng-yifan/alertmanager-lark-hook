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
            public_base_url: env_opt("ALKH_PUBLIC_BASE_URL"),
            lark_base_url: env_str("ALKH_LARK_BASE_URL", "https://open.feishu.cn"),
            log_level: env_str("ALKH_LOG_LEVEL", "info"),
            log_json: env_bool("ALKH_LOG_JSON", false),
            static_dir: env_opt("ALKH_STATIC_DIR").map(PathBuf::from),
            cookie_secure: env_bool("ALKH_COOKIE_SECURE", false),
            worker_tick_ms: env_u64("ALKH_WORKER_TICK_MS", 200).clamp(50, 5000),
            retry_interval_secs: env_u64("ALKH_RETRY_INTERVAL_SECS", 30).max(5),
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
