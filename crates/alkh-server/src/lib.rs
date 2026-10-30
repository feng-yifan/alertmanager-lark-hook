//! # alkh-server
//!
//! 交付层（组合根）：Axum HTTP 服务、内嵌管理后台、后台任务与可观测性。
//!
//! 二进制入口见 `src/main.rs`；本 lib 暴露 [`build_router`] 以便集成测试直接驱动真实 HTTP 栈。

pub mod api;
pub mod assets;
pub mod config;
pub mod credentials;
pub mod error;
pub mod limiter;
pub mod metrics;
pub mod state;
pub mod wiring;
pub mod workers;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tower_http::trace::TraceLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::metrics::Metrics;
use crate::state::AppState;

/// 组装完整路由：探针 + 管理 API + AlertManager 接收端点 + 静态前端。
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(api::healthz))
        .route("/readyz", get(api::readyz))
        .route("/metrics", get(metrics_handler))
        .nest("/api/v1", api::router())
        .fallback(assets::serve)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            count_requests,
        ))
        .with_state(state)
}

/// 初始化 `tracing` 订阅器。
pub fn init_telemetry(config: &Config) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.log_level.clone()));
    let registry = tracing_subscriber::registry().with(filter);
    if config.log_json {
        registry
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        registry.with(tracing_subscriber::fmt::layer()).init();
    }
}

async fn metrics_handler(State(state): State<AppState>) -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(),
    )
        .into_response()
}

async fn count_requests(State(state): State<AppState>, req: Request, next: Next) -> Response {
    Metrics::inc(&state.metrics.http_requests_total);
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let response = next.run(req).await;
    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        Metrics::inc(&state.metrics.http_errors_total);
    }
    // 诊断：回调路径与错误响应记录 method/path/status（不含请求体与请求头），
    // 便于排查飞书回调是否到达、打到哪个路径、返回什么状态。
    if path.contains("/lark/") || status.is_client_error() || status.is_server_error() {
        tracing::info!(%method, %path, %status, "http 请求");
    }
    response
}
