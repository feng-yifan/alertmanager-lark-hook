//! 卡片模板的 HTTP 接口。

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use alkh_app::channels::TestSample;
use alkh_app::templates::{CardTemplateView, PreviewReport, TemplateInput, ValidationReport};
use alkh_domain::ids::{CardTemplateId, ChannelId};
use alkh_domain::template::TemplateKind;

use crate::api::auth::Admin;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ListQuery {
    pub kind: Option<TemplateKind>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PreviewRequest {
    #[serde(flatten)]
    pub template: TemplateInput,
    pub sample: Option<SampleArg>,
    pub channel_id: Option<ChannelId>,
}

impl Default for PreviewRequest {
    fn default() -> Self {
        Self {
            template: TemplateInput {
                name: String::new(),
                kind: TemplateKind::CardMessage,
                card_message: None,
                app_feed_spec: None,
                is_default: false,
            },
            sample: None,
            channel_id: None,
        }
    }
}

/// 预览样例，接受 `firing` / `resolved`。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleArg {
    Firing,
    Resolved,
}

impl From<SampleArg> for TestSample {
    fn from(value: SampleArg) -> Self {
        match value {
            SampleArg::Firing => TestSample::Firing,
            SampleArg::Resolved => TestSample::Resolved,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendTestRequest {
    pub channel_id: ChannelId,
    #[serde(default)]
    pub sample: Option<TestSample>,
}

pub async fn list(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<CardTemplateView>>> {
    Ok(Json(
        state.templates.list(q.kind).await.map_err(ApiError::from)?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<CardTemplateId>,
) -> ApiResult<Json<CardTemplateView>> {
    Ok(Json(
        state.templates.get(&id).await.map_err(ApiError::from)?,
    ))
}

pub async fn create(
    State(state): State<AppState>,
    admin: Admin,
    Json(input): Json<TemplateInput>,
) -> ApiResult<Response> {
    let actor = admin.actor();
    let view = state
        .templates
        .create(input, &actor)
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

pub async fn update(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<CardTemplateId>,
    Json(input): Json<TemplateInput>,
) -> ApiResult<Json<CardTemplateView>> {
    let actor = admin.actor();
    Ok(Json(
        state
            .templates
            .update(&id, input, &actor)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<CardTemplateId>,
) -> ApiResult<Response> {
    let actor = admin.actor();
    state
        .templates
        .delete(&id, &actor)
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

pub async fn validate(
    _admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<TemplateInput>,
) -> ApiResult<Json<ValidationReport>> {
    Ok(Json(state.templates.validate(input)))
}

pub async fn preview(
    State(state): State<AppState>,
    _admin: Admin,
    Json(req): Json<PreviewRequest>,
) -> ApiResult<Json<PreviewReport>> {
    let sample = req.sample.map(Into::into).unwrap_or(TestSample::Firing);
    let preview_sample = match sample {
        TestSample::Firing => alkh_app::templates::PreviewSample::Firing,
        TestSample::Resolved => alkh_app::templates::PreviewSample::Resolved,
    };
    Ok(Json(
        state
            .templates
            .preview(req.template, preview_sample, req.channel_id)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub async fn send_test(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<CardTemplateId>,
    Json(req): Json<SendTestRequest>,
) -> ApiResult<Json<alkh_domain::alert::Delivery>> {
    let actor = admin.actor();
    let template = state.templates.get(&id).await.map_err(ApiError::from)?;
    let message_type = match template.template.kind {
        TemplateKind::CardMessage => alkh_domain::policy::MessageType::CardMessage,
        TemplateKind::AppFeedCard => alkh_domain::policy::MessageType::AppFeedCard,
    };
    Ok(Json(
        state
            .channels
            .send_test(
                &req.channel_id,
                Some(message_type),
                Some(id),
                req.sample.unwrap_or_default(),
                &actor,
            )
            .await
            .map_err(ApiError::from)?,
    ))
}
