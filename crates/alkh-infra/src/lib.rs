//! # alkh-infra
//!
//! 适配器层：实现 `alkh-domain::ports` 中的端口。
//!
//! - [`sqlite`]：SQLite 仓储（sqlx + WAL）
//! - [`lark`]：飞书开放平台网关（tenant_access_token 缓存、限频重试）
//! - [`crypto`]：AES-256-GCM 密钥加密与 Argon2id 口令哈希
//! - [`clock`]：系统时钟
//! - [`db`]：连接池与迁移

pub mod clock;
pub mod crypto;
pub mod db;
pub mod lark;
pub mod sqlite;

pub use clock::SystemClock;
pub use crypto::{AesGcmCipher, Argon2Hasher};
pub use db::{connect, migrate};
pub use lark::LarkClient;
