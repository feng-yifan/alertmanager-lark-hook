//! 端口（ports）：领域层向外声明的能力契约，由 `alkh-infra` 实现。

use async_trait::async_trait;

use crate::alert::{AlertGroup, Delivery, DeliveryState, GroupStatus};
use crate::audit::{AuditActor, AuditLog};
use crate::auth::{AdminUser, Session};
use crate::channel::{Channel, ReceiveIdType};
use crate::error::PortResult;
use crate::hook::Hook;
use crate::ids::{
    AdminUserId, AlertGroupId, CardTemplateId, ChannelId, DeliveryId, HookId, SessionId,
};
use crate::policy::NotificationPolicy;
use crate::settings::{AppSettings, EncryptedSecret};
use crate::template::CardTemplate;
use crate::{Page, Timestamp};

// ---------------------------------------------------------------------------
// 查询条件
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ChannelFilter {
    pub enabled: Option<bool>,
    pub q: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Default)]
pub struct AlertGroupFilter {
    pub status: Option<GroupStatus>,
    pub channel_id: Option<ChannelId>,
    pub severity: Option<String>,
    pub q: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DeliveryFilter {
    pub alert_group_id: Option<AlertGroupId>,
    pub channel_id: Option<ChannelId>,
    pub state: Option<DeliveryState>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    pub action: Option<String>,
    pub outcome: Option<String>,
    /// 只返回该时间之后的记录。
    pub since: Option<Timestamp>,
    pub limit: u32,
    pub offset: u32,
}

/// 统一把 limit 收敛到安全范围。
pub fn clamp_limit(limit: u32, default: u32, max: u32) -> u32 {
    if limit == 0 {
        default
    } else {
        limit.min(max)
    }
}

// ---------------------------------------------------------------------------
// 飞书网关
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LarkCredential {
    pub app_id: String,
    pub app_secret: String,
    pub base_url: String,
}

/// 运行时凭据提供者：从设置中读取并解密（凭据可在管理后台随时更换）。
#[async_trait]
pub trait LarkCredentials: Send + Sync {
    async fn current(&self) -> PortResult<Option<LarkCredential>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LarkChat {
    pub chat_id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub description: Option<String>,
    pub external: bool,
    pub dissolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct LarkSendOutcome {
    /// `im/v1/messages` 返回的消息 ID。
    pub message_id: Option<String>,
}

/// 需要被观测的事件（指标维度）。
///
/// 领域/应用层只声明「这里发生了一件值得计数的事」，不依赖任何具体指标实现 ——
/// 否则这些计数器就会像 `deliveries_recalled_total` 那样：声明了、导出了，
/// 却因为 app 层拿不到 metrics 而永远保持 0。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counter {
    /// 卡片渲染失败。
    RenderFailure,
    /// 飞书开放平台返回错误。
    LarkApiError,
    /// 成功撤回（删除）已发出的消息。
    DeliveryRecalled,
    /// 成功 Pin 一条告警卡片（R4）。
    MessagePinned,
    /// 成功取消 Pin（R4）。
    MessageUnpinned,
    /// 成功执行「忽略并 unpin」（R4c）。
    PinsIgnored,
}

/// 指标出口；由 `alkh-server` 实现为 Prometheus 文本指标。
pub trait MetricsSink: Send + Sync {
    fn incr(&self, counter: Counter, by: u64);
}

/// 空实现：测试与不关心指标的调用方使用。
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMetrics;

impl MetricsSink for NoMetrics {
    fn incr(&self, _counter: Counter, _by: u64) {}
}

/// 飞书开放平台网关。
#[async_trait]
pub trait LarkGateway: Send + Sync {
    /// 校验凭据：换取 tenant_access_token 并拉取群列表。
    async fn verify_credentials(&self) -> PortResult<Vec<LarkChat>>;

    /// `GET /open-apis/im/v1/chats`（分页聚合）。
    async fn list_chats(&self) -> PortResult<Vec<LarkChat>>;

    /// `POST /open-apis/cardkit/v1/cards`（`type=card_json`）→ `card_id`。
    async fn create_card_entity(&self, card_json: &serde_json::Value) -> PortResult<String>;

    /// `POST /open-apis/cardkit/v1/cards`（`type=template`）→ `card_id`。
    async fn create_card_entity_from_template(
        &self,
        template_id: &str,
        template_version_name: Option<&str>,
        variables: &serde_json::Map<String, serde_json::Value>,
    ) -> PortResult<String>;

    /// `POST /open-apis/im/v1/messages`，`content = {"type":"card","data":{"card_id":…}}`。
    async fn send_card_entity(
        &self,
        receive_id: &str,
        receive_id_type: ReceiveIdType,
        card_entity_id: &str,
        uuid: &str,
    ) -> PortResult<LarkSendOutcome>;

    /// `POST /open-apis/im/v1/messages`，`content` 原样传入（如搭建工具模板内容）。
    async fn send_message_content(
        &self,
        receive_id: &str,
        receive_id_type: ReceiveIdType,
        content: &serde_json::Value,
        uuid: &str,
    ) -> PortResult<LarkSendOutcome>;

