//! # alkh-app
//!
//! 应用层：用例编排、投递决策执行、卡片渲染、后台任务。只依赖 `alkh-domain` 的端口抽象。

pub mod aggregator;
pub mod auth;
pub mod bootstrap;
pub mod channels;
pub mod defaults;
pub mod deps;
pub mod error;
pub mod intake;
pub mod meta;
pub mod mock;
pub mod policies;
pub mod query;
pub mod render;
pub mod sender;
pub mod settings;
pub mod templates;

pub use deps::Deps;
pub use error::{AppError, AppResult};
