//! AlertManager 接收端点。
//!
//! 路径：`POST /api/v1/hooks/alertmanager/{token}`
//!
//! 返回约定见 `docs/business-rules.md` §1：只要状态已持久化就返回 200（含被跳过的情况），
//! 这样 AlertManager 不会因为下游发送失败而重试风暴；真正的重试由后台任务负责。

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::Json;

use crate::error::{ApiError, ApiResult};
use crate::metrics::Metrics;
use crate::state::AppState;

pub async fn alertmanager(
    State(state): State<AppState>,
    Path(token): Path<String>,
    body: Bytes,
) -> ApiResult<Json<alkh_app::intake::IntakeOutcome>> {
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
