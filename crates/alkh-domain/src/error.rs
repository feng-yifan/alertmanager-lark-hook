use std::fmt;

/// 领域错误：业务规则的违反。
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    /// 输入不满足校验规则。
    #[error("{0}")]
    Validation(String),
    /// 引用的对象不存在。
    #[error("{0}")]
    NotFound(String),
    /// 与已有数据冲突（唯一性等）。
    #[error("{0}")]
    Conflict(String),
    /// 当前状态不允许该操作。
    #[error("{0}")]
    InvalidState(String),
}

impl DomainError {
    pub fn validation(msg: impl fmt::Display) -> Self {
        Self::Validation(msg.to_string())
    }
    pub fn not_found(msg: impl fmt::Display) -> Self {
        Self::NotFound(msg.to_string())
    }
    pub fn conflict(msg: impl fmt::Display) -> Self {
        Self::Conflict(msg.to_string())
    }
    pub fn invalid_state(msg: impl fmt::Display) -> Self {
        Self::InvalidState(msg.to_string())
    }
}

pub type DomainResult<T> = Result<T, DomainError>;

/// 飞书 OpenAPI 返回的业务错误（`code != 0` 或 HTTP 非 2xx）。
#[derive(Debug, Clone, thiserror::Error)]
#[error("lark api error (code={code}): {message}")]
pub struct LarkError {
    pub code: i64,
    pub message: String,
    pub log_id: Option<String>,
    pub http_status: Option<u16>,
    /// 是否值得重试（限频 / 5xx / 网络类）。
    pub retryable: bool,
}

impl LarkError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            log_id: None,
            http_status: None,
            retryable: false,
        }
    }

    pub fn retryable(mut self, yes: bool) -> Self {
        self.retryable = yes;
        self
    }

    pub fn with_log_id(mut self, log_id: Option<String>) -> Self {
        self.log_id = log_id;
        self
    }

    /// 飞书限频错误码（`230020`：当前操作触发频率限制）。
    pub const CODE_RATE_LIMITED: i64 = 230020;
    /// 通用参数错误。
    pub const CODE_INVALID_PARAM: i64 = 230001;
    /// **本适配器合成**：应用消息流卡片的 `failed_cards[].reason` 表示「卡片不存在 / 未创建」。
    ///
    /// 飞书该接口不返回数字错误码，只有 `reason` 枚举（`0` 未知 / `1` 无权限 /
    /// `2` 未创建 / `3` 频率限制 / `4` 重复，也常以 `NOT_CREATED` / `NO_PERMISSION` /
    /// `RATE_LIMITED` / `DUPLICATE` 出现）。归一成稳定码后，R1「原地更新不可用 ⇒
    /// 降级发新消息」才能对应用消息流卡片生效。
    pub const CODE_FEED_CARD_NOT_FOUND: i64 = -1001;
}

/// 端口错误：基础设施层的失败。应用层负责翻译成 HTTP。
#[derive(Debug, Clone, thiserror::Error)]
pub enum PortError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    /// 依赖不可用（网络、数据库连接）。
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// 适配器尚未完成必要配置（例如飞书凭据未填写）。
    #[error("{0}")]
    NotConfigured(String),
    #[error(transparent)]
    Lark(#[from] LarkError),
    #[error("internal: {0}")]
    Internal(String),
}

impl From<DomainError> for PortError {
    fn from(value: DomainError) -> Self {
        match value {
            DomainError::NotFound(m) => PortError::NotFound(m),
            DomainError::Conflict(m) => PortError::Conflict(m),
            other => PortError::Internal(other.to_string()),
        }
    }
}

pub type PortResult<T> = Result<T, PortError>;
