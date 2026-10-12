//! 应用设置的 HTTP 接口。

use axum::extract::State;
use axum::Json;

use alkh_app::settings::{CredentialCheck, SettingsInput, SettingsView};

use crate::api::auth::Admin;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub async fn get(State(state): State<AppState>, _admin: Admin) -> ApiResult<Json<SettingsView>> {
    Ok(Json(state.settings.view().await.map_err(ApiError::from)?))
}

pub async fn update(
    State(state): State<AppState>,
    admin: Admin,
    Json(input): Json<SettingsInput>,
) -> ApiResult<Json<SettingsView>> {
    let actor = admin.actor();
    Ok(Json(
        state
            .settings
            .update(input, &actor)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub async fn check(
    State(state): State<AppState>,
    _admin: Admin,
) -> ApiResult<Json<CredentialCheck>> {
    Ok(Json(state.settings.check().await.map_err(ApiError::from)?))
}
