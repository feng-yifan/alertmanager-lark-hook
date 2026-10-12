//! # alkh-domain
//!
//! 纯领域层。这里只有业务概念、不变量与端口（trait）定义，**不依赖**任何 I/O、框架或运行时。
//!
//! 分层：`alkh-server` → `alkh-app` → `alkh-domain`；`alkh-infra` 实现 `ports` 中的 trait。

pub mod alert;
pub mod audit;
pub mod auth;
pub mod channel;
pub mod error;
pub mod hook;
pub mod ids;
pub mod policy;
pub mod ports;
pub mod settings;
pub mod template;

use std::collections::BTreeMap;

pub use error::{DomainError, DomainResult, LarkError, PortError, PortResult};
pub use ids::*;

/// 领域层统一时间戳：UTC。
pub type Timestamp = chrono::DateTime<chrono::Utc>;

/// 标签集合。使用 `BTreeMap` 以保证序列化顺序稳定（内容哈希、渲染结果可复现）。
pub type Labels = BTreeMap<String, String>;

/// 注解集合。
pub type Annotations = BTreeMap<String, String>;

/// 当前时间。
pub fn now() -> Timestamp {
    chrono::Utc::now()
}

/// 用于分页查询的通用切片。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, total: u64, limit: u32, offset: u32) -> Self {
        Self {
            items,
            total,
            limit,
            offset,
        }
    }
}