    /// `PUT /open-apis/cardkit/v1/cards/:card_id` 全量更新卡片实体（`sequence` 必须严格递增）。
    async fn update_card_entity(
        &self,
        card_entity_id: &str,
        card_json: &serde_json::Value,
        sequence: i64,
        uuid: &str,
    ) -> PortResult<()>;

    /// `PATCH /open-apis/im/v1/messages/:message_id` 原地更新已发送的卡片消息。
    async fn update_message_content(
        &self,
        message_id: &str,
        content: &serde_json::Value,
    ) -> PortResult<()>;

    /// `DELETE /open-apis/im/v1/messages/:message_id` 撤回消息。
    async fn recall_message(&self, message_id: &str) -> PortResult<()>;

    /// `POST /open-apis/im/v1/pins` Pin 一条消息（R4）。
    ///
    /// 幂等：消息已经被 Pin 时飞书返回既有 Pin 信息，同样算成功。
    async fn pin_message(&self, message_id: &str) -> PortResult<()>;

    /// `DELETE /open-apis/im/v1/pins/:message_id` 取消 Pin（R4）。
    ///
    /// 幂等：消息未被 Pin 或已被撤回时飞书同样返回成功。
    async fn unpin_message(&self, message_id: &str) -> PortResult<()>;
}

// ---------------------------------------------------------------------------
// 基础设施能力
// ---------------------------------------------------------------------------

/// 敏感字段加解密（AES-256-GCM）。
pub trait SecretCipher: Send + Sync {
    fn encrypt(&self, plaintext: &str) -> PortResult<EncryptedSecret>;
    fn decrypt(&self, secret: &EncryptedSecret) -> PortResult<String>;
}

/// 密码哈希（Argon2id）。
pub trait PasswordHasher: Send + Sync {
    fn hash(&self, password: &str) -> PortResult<String>;
    fn verify(&self, password: &str, phc: &str) -> PortResult<bool>;
}

/// 时钟抽象，便于测试注入。
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// 投递幂等去重窗口（R9）。
///
/// 语义是「**认领**」而不是「记录」：先原子地占住这个 hash，处理成功后保持占用，
/// 处理失败则用 [`release`](Self::release) 归还。
#[async_trait]
pub trait EventDedup: Send + Sync {
    /// 原子认领一个事件 hash。
    ///
    /// - 窗口内**首次**到达 ⇒ 写入记录并返回 `true`
    /// - 窗口内**重复**到达 ⇒ 返回 `false`（调用方应跳过）
    /// - 记录已存在但早于窗口 ⇒ 刷新时间戳并返回 `true`（窗口已滑过，可再次处理）
    ///
    /// 实现必须用**单条原子语句**完成「检查 + 写入」，不能用先查后写：
    /// 两个并发的相同请求必须只有一个拿到认领。
    ///
    /// ⚠️ 认领成功但后续处理失败时，调用方**必须**调用 [`release`](Self::release)。
    /// 否则 `AlertManager` 的重试会落进同一个窗口被判为重复，而重试会收到 200，
    /// 于是这条告警被静默丢弃。
    async fn claim(
        &self,
        hook_id: &HookId,
        hash: &str,
        at: Timestamp,
        window_secs: i64,
    ) -> PortResult<bool>;

    /// 归还认领（处理失败时调用），使同一 hash 可以被重新认领。
    async fn release(&self, hook_id: &HookId, hash: &str) -> PortResult<()>;

