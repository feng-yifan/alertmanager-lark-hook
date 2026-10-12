//! HTTP 错误映射：`AppError` → 状态码 + 统一错误信封。

use alkh_app::AppError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(value: AppError) -> Self {
        Self(value)
    }
}

impl From<alkh_domain::DomainError> for ApiError {
    fn from(value: alkh_domain::DomainError) -> Self {
        Self(value.into())
    }
}

impl From<alkh_domain::PortError> for ApiError {
    fn from(value: alkh_domain::PortError) -> Self {
        Self(value.into())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(value: serde_json::Error) -> Self {
        Self(AppError::Internal(format!("JSON 处理失败: {value}")))
    }
}

impl std::fmt::Debug for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ApiError").field(&self.0).finish()
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message, details) = match &self.0 {
            AppError::Validation(m) => (
                StatusCode::BAD_REQUEST,
                "validation_failed",
                m.clone(),
                None,
            ),
            AppError::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "未认证或会话已过期".to_string(),
                None,
            ),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", m.clone(), None),
            // 未知 hook token 使用专用错误码（`docs/api.md` §8）
            AppError::NotFound(m) if m == "unknown_hook" => (
                StatusCode::NOT_FOUND,
                "unknown_hook",
                "hook 地址无效".to_string(),
                None,
            ),
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m.clone(), None),
            AppError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone(), None),
            AppError::LarkNotConfigured(m) => (
                StatusCode::BAD_GATEWAY,
                "lark_not_configured",
                m.clone(),
                None,
            ),
            AppError::Lark(e) => {
                let details = json!({
                    "code": e.code,
                    "log_id": e.log_id,
                    "http_status": e.http_status,
                    "retryable": e.retryable,
                });
                (
                    StatusCode::BAD_GATEWAY,
                    "lark_api_error",
                    e.message.clone(),
                    Some(details),
                )
            }
            AppError::Internal(m) => {
                tracing::error!(error = %m, "内部错误");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "服务内部错误".to_string(),
                    None,
                )
            }
        };

        let body = match details {
            Some(d) => json!({ "error": { "code": code, "message": message, "details": d } }),
            None => json!({ "error": { "code": code, "message": message } }),
        };
        (status, Json(body)).into_response()
    }
}

/// `Result` 别名，便于 handler 返回。
pub type ApiResult<T> = Result<T, ApiError>;
