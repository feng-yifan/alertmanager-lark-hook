//! 卡片模板的 HTTP 接口。

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use alkh_app::channels::TestSample;
use alkh_app::mock::{MockBaseline, MockSpec};
use alkh_app::templates::{CardTemplateView, PreviewReport, TemplateInput, ValidationReport};
use alkh_domain::ids::{AlertGroupId, CardTemplateId, ChannelId};
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
    /// mock 覆盖（P1）：变量覆盖 + 可选的历史告警分组数据源。
    pub mock: MockSpec,
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
            mock: MockSpec::default(),
        }
    }
}

/// `GET /card-templates/mock` 的查询参数。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct MockQuery {
    pub channel_id: Option<ChannelId>,
    pub sample: Option<SampleArg>,
    /// 用历史告警分组作为基线数据源。
    pub alert_group_id: Option<AlertGroupId>,
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
    #[serde(default)]
    pub mock: MockSpec,
}

/// 用**未保存的草稿模板**发送测试消息（卡片编辑器 P1）。
#[derive(Debug, Clone, Deserialize)]
pub struct DraftSendTestRequest {
    pub channel_id: ChannelId,
    #[serde(flatten)]
    pub template: TemplateInput,
    #[serde(default)]
    pub sample: Option<TestSample>,
    #[serde(default)]
    pub mock: MockSpec,
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

/// 内置默认模板内容：卡片编辑器用作「起步卡片」与旧来源转换的落点。
pub async fn defaults(_admin: Admin) -> ApiResult<Json<alkh_app::templates::DefaultTemplatesView>> {
    Ok(Json(alkh_app::templates::default_templates()))
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
            .preview(req.template, preview_sample, req.channel_id, req.mock)
            .await
            .map_err(ApiError::from)?,
    ))
}

/// 返回某基线（内置样例 / 历史告警分组）渲染出的全部变量，供 mock 面板编辑。
pub async fn mock_baseline(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<MockQuery>,
) -> ApiResult<Json<MockBaseline>> {
    let sample = q.sample.map(Into::into).unwrap_or(TestSample::Firing);
    let preview_sample = match sample {
        TestSample::Firing => alkh_app::templates::PreviewSample::Firing,
        TestSample::Resolved => alkh_app::templates::PreviewSample::Resolved,
    };
    Ok(Json(
        state
            .templates
            .mock_baseline(q.channel_id, preview_sample, q.alert_group_id)
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
                &req.mock,
                &actor,
            )
            .await
            .map_err(ApiError::from)?,
    ))
}

/// 用请求体里的**草稿**模板发送测试消息：不落库、不污染模板列表。
pub async fn send_test_draft(
    State(state): State<AppState>,
    admin: Admin,
    Json(req): Json<DraftSendTestRequest>,
) -> ApiResult<Json<alkh_domain::alert::Delivery>> {
    let actor = admin.actor();
    let template = state
        .templates
        .build_draft(req.template)
        .map_err(ApiError::from)?;
    Ok(Json(
        state
            .channels
            .send_test_draft(
                &req.channel_id,
                &template,
                req.sample.unwrap_or_default(),
                &req.mock,
                &actor,
            )
            .await
            .map_err(ApiError::from)?,
    ))
}