    async fn purge_before(&self, before: Timestamp) -> PortResult<u64>;
}

// ---------------------------------------------------------------------------
// 仓储
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ChannelRepository: Send + Sync {
    async fn list(&self, filter: ChannelFilter) -> PortResult<Page<Channel>>;
    async fn get(&self, id: &ChannelId) -> PortResult<Option<Channel>>;
    async fn find_by_receive_id(
        &self,
        receive_id_type: ReceiveIdType,
        receive_id: &str,
    ) -> PortResult<Option<Channel>>;
    /// 按主键插入或更新。
    async fn upsert(&self, channel: &Channel) -> PortResult<()>;
    async fn delete(&self, id: &ChannelId) -> PortResult<()>;
    async fn count(&self) -> PortResult<(u64, u64)>;
}

#[async_trait]
pub trait HookRepository: Send + Sync {
    async fn list(&self, channel_id: Option<&ChannelId>) -> PortResult<Vec<Hook>>;
    async fn get(&self, id: &HookId) -> PortResult<Option<Hook>>;
    async fn find_by_token_hash(&self, token_hash: &str) -> PortResult<Option<Hook>>;
    async fn insert(&self, hook: &Hook) -> PortResult<()>;
    async fn update(&self, hook: &Hook) -> PortResult<()>;
    async fn delete(&self, id: &HookId) -> PortResult<()>;
    /// 记录一次成功到达的调用。
    async fn record_call(&self, id: &HookId, at: Timestamp) -> PortResult<()>;
    async fn count(&self) -> PortResult<(u64, u64)>;
}

#[async_trait]
pub trait CardTemplateRepository: Send + Sync {
    async fn list(&self) -> PortResult<Vec<CardTemplate>>;
    async fn get(&self, id: &CardTemplateId) -> PortResult<Option<CardTemplate>>;
    async fn insert(&self, template: &CardTemplate) -> PortResult<()>;
    async fn update(&self, template: &CardTemplate) -> PortResult<()>;
    async fn delete(&self, id: &CardTemplateId) -> PortResult<()>;
    async fn find_default(&self) -> PortResult<Option<CardTemplate>>;
    /// 清空默认标记，用于设置唯一默认。
    async fn clear_default(&self) -> PortResult<()>;
}

#[async_trait]
pub trait NotificationPolicyRepository: Send + Sync {
    async fn get(&self, channel_id: &ChannelId) -> PortResult<Option<NotificationPolicy>>;
    async fn upsert(&self, policy: &NotificationPolicy) -> PortResult<()>;
    /// 有多少渠道策略引用了该模板。
    async fn count_by_template(&self, id: &CardTemplateId) -> PortResult<u64>;
    /// 删除某渠道的策略（渠道删除时级联）。
    async fn delete_for_channel(&self, channel_id: &ChannelId) -> PortResult<()>;
}

#[async_trait]
pub trait AlertGroupRepository: Send + Sync {
    async fn find(&self, hook_id: &HookId, group_key: &str) -> PortResult<Option<AlertGroup>>;
    async fn get(&self, id: &AlertGroupId) -> PortResult<Option<AlertGroup>>;
    async fn upsert(&self, group: &AlertGroup) -> PortResult<()>;
    /// 设置/清除当前活跃投递引用，并可选刷新通知时间。
    async fn set_active_delivery(
        &self,
        id: &AlertGroupId,
        delivery_id: Option<&DeliveryId>,
        notified_at: Option<Timestamp>,
    ) -> PortResult<()>;
    async fn list(&self, filter: AlertGroupFilter) -> PortResult<Page<AlertGroup>>;
    async fn delete(&self, id: &AlertGroupId) -> PortResult<()>;
    /// 返回 (firing, resolved) 计数。
    async fn count_by_status(&self) -> PortResult<(u64, u64)>;
    async fn delete_before(&self, before: Timestamp) -> PortResult<u64>;
}

#[async_trait]
pub trait DeliveryRepository: Send + Sync {
    async fn insert(&self, delivery: &Delivery) -> PortResult<()>;
    async fn update(&self, delivery: &Delivery) -> PortResult<()>;
    async fn get(&self, id: &DeliveryId) -> PortResult<Option<Delivery>>;
    async fn list(&self, filter: DeliveryFilter) -> PortResult<Page<Delivery>>;
    async fn list_by_group(&self, alert_group_id: &AlertGroupId) -> PortResult<Vec<Delivery>>;
    /// 该分组当前活跃（可更新/可撤回）的投递。
    async fn find_active(&self, alert_group_id: &AlertGroupId) -> PortResult<Option<Delivery>>;
    /// 可重试的失败/待发送投递。
    async fn list_retryable(&self, limit: u32) -> PortResult<Vec<Delivery>>;
    /// 自 `since` 起按状态统计（用于概览）。
    async fn count_by_state_since(&self, since: Timestamp)
        -> PortResult<Vec<(DeliveryState, u64)>>;
    async fn delete_before(&self, before: Timestamp) -> PortResult<u64>;
}

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn get(&self) -> PortResult<AppSettings>;
    async fn save(&self, settings: &AppSettings) -> PortResult<()>;
}

#[async_trait]
pub trait AdminUserRepository: Send + Sync {
    async fn get(&self, id: &AdminUserId) -> PortResult<Option<AdminUser>>;
    async fn find_by_username(&self, username: &str) -> PortResult<Option<AdminUser>>;
    async fn insert(&self, user: &AdminUser) -> PortResult<()>;
    async fn update(&self, user: &AdminUser) -> PortResult<()>;
    async fn count(&self) -> PortResult<u64>;
}

#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn insert(&self, session: &Session) -> PortResult<()>;
    async fn find_by_token_hash(&self, token_hash: &str) -> PortResult<Option<Session>>;
    async fn delete(&self, id: &SessionId) -> PortResult<()>;
    async fn delete_for_user(&self, user_id: &AdminUserId) -> PortResult<()>;
    async fn purge_expired(&self, now: Timestamp) -> PortResult<u64>;
}

#[async_trait]
pub trait AuditLogRepository: Send + Sync {
    async fn append(&self, log: &AuditLog) -> PortResult<()>;
    async fn append_many(&self, logs: &[AuditLog]) -> PortResult<()>;
    async fn list(&self, filter: AuditFilter) -> PortResult<Page<AuditLog>>;
    async fn recent(&self, limit: u32) -> PortResult<Vec<AuditLog>>;
    async fn delete_before(&self, before: Timestamp) -> PortResult<u64>;
}

/// 便于测试与日志的主体描述。
pub fn actor_label(actor: &AuditActor) -> String {
    match actor {
        AuditActor::Admin { username } => format!("admin:{username}"),
        AuditActor::AlertManager { hook_id } => format!("hook:{hook_id}"),
        AuditActor::System { job } => format!("job:{job}"),
    }
}
