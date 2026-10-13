//! HTTP API 路由（对应 `docs/api.md`）。

pub mod alerts;
pub mod auth;
pub mod channels;
pub mod settings;
pub mod templates;
pub mod webhooks;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::metrics::Metrics;
use crate::state::AppState;

/// 组装 `/api/v1` 下的全部路由。
pub fn router() -> Router<AppState> {
    Router::new()
        // --- 元数据与健康 ---
        .route("/meta", get(meta))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        // --- 鉴权 ---
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/session", get(auth::session))
        .route("/auth/password", post(auth::change_password))
        // --- 设置 ---
        .route("/settings", get(settings::get).put(settings::update))
        .route("/settings/check", post(settings::check))
        // --- 渠道 ---
        .route("/channels", get(channels::list).post(channels::create))
        .route("/channels/sync", post(channels::sync))
        .route(
            "/channels/{id}",
            get(channels::get)
                .patch(channels::update)
                .delete(channels::delete),
        )
        .route(
            "/channels/{id}/policy",
            get(channels::get_policy).put(channels::put_policy),
        )
        .route("/channels/{id}/test-message", post(channels::test_message))
        // --- Hook ---
        .route(
            "/hooks",
            get(channels::list_hooks).post(channels::create_hook),
        )
        .route(
            "/hooks/{id}",
            axum::routing::patch(channels::update_hook).delete(channels::delete_hook),
        )
        .route("/hooks/{id}/rotate", post(channels::rotate_hook))
        // AlertManager 接收端点（token 即凭证，无需会话）
        .route("/hooks/alertmanager/{token}", post(webhooks::alertmanager))
        // --- 卡片模板 ---
        .route(
            "/card-templates",
            get(templates::list).post(templates::create),
        )
        .route("/card-templates/validate", post(templates::validate))
        .route("/card-templates/preview", post(templates::preview))
        .route("/card-templates/default", get(templates::defaults))
        .route("/card-templates/mock", get(templates::mock_baseline))
        .route(
            "/card-templates/send-test",
            post(templates::send_test_draft),
        )
        .route(
            "/card-templates/{id}",
            get(templates::get)
                .patch(templates::update)
                .delete(templates::delete),
        )
        .route("/card-templates/{id}/send-test", post(templates::send_test))
        // --- 告警与投递 ---
        .route("/alert-groups", get(alerts::list_groups))
        .route(
            "/alert-groups/{id}",
            get(alerts::get_group).delete(alerts::delete_group),
        )
        .route("/deliveries", get(alerts::list_deliveries))
        .route("/deliveries/{id}/retry", post(alerts::retry_delivery))
        .route("/deliveries/{id}/revoke", post(alerts::revoke_delivery))
        // --- 概览与审计 ---
        .route("/overview", get(alerts::overview))
        .route("/audit-logs", get(alerts::audit_logs))
        .fallback(api_not_found)
}

async fn api_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": {"code": "not_found", "message": "接口不存在"}})),
    )
        .into_response()
}

/// 枚举与内置变量清单，供前端渲染表单。
async fn meta() -> impl IntoResponse {
    Json(alkh_app::meta::meta())
}

async fn healthz_inner() -> impl IntoResponse {
    Json(json!({ "status": "ok", "version": alkh_app::meta::VERSION }))
}

pub async fn healthz() -> impl IntoResponse {
    healthz_inner().await
}

pub async fn readyz(State(state): State<AppState>) -> Response {
    let db_ok = alkh_infra::db::health_check(&state.pool).await.is_ok();
    let lark_configured = state
        .deps
        .settings
        .get()
        .await
        .map(|s| s.is_configured())
        .unwrap_or(false);
    let ready = db_ok && state.ready.load(std::sync::atomic::Ordering::Relaxed);

    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    Metrics::inc(&state.metrics.http_requests_total);
    (
        status,
        Json(json!({
            "ready": ready,
            "database": db_ok,
            "lark_configured": lark_configured,
            "version": alkh_app::meta::VERSION,
        })),
    )
        .into_response()
}

/// 通用分页查询参数。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PageQuery {
    #[serde(default)]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

/// 把创建/轮换 Hook 的明文 token 合并进渠道/ Hook 响应（只此一次返回）。
pub fn attach_token(value: &mut serde_json::Value, token: &str, hook_url: &str) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert("hook_token".into(), json!(token));
        obj.insert("hook_url".into(), json!(hook_url));
    }
}
