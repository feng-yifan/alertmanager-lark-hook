//! SQLite 仓储实现。
//!
//! 每个仓储结构体持有 `SqlitePool`，通过 `new(pool)` 构造。行 → 领域对象的映射
//! 统一使用 [`common`] 中的 JSON / 时间 / 枚举编解码助手。

pub mod alert_groups;
pub mod audit;
pub mod channels;
pub mod common;
pub mod dedup;
pub mod deliveries;
pub mod hooks;
pub mod policies;
pub mod settings;
pub mod templates;
pub mod users;
