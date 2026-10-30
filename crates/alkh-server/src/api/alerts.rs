//! 告警分组、投递、概览与审计的 HTTP 接口。

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use alkh_app::query::{AlertGroupDetail, Overview};
use alkh_domain::alert::{AlertGroup, Delivery, DeliveryState, GroupStatus};
use alkh_domain::audit::AuditLog;
use alkh_domain::ids::{AlertGroupId, ChannelId, DeliveryId};
use alkh_domain::ports::{AlertGroupFilter, AuditFilter, DeliveryFilter};
use alkh_domain::Page;

use crate::api::auth::Admin;
use crate::api::PageQuery;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use alkh_app::AppError;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct GroupQuery {
    pub status: Option<GroupStatus>,
    pub channel_id: Option<ChannelId>,
    pub severity: Option<String>,
    pub q: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct DeliveryQuery {
    pub alert_group_id: Option<AlertGroupId>,
    pub channel_id: Option<ChannelId>,
    pub state: Option<DeliveryState>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AuditQuery {
    pub action: Option<String>,
    pub outcome: Option<String>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: u32,
    pub offset: u32,
}

pub async fn overview(State(state): State<AppState>, _admin: Admin) -> ApiResult<Json<Overview>> {
    Ok(Json(state.query.overview().await.map_err(ApiError::from)?))
}

pub async fn list_groups(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<GroupQuery>,
) -> ApiResult<Json<Page<AlertGroup>>> {
    let filter = AlertGroupFilter {
        status: q.status,
        channel_id: q.channel_id,
        severity: q.severity,
        q: q.q,
        limit: q.limit,
        offset: q.offset,
    };
    Ok(Json(
        state
            .query
            .alert_groups(filter)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub async fn get_group(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<AlertGroupId>,
) -> ApiResult<Json<AlertGroupDetail>> {
    Ok(Json(
        state
            .query
            .alert_group_detail(&id)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub async fn delete_group(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<AlertGroupId>,
) -> ApiResult<Response> {
    state
        .query
        .delete_alert_group(&id)
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

/// 管理端「忽略并 unpin」：撤销该分组 Pin 并停止后续 Pin（等价于点卡片按钮）。
pub async fn ignore_pin(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<AlertGroupId>,
) -> ApiResult<Json<alkh_app::intake::IgnoreOutcome>> {
    Ok(Json(
        state
            .intake
            .ignore_and_unpin(&id, None)
            .await
            .map_err(ApiError::from)?,
    ))
}

/// 管理端「恢复 Pin」：清除忽略标志并按策略重新对账（闭环反向出口）。
pub async fn unignore_pin(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<AlertGroupId>,
) -> ApiResult<Json<alkh_app::intake::IgnoreOutcome>> {
    Ok(Json(
        state
            .intake
            .unignore_pin(&id)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub async fn list_deliveries(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<DeliveryQuery>,
) -> ApiResult<Json<Page<Delivery>>> {
    let filter = DeliveryFilter {
        alert_group_id: q.alert_group_id,
        channel_id: q.channel_id,
        state: q.state,
        limit: q.limit,
        offset: q.offset,
    };
    Ok(Json(
        state
            .query
            .deliveries(filter)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub async fn retry_delivery(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<DeliveryId>,
) -> ApiResult<Json<Delivery>> {
    let delivery = state
        .query
        .get_delivery(&id)
        .await
        .map_err(ApiError::from)?;
    if matches!(
        delivery.state,
        DeliveryState::Active | DeliveryState::Resolved
    ) {
        return Err(ApiError(AppError::conflict(
            "该投递已是活跃消息，无需重试；请先撤回或等待新的告警事件",
        )));
    }
    let updated = state
        .sender()
        .retry(&delivery)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(updated))
}

pub async fn revoke_delivery(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<DeliveryId>,
) -> ApiResult<Json<Delivery>> {
    let delivery = state
        .query
        .get_delivery(&id)
        .await
        .map_err(ApiError::from)?;
    if !delivery.has_message_ref() {
        return Err(ApiError(AppError::conflict(
            "该投递没有可撤回的飞书消息引用",
        )));
    }
    let updated = state
        .sender()
        .revoke_delivery(&delivery)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(updated))
}

pub async fn audit_logs(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Page<AuditLog>>> {
    let filter = AuditFilter {
        action: q.action,
        outcome: q.outcome,
        since: q.since,
        limit: if q.limit == 0 { 100 } else { q.limit },
        offset: q.offset,
    };
    Ok(Json(
        state
            .query
            .audit_logs(filter)
            .await
            .map_err(ApiError::from)?,
    ))
}

/// 便于测试的分页参数解析（`PageQuery` 在 `api::mod` 中定义）。
#[allow(dead_code)]
fn page_defaults(q: PageQuery) -> (u32, u32) {
    (if q.limit == 0 { 50 } else { q.limit }, q.offset)
}
