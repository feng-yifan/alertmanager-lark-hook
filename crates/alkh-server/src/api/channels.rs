//! 渠道与 Hook 的 HTTP 接口。

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use alkh_app::channels::{CreateChannel, SyncRequest, TestSample, UpdateChannel};
use alkh_app::policies::PolicyInput;
use alkh_domain::ids::{CardTemplateId, ChannelId, HookId};
use alkh_domain::policy::{MessageType, NotificationPolicy};
use alkh_domain::ports::ChannelFilter;
use alkh_domain::Page;

use crate::api::auth::Admin;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use alkh_app::AppError;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ListChannelsQuery {
    pub enabled: Option<bool>,
    pub q: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct DeleteQuery {
    pub force: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestMessageRequest {
    #[serde(default)]
    pub message_type: Option<MessageType>,
    #[serde(default)]
    pub card_template_id: Option<CardTemplateId>,
    #[serde(default)]
    pub sample: Option<TestSample>,
}

pub async fn list(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<ListChannelsQuery>,
) -> ApiResult<Json<Page<alkh_app::channels::ChannelSummary>>> {
    let filter = ChannelFilter {
        enabled: q.enabled,
        q: q.q.clone(),
        limit: q.limit,
        offset: q.offset,
    };
    Ok(Json(
        state.channels.list(filter).await.map_err(ApiError::from)?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<ChannelId>,
) -> ApiResult<Json<alkh_app::channels::ChannelDetail>> {
    Ok(Json(state.channels.get(&id).await.map_err(ApiError::from)?))
}

pub async fn create(
    State(state): State<AppState>,
    admin: Admin,
    Json(input): Json<CreateChannel>,
) -> ApiResult<Response> {
    let actor = admin.actor();
    let (detail, token) = state
        .channels
        .create(input, &actor)
        .await
        .map_err(ApiError::from)?;

    let settings = state.deps.settings.get().await.map_err(ApiError::from)?;
    let mut value = serde_json::to_value(&detail)?;
    crate::api::attach_token(&mut value, &token, &settings.hook_url(&token));
    Ok((StatusCode::CREATED, Json(value)).into_response())
}

pub async fn update(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<ChannelId>,
    Json(input): Json<UpdateChannel>,
) -> ApiResult<Json<alkh_app::channels::ChannelDetail>> {
    let actor = admin.actor();
    Ok(Json(
        state
            .channels
            .update(&id, input, &actor)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<ChannelId>,
    Query(q): Query<DeleteQuery>,
) -> ApiResult<Response> {
    let actor = admin.actor();
    state
        .channels
        .delete(&id, q.force.unwrap_or(false), &actor)
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

pub async fn sync(
    State(state): State<AppState>,
    admin: Admin,
    body: Option<Json<SyncRequest>>,
) -> ApiResult<Json<alkh_app::channels::SyncReport>> {
    let actor = admin.actor();
    let req = body.map(|Json(r)| r).unwrap_or(SyncRequest {
        create_missing: true,
        import_names: true,
    });
    Ok(Json(
        state
            .channels
            .sync(req, &actor)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub async fn get_policy(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<ChannelId>,
) -> ApiResult<Json<NotificationPolicy>> {
    Ok(Json(state.policies.get(&id).await.map_err(ApiError::from)?))
}

pub async fn put_policy(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<ChannelId>,
    Json(input): Json<PolicyInput>,
) -> ApiResult<Json<NotificationPolicy>> {
    let actor = admin.actor();
    Ok(Json(
        state
            .policies
            .upsert(&id, input, &actor)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub async fn test_message(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<ChannelId>,
    Json(req): Json<TestMessageRequest>,
) -> ApiResult<Json<alkh_domain::alert::Delivery>> {
    let actor = admin.actor();
    Ok(Json(
        state
            .channels
            .send_test(
                &id,
                req.message_type,
                req.card_template_id,
                req.sample.unwrap_or_default(),
                &actor,
            )
            .await
            .map_err(ApiError::from)?,
    ))
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ListHooksQuery {
    pub channel_id: Option<ChannelId>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateHookRequest {
    pub channel_id: ChannelId,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateHookRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RotateHookResponse {
    pub id: HookId,
    pub token: String,
    pub hook_url: String,
    pub hook_path: String,
    pub token_prefix: String,
    /// 明文只返回这一次。
    pub warning: &'static str,
}

pub async fn list_hooks(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<ListHooksQuery>,
) -> ApiResult<Json<Vec<alkh_app::channels::HookView>>> {
    let settings = state.deps.settings.get().await.map_err(ApiError::from)?;
    let hooks = state
        .deps
        .hooks
        .list(q.channel_id.as_ref())
        .await
        .map_err(ApiError::from)?;
    Ok(Json(
        hooks
            .iter()
            .map(|h| alkh_app::channels::HookView::from_hook(h, &settings))
            .collect(),
    ))
}

pub async fn create_hook(
    State(state): State<AppState>,
    admin: Admin,
    Json(req): Json<CreateHookRequest>,
) -> ApiResult<Response> {
    if state
        .deps
        .channels
        .get(&req.channel_id)
        .await
        .map_err(ApiError::from)?
        .is_none()
    {
        return Err(ApiError(AppError::not_found("渠道不存在")));
    }

    let (hook, token) = alkh_domain::hook::Hook::issue(req.channel_id.clone(), req.name);
    hook.validate().map_err(|e| ApiError(e.into()))?;
    state
        .deps
        .hooks
        .insert(&hook)
        .await
        .map_err(ApiError::from)?;

    let settings = state.deps.settings.get().await.map_err(ApiError::from)?;
    let actor = admin.actor();
    let log = alkh_domain::audit::AuditLog::ok(
        actor,
        alkh_domain::audit::action::HOOK_CREATED,
        state.deps.now(),
    )
    .target(hook.id.to_string())
    .detail(serde_json::json!({"channel_id": req.channel_id.to_string(), "name": hook.name}));
    let _ = state.deps.audit.append(&log).await;

    let mut value =
        serde_json::to_value(alkh_app::channels::HookView::from_hook(&hook, &settings))?;
    crate::api::attach_token(&mut value, &token, &settings.hook_url(&token));
    Ok((StatusCode::CREATED, Json(value)).into_response())
}

pub async fn update_hook(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<HookId>,
    Json(req): Json<UpdateHookRequest>,
) -> ApiResult<Json<alkh_app::channels::HookView>> {
    let mut hook = state
        .deps
        .hooks
        .get(&id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError(AppError::not_found("Hook 不存在")))?;
    if let Some(name) = req.name {
        hook.name = name;
    }
    if let Some(enabled) = req.enabled {
        hook.enabled = enabled;
    }
    hook.validate().map_err(|e| ApiError(e.into()))?;
    state
        .deps
        .hooks
        .update(&hook)
        .await
        .map_err(ApiError::from)?;

    let settings = state.deps.settings.get().await.map_err(ApiError::from)?;
    Ok(Json(alkh_app::channels::HookView::from_hook(
        &hook, &settings,
    )))
}

pub async fn delete_hook(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<HookId>,
) -> ApiResult<Response> {
    state.deps.hooks.delete(&id).await.map_err(ApiError::from)?;
    let log = alkh_domain::audit::AuditLog::ok(
        admin.actor(),
        alkh_domain::audit::action::HOOK_DELETED,
        state.deps.now(),
    )
    .target(id.to_string());
    let _ = state.deps.audit.append(&log).await;
    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

pub async fn rotate_hook(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<HookId>,
) -> ApiResult<Json<RotateHookResponse>> {
    let mut hook = state
        .deps
        .hooks
        .get(&id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError(AppError::not_found("Hook 不存在")))?;
    let token = hook.rotate();
    state
        .deps
        .hooks
        .update(&hook)
        .await
        .map_err(ApiError::from)?;

    let settings = state.deps.settings.get().await.map_err(ApiError::from)?;
    let log = alkh_domain::audit::AuditLog::ok(
        admin.actor(),
        alkh_domain::audit::action::HOOK_ROTATED,
        state.deps.now(),
    )
    .target(hook.id.to_string());
    let _ = state.deps.audit.append(&log).await;

    Ok(Json(RotateHookResponse {
        id: hook.id.clone(),
        token: token.clone(),
        hook_url: settings.hook_url(&token),
        hook_path: alkh_domain::hook::hook_path(&token),
        token_prefix: hook.token_prefix.clone(),
        warning: "明文 token 仅返回这一次，请立即写入 AlertManager 配置；旧的 token 已失效。",
    }))
}
