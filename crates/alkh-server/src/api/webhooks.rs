//! AlertManager 接收端点。
//!
//! 路径：`POST /api/v1/hooks/alertmanager/{token}`
//!
//! 返回约定见 `docs/business-rules.md` §1：只要状态已持久化就返回 200（含被跳过的情况），
//! 这样 AlertManager 不会因为下游发送失败而重试风暴；真正的重试由后台任务负责。

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;

use alkh_app::error::AppError;

use crate::error::{ApiError, ApiResult};
use crate::metrics::Metrics;
use crate::state::AppState;

pub async fn alertmanager(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<alkh_app::intake::IntakeOutcome>> {
    // 未认证入口：任何来源都能打，先按来源 IP 过一道宽松限流。
    // 放在最前面，连 token 哈希与 DB 查询都省掉。
    let client = client_ip(&headers);
    if let Some(retry_after) = state.hook_limiter.check(client.as_deref()) {
        Metrics::inc(&state.metrics.hook_rejected_total);
        tracing::warn!(
            retry_after_secs = retry_after,
            "hook 接收端点触发限流，已拒绝"
        );
        return Err(ApiError(AppError::rate_limited(retry_after)));
    }

    match state.intake.handle_hook(&token, &body).await {
        Ok(outcome) => {
            Metrics::inc(&state.metrics.hook_received_total);
            if outcome.skipped.is_some() {
                Metrics::inc(&state.metrics.hook_skipped_total);
            }
            if let Some(event) = outcome.event {
                match event {
                    alkh_domain::alert::EventKind::Resolved => {
                        Metrics::inc(&state.metrics.alerts_resolved_total)
                    }
                    _ => Metrics::inc(&state.metrics.alerts_firing_total),
                }
            }
            // 指标只反映**实际结果**：pending 交给后台重试任务在终态时计数。
            use alkh_domain::alert::{DeliveryAction as A, DeliveryState as S};
            match (outcome.action, outcome.delivery_state) {
                (Some(A::SendNew) | Some(A::SendNewThenRevoke), Some(S::Active)) => {
                    Metrics::inc(&state.metrics.deliveries_sent_total)
                }
                (Some(A::SendNew) | Some(A::SendNewThenRevoke), Some(S::Failed)) => {
                    Metrics::inc(&state.metrics.deliveries_failed_total)
                }
                (Some(A::UpdateInPlace), Some(S::Active) | Some(S::Resolved)) => {
                    Metrics::inc(&state.metrics.deliveries_updated_total)
                }
                (Some(A::Suppress), _) => Metrics::inc(&state.metrics.deliveries_suppressed_total),
                _ => {}
            }
            Ok(Json(outcome))
        }
        Err(err) => {
            Metrics::inc(&state.metrics.hook_rejected_total);
            Err(ApiError(err))
        }
    }
}

/// 取来源 IP：优先 `X-Forwarded-For` 的第一段（反代场景），其次 `X-Real-IP`。
fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
}
