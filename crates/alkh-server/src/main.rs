//! alertmanager-lark-hook 服务入口。
//!
//! ```text
//! AlertManager ──webhook──▶ POST /api/v1/hooks/alertmanager/{token}
//! 管理后台（内嵌 SPA）      ──▶ /api/v1/**
//! 探针                     ──▶ /healthz /readyz /metrics
//! ```

use std::sync::atomic::Ordering;
use std::sync::Arc;

use alkh_server::{build_router, config::Config, init_telemetry, wiring, workers};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 容器 HEALTHCHECK 用：只做一次 TCP 连通性探测，不初始化日志/数据库。
    if std::env::args().any(|a| a == "--healthcheck") {
        return healthcheck().await;
    }

    let config = Arc::new(Config::from_env()?);
    init_telemetry(&config);

    tracing::info!(
        version = alkh_app::meta::VERSION,
        addr = %config.http_addr,
        data_dir = %config.data_dir.display(),
        "alertmanager-lark-hook 启动中"
    );

    std::fs::create_dir_all(&config.data_dir)
        .map_err(|e| anyhow::anyhow!("无法创建数据目录 {}: {e}", config.data_dir.display()))?;

    let pool = alkh_infra::connect(&config.database_url, config.db_max_connections).await?;
    alkh_infra::migrate(&pool).await?;

    let state = wiring::build_state(config.clone(), pool.clone(), None).await?;

    // 首次启动引导：管理员账号 + 默认卡片模板
    match state
        .auth
        .ensure_bootstrap_admin(&config.admin_username, config.admin_password.clone())
        .await?
    {
        alkh_app::auth::BootstrapOutcome::Created {
            generated_password: Some(password),
        } => {
            tracing::warn!(
                username = %config.admin_username,
                password = %password,
                "已创建管理员账号并随机生成初始密码（仅本次启动打印）。请登录后立即修改，或通过 ALKH_ADMIN_PASSWORD 指定。"
            );
        }
        alkh_app::auth::BootstrapOutcome::Created {
            generated_password: None,
        } => {
            tracing::info!(
                username = %config.admin_username,
                "已按 ALKH_ADMIN_PASSWORD 创建管理员账号"
            );
        }
        alkh_app::auth::BootstrapOutcome::AlreadyExists => {
            tracing::info!(username = %config.admin_username, "管理员账号已存在，跳过引导");
        }
    }
    alkh_app::bootstrap::ensure_default_templates(&state.deps).await?;

    state.ready.store(true, Ordering::Relaxed);
    workers::spawn(state.clone());

    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind(config.http_addr).await?;
    tracing::info!(addr = %config.http_addr, "服务已就绪，管理后台: http://{}/", config.http_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("正在刷写 SQLite WAL 并退出");
    alkh_infra::db::checkpoint(&pool).await;
    Ok(())
}

/// `--healthcheck`：TCP 连通性探测，成功退出码 0。
async fn healthcheck() -> anyhow::Result<()> {
    let addr: std::net::SocketAddr = std::env::var("ALKH_HTTP_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse()
        .map_err(|e| anyhow::anyhow!("ALKH_HTTP_ADDR 非法: {e}"))?;
    let target = if addr.ip().is_unspecified() {
        format!("127.0.0.1:{}", addr.port())
    } else {
        addr.to_string()
    };
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect(&target),
    )
    .await
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => anyhow::bail!("健康检查失败：无法连接 {target}: {e}"),
        Err(_) => anyhow::bail!("健康检查失败：连接 {target} 超时"),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "无法注册 SIGTERM 处理器");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("收到停机信号");
}
