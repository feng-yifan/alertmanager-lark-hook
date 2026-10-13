use alkh_domain::error::{DomainError, LarkError, PortError};

/// 应用层错误。`alkh-server` 负责映射为 HTTP 状态码与错误信封。
#[derive(Debug, Clone, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("未认证")]
    Unauthenticated,
    #[error("{0}")]
    Forbidden(String),
    /// 登录失败次数过多，暂时拒绝该来源（429）。`retry_after_secs` 为建议的等待秒数。
    #[error("尝试过于频繁")]
    RateLimited { retry_after_secs: u64 },
    /// 尚未配置飞书应用凭据（或其它必需配置）。
    #[error("{0}")]
    LarkNotConfigured(String),
    /// 飞书接口返回错误。
    #[error(transparent)]
    Lark(#[from] LarkError),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn validation(msg: impl std::fmt::Display) -> Self {
        Self::Validation(msg.to_string())
    }
    pub fn not_found(msg: impl std::fmt::Display) -> Self {
        Self::NotFound(msg.to_string())
    }
    pub fn conflict(msg: impl std::fmt::Display) -> Self {
        Self::Conflict(msg.to_string())
    }
    pub fn internal(msg: impl std::fmt::Display) -> Self {
        Self::Internal(msg.to_string())
    }
    pub fn rate_limited(retry_after_secs: u64) -> Self {
        Self::RateLimited { retry_after_secs }
    }
    pub fn not_configured(msg: impl std::fmt::Display) -> Self {
        Self::LarkNotConfigured(msg.to_string())
    }

    /// 机器可读错误码（对应 `docs/api.md` §8）。
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Validation(_) => "validation_failed",
            AppError::NotFound(_) => "not_found",
            AppError::Conflict(_) => "conflict",
            AppError::Unauthenticated => "unauthenticated",
            AppError::Forbidden(_) => "forbidden",
            AppError::RateLimited { .. } => "rate_limited",
            AppError::LarkNotConfigured(_) => "lark_not_configured",
            AppError::Lark(_) => "lark_api_error",
            AppError::Internal(_) => "internal_error",
        }
    }
}

impl From<DomainError> for AppError {
    fn from(value: DomainError) -> Self {
        match value {
            DomainError::Validation(m) => AppError::Validation(m),
            DomainError::NotFound(m) => AppError::NotFound(m),
            DomainError::Conflict(m) => AppError::Conflict(m),
            DomainError::InvalidState(m) => AppError::Conflict(m),
        }
    }
}

impl From<PortError> for AppError {
    fn from(value: PortError) -> Self {
        match value {
            PortError::NotFound(m) => AppError::NotFound(m),
            PortError::Conflict(m) => AppError::Conflict(m),
            PortError::Unavailable(m) => AppError::Internal(format!("依赖不可用: {m}")),
            PortError::NotConfigured(m) => AppError::LarkNotConfigured(m),
            PortError::Lark(e) => AppError::Lark(e),
            PortError::Internal(m) => AppError::Internal(m),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        AppError::Internal(format!("JSON 处理失败: {value}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_errors_map_to_app_errors() {
        let e: AppError = DomainError::Validation("x".into()).into();
        assert_eq!(e.code(), "validation_failed");
        let e: AppError = DomainError::NotFound("x".into()).into();
        assert_eq!(e.code(), "not_found");
        let e: AppError = DomainError::Conflict("x".into()).into();
        assert_eq!(e.code(), "conflict");
        let e: AppError = DomainError::InvalidState("x".into()).into();
        assert_eq!(e.code(), "conflict");
    }

    #[test]
    fn port_errors_map_to_app_errors() {
        let e: AppError = PortError::Unavailable("db down".into()).into();
        assert_eq!(e.code(), "internal_error");
        let e: AppError = PortError::Lark(LarkError::new(230002, "bot not in group")).into();
        assert_eq!(e.code(), "lark_api_error");
    }
}
