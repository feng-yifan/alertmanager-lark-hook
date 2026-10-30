use std::collections::BTreeSet;

use chrono::Datelike;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{DomainError, DomainResult};
use crate::ids::{AlertGroupId, CardTemplateId, ChannelId, DeliveryId, HookId};
use crate::policy::{GroupUpdatePolicy, NotificationPolicy, RepeatPolicy, ResolvedPolicy};
use crate::{Annotations, Labels, Timestamp};

// ---------------------------------------------------------------------------
// AlertManager v4 输入契约
// ---------------------------------------------------------------------------

/// AlertManager `webhook_config` 推送的 JSON（协议版本 `4`）。
///
/// 时间字段保留为字符串：AlertManager 用 `0001-01-01T00:00:00Z` 表示 `endsAt` 零值，
/// 解析为字符串后再归一化更健壮。
#[derive(Debug, Clone, Deserialize)]
pub struct AlertManagerPayload {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(rename = "groupKey", default)]
    pub group_key: Option<String>,
    #[serde(rename = "truncatedAlerts", default)]
    pub truncated_alerts: u64,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub receiver: Option<String>,
    #[serde(rename = "groupLabels", default)]
    pub group_labels: Labels,
    #[serde(rename = "commonLabels", default)]
    pub common_labels: Labels,
    #[serde(rename = "commonAnnotations", default)]
    pub common_annotations: Annotations,
    #[serde(rename = "externalURL", default)]
    pub external_url: Option<String>,
    #[serde(default)]
    pub alerts: Vec<AmAlert>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AmAlert {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub labels: Labels,
    #[serde(default)]
    pub annotations: Annotations,
    #[serde(rename = "startsAt", default)]
    pub starts_at: Option<String>,
    #[serde(rename = "endsAt", default)]
    pub ends_at: Option<String>,
    #[serde(rename = "generatorURL", default)]
    pub generator_url: Option<String>,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

/// AlertManager 协议版本（`Message.Version` 常量）。
pub const ALERTMANAGER_PROTOCOL_VERSION: &str = "4";

// ---------------------------------------------------------------------------
// 归一化模型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertStatus {
    Firing,
    Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupStatus {
    Firing,
    Resolved,
}

impl GroupStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            GroupStatus::Firing => "firing",
            GroupStatus::Resolved => "resolved",
        }
    }
}

/// 单条告警（归一化后）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertInstance {
    pub fingerprint: String,
    pub status: AlertStatus,
    pub labels: Labels,
    pub annotations: Annotations,
    #[serde(default)]
    pub starts_at: Option<Timestamp>,
    #[serde(default)]
    pub ends_at: Option<Timestamp>,
    #[serde(default)]
    pub generator_url: Option<String>,
}

impl AlertInstance {
    /// 合并标签：告警自身标签覆盖 `commonLabels`。
    pub fn merged_labels(&self, common: &Labels) -> Labels {
        let mut merged = common.clone();
        for (k, v) in &self.labels {
            merged.insert(k.clone(), v.clone());
        }
        merged
    }

    pub fn is_firing(&self) -> bool {
        self.status == AlertStatus::Firing
    }

    /// 缺失 fingerprint 时按标签稳定派生。
    pub fn derive_fingerprint(labels: &Labels) -> String {
        let mut hasher = Sha256::new();
        for (k, v) in labels {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(v.as_bytes());
            hasher.update(b"\x1e");
        }
        hex::encode(&hasher.finalize()[..8])
    }
}

/// 一次 hook 请求归一化后的事件。
#[derive(Debug, Clone)]
pub struct NormalizedEvent {
    pub group_key: String,
    pub receiver: String,
    pub truncated_alerts: u64,
    pub declared_status: Option<GroupStatus>,
    pub status: GroupStatus,
    pub external_url: Option<String>,
    pub group_labels: Labels,
    pub common_labels: Labels,
    pub common_annotations: Annotations,
    pub alerts: Vec<AlertInstance>,
}

impl NormalizedEvent {
    pub fn from_payload(payload: &AlertManagerPayload) -> DomainResult<Self> {
        if let Some(v) = payload.version.as_deref() {
            if !v.is_empty() && v != ALERTMANAGER_PROTOCOL_VERSION {
                // 不拒绝：新版本可能只是新增字段。调用方负责记 warning。
            }
        }

        let mut alerts: Vec<AlertInstance> = Vec::with_capacity(payload.alerts.len());
        for a in &payload.alerts {
            let status = match a.status.as_deref() {
                Some("firing") | None => AlertStatus::Firing,
                Some("resolved") => AlertStatus::Resolved,
                Some(other) => {
                    return Err(DomainError::validation(format!(
                        "未知的告警状态: {other}（应为 firing / resolved）"
                    )))
                }
            };
            let mut labels = a.labels.clone();
            // 合并 commonLabels 中告警缺失的键，便于匹配与去重。
            for (k, v) in &payload.common_labels {
                labels.entry(k.clone()).or_insert_with(|| v.clone());
            }
            let fingerprint = a
                .fingerprint
                .clone()
                .filter(|f| !f.trim().is_empty())
                .unwrap_or_else(|| AlertInstance::derive_fingerprint(&labels));

            alerts.push(AlertInstance {
                fingerprint,
                status,
                labels,
                annotations: a.annotations.clone(),
                starts_at: parse_am_time(a.starts_at.as_deref()),
                ends_at: parse_am_time(a.ends_at.as_deref()),
                generator_url: sanitize_http_url(a.generator_url.as_deref()),
            });
        }

        let firing = alerts.iter().filter(|a| a.is_firing()).count();
        let status = if alerts.is_empty() {
            payload
                .status
                .as_deref()
                .and_then(|s| match s {
                    "firing" => Some(GroupStatus::Firing),
                    "resolved" => Some(GroupStatus::Resolved),
                    _ => None,
                })
                .unwrap_or(GroupStatus::Resolved)
        } else if firing > 0 {
            GroupStatus::Firing
        } else {
            GroupStatus::Resolved
        };

        let declared_status = payload.status.as_deref().and_then(|s| match s {
            "firing" => Some(GroupStatus::Firing),
            "resolved" => Some(GroupStatus::Resolved),
            _ => None,
        });

        let group_key = payload
            .group_key
            .clone()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .unwrap_or_else(|| {
                derive_group_key(&payload.group_labels, &payload.common_labels, &alerts)
            });

        Ok(Self {
            group_key,
            receiver: payload.receiver.clone().unwrap_or_default(),
            truncated_alerts: payload.truncated_alerts,
            declared_status,
            status,
            external_url: sanitize_http_url(payload.external_url.as_deref()),
            group_labels: payload.group_labels.clone(),
            common_labels: payload.common_labels.clone(),
            common_annotations: payload.common_annotations.clone(),
            alerts,
        })
    }

    pub fn firing_count(&self) -> u32 {
        self.alerts.iter().filter(|a| a.is_firing()).count() as u32
    }

    pub fn resolved_count(&self) -> u32 {
        self.alerts.iter().filter(|a| !a.is_firing()).count() as u32
    }

    /// 只保留命中策略过滤器（R11）的告警。
    pub fn matching_alerts(&self, policy: &NotificationPolicy) -> Vec<AlertInstance> {
        if !policy.has_filters() {
            return self.alerts.clone();
        }
        self.alerts
            .iter()
            .filter(|a| policy.matches_labels(&a.merged_labels(&self.common_labels)))
            .cloned()
            .collect()
    }

    pub fn severity(&self) -> Option<String> {
        self.common_labels.get("severity").cloned().or_else(|| {
            self.alerts
                .iter()
                .find_map(|a| a.labels.get("severity").cloned())
        })
    }

    pub fn alertname(&self) -> Option<String> {
        self.common_labels
            .get("alertname")
            .cloned()
            .or_else(|| self.group_labels.get("alertname").cloned())
            .or_else(|| {
                self.alerts
                    .iter()
                    .find_map(|a| a.labels.get("alertname").cloned())
            })
    }
}

/// 幂等键（R9）。
pub fn event_hash(hook_id: &HookId, event: &NormalizedEvent) -> String {
    let mut hasher = Sha256::new();
    hasher.update(hook_id.as_str().as_bytes());
    hasher.update(b"|");
    hasher.update(event.group_key.as_bytes());
    hasher.update(b"|");
    hasher.update(event.status.as_str().as_bytes());
    hasher.update(b"|");

    let mut rows: Vec<String> = event
        .alerts
        .iter()
        .map(|a| {
            format!(
                "{}:{}:{}",
                a.fingerprint,
                match a.status {
                    AlertStatus::Firing => "firing",
                    AlertStatus::Resolved => "resolved",
                },
                a.starts_at.map(|t| t.to_rfc3339()).unwrap_or_default()
            )
        })
        .collect();
    rows.sort();
    for r in rows {
        hasher.update(r.as_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

/// 活跃告警集合的内容哈希。集合不变 ⇒ 判定为"重复通知"而非"内容更新"。
pub fn content_hash(alerts: &[AlertInstance]) -> String {
    let mut hasher = Sha256::new();
    let mut active: Vec<&AlertInstance> = alerts.iter().filter(|a| a.is_firing()).collect();
    active.sort_by(|a, b| a.fingerprint.cmp(&b.fingerprint));
    for a in active {
        hasher.update(a.fingerprint.as_bytes());
        hasher.update(b"\x1f");
        for (k, v) in &a.labels {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(v.as_bytes());
            hasher.update(b"\x1e");
        }
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

fn derive_group_key(
    group_labels: &Labels,
    common_labels: &Labels,
    alerts: &[AlertInstance],
) -> String {
    // 优先 groupLabels，其次 commonLabels，最后退化为 fingerprint 集合。
    let source = if !group_labels.is_empty() {
        group_labels
    } else {
        common_labels
    };
    if !source.is_empty() {
        let inner: Vec<String> = source.iter().map(|(k, v)| format!("{k}=\"{v}\"")).collect();
        return format!("{{{}}}", inner.join(", "));
    }
    let mut fps: Vec<&str> = alerts.iter().map(|a| a.fingerprint.as_str()).collect();
    fps.sort_unstable();
    format!("{{fingerprint=~\"{}\"}}", fps.join("|"))
}

/// 只接受 `http://` / `https://` 的绝对 URL，其余一律丢弃。
///
/// 告警的 `generatorURL` / `externalURL` 完全由发送方控制（任何持有 hook token 的人
/// 都能构造）。如果原样入库，前端把它放进 `<a href>` 时 `javascript:` 会在管理后台
/// 同源执行脚本 —— 即「发一条告警」升级成「接管后台」。Feishu 卡片里的链接同理，
/// 所以这里在**归一化边界**就丢掉非法值，而不是等到渲染时再补救。
pub fn sanitize_http_url(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    // 不接受 `java\tscript:` 这类夹带控制字符的变体：只做前缀判断，
    // 且要求协议后面紧跟 `//`。
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Some(value.to_string())
    } else {
        None
    }
}

/// 解析 AlertManager 时间：非法或零值（year <= 1）返回 `None`。
fn parse_am_time(raw: Option<&str>) -> Option<Timestamp> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(raw).ok()?;
    let dt = parsed.with_timezone(&chrono::Utc);
    if dt.year() <= 1 {
        return None;
    }
    Some(dt)
}

// ---------------------------------------------------------------------------
// AlertGroup 聚合根
// ---------------------------------------------------------------------------

/// 一条 Pin 记录的角色：由「未解决告警 Pin」产生，还是由「已解决告警 Pin」产生。
///
/// 两者分开记录，才能在「全部恢复」时分别执行「取消未解决 Pin」与「Pin 恢复卡片」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    /// 未解决期间 Pin 的告警卡片。
    Unresolved,
    /// 全部恢复后 Pin 的恢复卡片。
    Resolved,
}

/// 一条被本服务 Pin 到会话的卡片消息（`om_…`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedMessage {
    pub message_id: String,
    pub kind: PinKind,
}

impl PinnedMessage {
    pub fn new(message_id: impl Into<String>, kind: PinKind) -> Self {
        Self {
            message_id: message_id.into(),
            kind,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertGroup {
    pub id: AlertGroupId,
    pub hook_id: HookId,
    pub channel_id: ChannelId,
    pub group_key: String,
    pub receiver: String,
    pub status: GroupStatus,
    pub alertname: Option<String>,
    pub severity: Option<String>,
    pub group_labels: Labels,
    pub common_labels: Labels,
    pub common_annotations: Annotations,
    pub alerts: Vec<AlertInstance>,
    pub firing_count: u32,
    pub resolved_count: u32,
    pub truncated_alerts: u64,
    pub event_count: u64,
    pub last_event_hash: Option<String>,
    pub external_url: Option<String>,
    pub first_seen_at: Timestamp,
    pub last_seen_at: Timestamp,
    pub last_notified_at: Option<Timestamp>,
    pub resolved_at: Option<Timestamp>,
    pub active_delivery_id: Option<DeliveryId>,
    /// 当前被本服务 Pin 的卡片消息（可多条）。
    ///
    /// `repeat_policy = send` 时每次重复通知都会新发一张卡片，未解决期间会逐条累积；
    /// 全部恢复时按策略批量取消（或保留）未解决 Pin，并可按「已解决告警 Pin」再 Pin
    /// 当前恢复卡片。为空表示该分组没有任何 Pin（R4）。
    pub pinned_messages: Vec<PinnedMessage>,
}

impl AlertGroup {
    pub fn new(
        hook_id: HookId,
        channel_id: ChannelId,
        event: &NormalizedEvent,
        now: Timestamp,
    ) -> Self {
        let mut g = Self {
            id: AlertGroupId::new(),
            hook_id,
            channel_id,
            group_key: event.group_key.clone(),
            receiver: event.receiver.clone(),
            status: event.status,
            alertname: event.alertname(),
            severity: event.severity(),
            group_labels: event.group_labels.clone(),
            common_labels: event.common_labels.clone(),
            common_annotations: event.common_annotations.clone(),
            alerts: event.alerts.clone(),
            firing_count: event.firing_count(),
            resolved_count: event.resolved_count(),
            truncated_alerts: event.truncated_alerts,
            event_count: 1,
            last_event_hash: None,
            external_url: event.external_url.clone(),
            first_seen_at: now,
            last_seen_at: now,
            last_notified_at: None,
            resolved_at: None,
            active_delivery_id: None,
            pinned_messages: Vec::new(),
        };
        g.normalize_status(now);
        g
    }

    /// 用新事件刷新快照与状态。不负责发送决策。
    pub fn apply_event(&mut self, event: &NormalizedEvent, hash: &str, now: Timestamp) {
        self.receiver = event.receiver.clone();
        self.group_labels = event.group_labels.clone();
        self.common_labels = event.common_labels.clone();
        self.common_annotations = event.common_annotations.clone();
        self.alerts = event.alerts.clone();
        self.firing_count = event.firing_count();
        self.resolved_count = event.resolved_count();
        self.truncated_alerts = event.truncated_alerts;
        self.alertname = event.alertname();
        self.severity = event.severity();
        self.external_url = event.external_url.clone();
        self.status = event.status;
        self.event_count += 1;
        self.last_event_hash = Some(hash.to_string());
        self.last_seen_at = now;
        self.normalize_status(now);
    }

    fn normalize_status(&mut self, now: Timestamp) {
        match self.status {
            GroupStatus::Resolved => {
                self.firing_count = 0;
                if self.resolved_at.is_none() {
                    self.resolved_at = Some(now);
                }
            }
            GroupStatus::Firing => {
                self.resolved_at = None;
            }
        }
    }

    /// 当前活跃（firing）告警的 fingerprint 集合。
    pub fn active_fingerprints(&self) -> BTreeSet<&str> {
        self.alerts
            .iter()
            .filter(|a| a.is_firing())
            .map(|a| a.fingerprint.as_str())
            .collect()
    }

    pub fn content_hash(&self) -> String {
        content_hash(&self.alerts)
    }

    pub fn mark_notified(&mut self, at: Timestamp) {
        self.last_notified_at = Some(at);
    }

    /// 恢复通知的语义校验：`status == Resolved` ⇔ 无 firing 告警。
    pub fn invariants_hold(&self) -> bool {
        match self.status {
            GroupStatus::Resolved => {
                self.alerts.iter().all(|a| !a.is_firing()) && self.resolved_at.is_some()
            }
            GroupStatus::Firing => {
                self.alerts.iter().any(|a| a.is_firing()) && self.resolved_at.is_none()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 事件分类与投递决策
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// 新告警分组（含恢复后重开）。
    NewFiring,
    /// 重复通知：内容哈希未变。
    Repeated,
    /// 分组内容变化但仍 firing（新增告警 / 部分恢复）。
    Updated,
    /// 全部恢复。
    Resolved,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::NewFiring => "new_firing",
            EventKind::Repeated => "repeated",
            EventKind::Updated => "updated",
            EventKind::Resolved => "resolved",
        }
    }
}

/// 事件分类（领域服务）。
pub fn classify(existing: Option<&AlertGroup>, event: &NormalizedEvent) -> EventKind {
    match existing {
        None => match event.status {
            GroupStatus::Firing => EventKind::NewFiring,
            // 未知告警的恢复通知：没有可更新/可撤回的消息，仍按 Resolved 记录（R19）。
            GroupStatus::Resolved => EventKind::Resolved,
        },
        Some(group) => match event.status {
            GroupStatus::Resolved => EventKind::Resolved,
            GroupStatus::Firing => {
                if group.status == GroupStatus::Resolved {
                    // R10：恢复后重新 firing ⇒ 视为新告警。
                    EventKind::NewFiring
                } else if content_hash(&event.alerts) == group.content_hash() {
                    EventKind::Repeated
                } else {
                    EventKind::Updated
                }
            }
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryAction {
    /// 发送新消息。
    SendNew,
    /// 发送新消息后撤回/删除旧消息。
    SendNewThenRevoke,
    /// 原地更新当前活跃消息。
    UpdateInPlace,
    /// 不发送。
    Suppress,
}

impl DeliveryAction {
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryAction::SendNew => "send_new",
            DeliveryAction::SendNewThenRevoke => "send_new_then_revoke",
            DeliveryAction::UpdateInPlace => "update_in_place",
            DeliveryAction::Suppress => "suppress",
        }
    }
}

/// 投递决策表（`docs/business-rules.md` §4）。
///
/// `has_active_previous` 表示该分组当前是否有一条可被更新/撤回的活跃消息。
/// 当为 `false` 时，`UpdateInPlace` 与 `SendNewThenRevoke` 都退化为 `SendNew`
/// （没有旧消息可更新，也没有旧消息可撤回）。
pub fn decide(
    kind: EventKind,
    policy: &NotificationPolicy,
    has_active_previous: bool,
) -> DeliveryAction {
    let raw = match kind {
        EventKind::NewFiring => DeliveryAction::SendNew,
        EventKind::Repeated => match policy.repeat_policy {
            RepeatPolicy::Suppress => DeliveryAction::Suppress,
            RepeatPolicy::Send => DeliveryAction::SendNew,
            RepeatPolicy::SendAndRevokePrevious => DeliveryAction::SendNewThenRevoke,
        },
        EventKind::Updated => match policy.group_update_policy {
            GroupUpdatePolicy::UpdatePrevious => DeliveryAction::UpdateInPlace,
            GroupUpdatePolicy::Send => DeliveryAction::SendNew,
            GroupUpdatePolicy::Suppress => DeliveryAction::Suppress,
        },
        EventKind::Resolved => match policy.resolved_policy {
            ResolvedPolicy::Send => DeliveryAction::SendNew,
            ResolvedPolicy::SendAndDeletePrevious => DeliveryAction::SendNewThenRevoke,
            ResolvedPolicy::UpdatePrevious => DeliveryAction::UpdateInPlace,
        },
    };

    match raw {
        DeliveryAction::UpdateInPlace if !has_active_previous => {
            // R19：未知告警的恢复通知，没有旧消息可更新 ⇒ 不发送，避免噪声。
            if kind == EventKind::Resolved {
                DeliveryAction::Suppress
            } else {
                DeliveryAction::SendNew
            }
        }
        DeliveryAction::SendNewThenRevoke if !has_active_previous => DeliveryAction::SendNew,
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Delivery 子实体
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryKind {
    FiringCreated,
    Repeated,
    Updated,
    Resolved,
    Test,
}

impl DeliveryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryKind::FiringCreated => "firing_created",
            DeliveryKind::Repeated => "repeated",
            DeliveryKind::Updated => "updated",
            DeliveryKind::Resolved => "resolved",
            DeliveryKind::Test => "test",
        }
    }
}

impl From<EventKind> for DeliveryKind {
    fn from(value: EventKind) -> Self {
        match value {
            EventKind::NewFiring => DeliveryKind::FiringCreated,
            EventKind::Repeated => DeliveryKind::Repeated,
            EventKind::Updated => DeliveryKind::Updated,
            EventKind::Resolved => DeliveryKind::Resolved,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// 已决策，尚未成功发送。
    Pending,
    /// 已发送且为当前活跃消息。
    Active,
    /// 被新消息取代，旧消息保留在会话中。
    Superseded,
    /// 已撤回 / 已删除。
    Recalled,
    /// 撤回失败（超出撤回时限、已被删除等）。
    RecallFailed,
    /// 已被原地更新为"已恢复"。
    Resolved,
    /// 重试耗尽。
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    pub id: DeliveryId,
    pub alert_group_id: AlertGroupId,
    pub channel_id: ChannelId,
    pub kind: DeliveryKind,
    pub state: DeliveryState,
    pub card_template_id: Option<CardTemplateId>,
    /// 卡片消息的 `message_id`（`om_…`）。
    pub lark_message_id: Option<String>,
    /// CardKit 卡片实体 ID（`card_json` 来源用于原地更新）。
    pub card_entity_id: Option<String>,
    /// CardKit 原地更新的 `sequence`，必须严格递增。
    pub card_sequence: i64,
    /// 渲染后内容摘要：相同则跳过原地更新（R3）。
    pub rendered_digest: String,
    pub attempt: u32,
    pub last_error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Delivery {
    /// 最大尝试次数（**含首次发送**）：1 次首发 + 5 次重试。
    ///
    /// 与 R14 的退避档位数量对齐（1s / 5s / 30s / 2m / 10m），否则最后一档永远用不到。
    pub const MAX_ATTEMPTS: u32 = 6;

    pub fn new(
        alert_group_id: AlertGroupId,
        channel_id: ChannelId,
        kind: DeliveryKind,
        card_template_id: Option<CardTemplateId>,
        rendered_digest: impl Into<String>,
        now: Timestamp,
    ) -> Self {
        Self {
            id: DeliveryId::new(),
            alert_group_id,
            channel_id,
            kind,
            state: DeliveryState::Pending,
            card_template_id,
            lark_message_id: None,
            card_entity_id: None,
            card_sequence: 0,
            rendered_digest: rendered_digest.into(),
            attempt: 0,
            last_error: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// 发送成功后置为活跃。
    pub fn mark_active(&mut self, now: Timestamp) {
        self.state = DeliveryState::Active;
        self.attempt = 0;
        self.last_error = None;
        self.updated_at = now;
    }

    pub fn mark_superseded(&mut self, now: Timestamp) {
        self.state = DeliveryState::Superseded;
        self.updated_at = now;
    }

    pub fn mark_recalled(&mut self, now: Timestamp) {
        self.state = DeliveryState::Recalled;
        self.updated_at = now;
    }

    pub fn mark_recall_failed(&mut self, err: impl Into<String>, now: Timestamp) {
        self.state = DeliveryState::RecallFailed;
        self.last_error = Some(err.into());
        self.updated_at = now;
    }

    pub fn mark_resolved(&mut self, now: Timestamp) {
        self.state = DeliveryState::Resolved;
        self.updated_at = now;
    }

    pub fn mark_failed(&mut self, err: impl Into<String>, now: Timestamp) {
        self.state = DeliveryState::Failed;
        self.last_error = Some(err.into());
        self.updated_at = now;
    }

    pub fn record_attempt(&mut self, err: Option<String>, now: Timestamp) {
        self.attempt += 1;
        self.last_error = err;
        self.updated_at = now;
    }

    pub fn exhausted(&self) -> bool {
        self.attempt >= Self::MAX_ATTEMPTS
    }

    /// 递增并返回下一次 CardKit 更新应使用的 `sequence`。
    pub fn next_sequence(&mut self) -> i64 {
        self.card_sequence += 1;
        self.card_sequence
    }

    /// 是否有可被原地更新 / 撤回的飞书消息引用。
    pub fn has_message_ref(&self) -> bool {
        self.lark_message_id.is_some()
    }

    pub fn is_active(&self) -> bool {
        self.state == DeliveryState::Active
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(v: serde_json::Value) -> AlertManagerPayload {
        serde_json::from_value(v).unwrap()
    }

    fn firing_payload(alertname: &str) -> AlertManagerPayload {
        payload(json!({
            "version": "4",
            "groupKey": format!("{{}}:{{alertname=\"{alertname}\"}}"),
            "status": "firing",
            "receiver": "lark",
            "commonLabels": {"alertname": alertname, "severity": "critical"},
            "commonAnnotations": {"summary": "cpu high"},
            "externalURL": "http://am:9093",
            "alerts": [{
                "status": "firing",
                "labels": {"alertname": alertname, "instance": "a:9100"},
                "annotations": {"summary": "cpu high"},
                "startsAt": "2026-10-13T00:00:00Z",
                "endsAt": "0001-01-01T00:00:00Z",
                "fingerprint": "fp1"
            }]
        }))
    }

    /// 安全回归：任何持有 hook token 的人都能构造 generatorURL / externalURL，
    /// 因此只允许 http(s)，其余（尤其是 `javascript:`）必须在归一化阶段丢弃，
    /// 否则前端把它放进 `<a href>` 会在管理后台同源执行脚本。
    #[test]
    fn only_http_urls_survive_sanitization() {
        assert_eq!(
            sanitize_http_url(Some("http://prom:9090/graph")),
            Some("http://prom:9090/graph".to_string())
        );
        assert_eq!(
            sanitize_http_url(Some("HTTPS://example.com/x")),
            Some("HTTPS://example.com/x".to_string())
        );
        for bad in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            " java\tscript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
            "//evil.example.com",
            "/relative/path",
            "file:///etc/passwd",
            "",
            "   ",
        ] {
            assert_eq!(sanitize_http_url(Some(bad)), None, "不应接受 {bad:?}");
        }
        assert_eq!(sanitize_http_url(None), None);
    }

    #[test]
    fn zero_ends_at_is_none() {
        let e = NormalizedEvent::from_payload(&firing_payload("HighCPU")).unwrap();
        assert_eq!(e.alerts[0].ends_at, None);
        assert!(e.alerts[0].starts_at.is_some());
    }

    #[test]
    fn status_is_derived_from_alerts_and_common_labels_are_merged() {
        let e = NormalizedEvent::from_payload(&firing_payload("HighCPU")).unwrap();
        assert_eq!(e.status, GroupStatus::Firing);
        assert_eq!(e.alerts[0].labels.get("severity").unwrap(), "critical");
        assert_eq!(e.severity().unwrap(), "critical");
        assert_eq!(e.alertname().unwrap(), "HighCPU");
    }

    #[test]
    fn group_key_is_derived_when_absent() {
        let mut p = firing_payload("X");
        p.group_key = Some("  ".into());
        let e = NormalizedEvent::from_payload(&p).unwrap();
        assert!(e.group_key.contains("alertname"));
    }

    #[test]
    fn resolved_event_status() {
        let p = payload(json!({
            "version": "4",
            "groupKey": "g",
            "status": "resolved",
            "alerts": [{"status": "resolved", "labels": {}, "fingerprint": "fp1"}]
        }));
        let e = NormalizedEvent::from_payload(&p).unwrap();
        assert_eq!(e.status, GroupStatus::Resolved);
    }

    #[test]
    fn classification_new_repeat_updated_resolved() {
        let e1 = NormalizedEvent::from_payload(&firing_payload("HighCPU")).unwrap();
        let now = crate::now();
        let group = AlertGroup::new(HookId::new(), ChannelId::new(), &e1, now);

        // 完全相同 → Repeated
        assert_eq!(classify(Some(&group), &e1), EventKind::Repeated);

        // 新增一条告警 → Updated
        let mut e2 = e1.clone();
        e2.alerts.push(AlertInstance {
            fingerprint: "fp2".into(),
            status: AlertStatus::Firing,
            labels: Labels::new(),
            annotations: Annotations::new(),
            starts_at: None,
            ends_at: None,
            generator_url: None,
        });
        assert_eq!(classify(Some(&group), &e2), EventKind::Updated);

        // 全部恢复 → Resolved
        let p = payload(json!({
            "version":"4","groupKey":"g","status":"resolved",
            "alerts":[{"status":"resolved","labels":{"alertname":"HighCPU"},"fingerprint":"fp1"}]
        }));
        let e3 = NormalizedEvent::from_payload(&p).unwrap();
        assert_eq!(classify(Some(&group), &e3), EventKind::Resolved);

        // 无历史 + firing → NewFiring
        assert_eq!(classify(None, &e1), EventKind::NewFiring);
    }

    #[test]
    fn reopen_after_resolved_is_new_firing() {
        let mut e = NormalizedEvent::from_payload(&firing_payload("HighCPU")).unwrap();
        let now = crate::now();
        let mut group = AlertGroup::new(HookId::new(), ChannelId::new(), &e, now);
        e.status = GroupStatus::Resolved;
        group.status = GroupStatus::Resolved;
        group.resolved_at = Some(now);
        group.alerts = vec![];
        // 重新 firing
        let firing_again = NormalizedEvent::from_payload(&firing_payload("HighCPU")).unwrap();
        group.status = GroupStatus::Resolved;
        assert_eq!(classify(Some(&group), &firing_again), EventKind::NewFiring);
    }

    #[test]
    fn group_invariants_hold_across_events() {
        let e = NormalizedEvent::from_payload(&firing_payload("HighCPU")).unwrap();
        let now = crate::now();
        let mut g = AlertGroup::new(HookId::new(), ChannelId::new(), &e, now);
        assert!(g.invariants_hold());
        assert_eq!(g.firing_count, 1);

        let mut resolved = e.clone();
        resolved.status = GroupStatus::Resolved;
        resolved
            .alerts
            .iter_mut()
            .for_each(|a| a.status = AlertStatus::Resolved);
        g.apply_event(&resolved, "h", now);
        assert!(g.invariants_hold());
        assert!(g.resolved_at.is_some());
        assert_eq!(g.firing_count, 0);
    }

    #[test]
    fn decision_table_matches_spec() {
        use crate::policy::NotificationPolicy;
        let mut p = NotificationPolicy::default_for(ChannelId::new());

        assert_eq!(
            decide(EventKind::NewFiring, &p, false),
            DeliveryAction::SendNew
        );

        p.repeat_policy = RepeatPolicy::Suppress;
        assert_eq!(
            decide(EventKind::Repeated, &p, true),
            DeliveryAction::Suppress
        );
        p.repeat_policy = RepeatPolicy::Send;
        assert_eq!(
            decide(EventKind::Repeated, &p, true),
            DeliveryAction::SendNew
        );
        p.repeat_policy = RepeatPolicy::SendAndRevokePrevious;
        assert_eq!(
            decide(EventKind::Repeated, &p, true),
            DeliveryAction::SendNewThenRevoke
        );

        p.group_update_policy = GroupUpdatePolicy::UpdatePrevious;
        assert_eq!(
            decide(EventKind::Updated, &p, true),
            DeliveryAction::UpdateInPlace
        );
        p.group_update_policy = GroupUpdatePolicy::Suppress;
        assert_eq!(
            decide(EventKind::Updated, &p, true),
            DeliveryAction::Suppress
        );

        p.resolved_policy = ResolvedPolicy::UpdatePrevious;
        assert_eq!(
            decide(EventKind::Resolved, &p, true),
            DeliveryAction::UpdateInPlace
        );
        assert_eq!(
            decide(EventKind::Resolved, &p, false),
            DeliveryAction::Suppress
        );
        p.resolved_policy = ResolvedPolicy::SendAndDeletePrevious;
        assert_eq!(
            decide(EventKind::Resolved, &p, true),
            DeliveryAction::SendNewThenRevoke
        );
        p.resolved_policy = ResolvedPolicy::Send;
        assert_eq!(
            decide(EventKind::Resolved, &p, false),
            DeliveryAction::SendNew
        );
    }

    #[test]
    fn volatile_fields_do_not_flip_repeat_to_update() {
        let e1 = NormalizedEvent::from_payload(&firing_payload("HighCPU")).unwrap();
        let now = crate::now();
        let mut group = AlertGroup::new(HookId::new(), ChannelId::new(), &e1, now);

        // 仅注解/时间变化不应触发 Updated
        let mut e2 = e1.clone();
        e2.alerts[0]
            .annotations
            .insert("note".into(), "changed".into());
        e2.common_annotations
            .insert("note".into(), "changed".into());
        group.apply_event(&e1, "h", now);
        assert_eq!(classify(Some(&group), &e2), EventKind::Repeated);
    }

    #[test]
    fn event_hash_is_stable_and_content_sensitive() {
        let hook = HookId::new();
        let e1 = NormalizedEvent::from_payload(&firing_payload("HighCPU")).unwrap();
        let e2 = NormalizedEvent::from_payload(&firing_payload("HighCPU")).unwrap();
        assert_eq!(event_hash(&hook, &e1), event_hash(&hook, &e2));
        let e3 = NormalizedEvent::from_payload(&firing_payload("Other")).unwrap();
        assert_ne!(event_hash(&hook, &e1), event_hash(&hook, &e3));
    }

    #[test]
    fn delivery_state_transitions() {
        let now = crate::now();
        let mut d = Delivery::new(
            AlertGroupId::new(),
            ChannelId::new(),
            DeliveryKind::FiringCreated,
            None,
            "digest",
            now,
        );
        assert_eq!(d.state, DeliveryState::Pending);
        d.mark_active(now);
        assert!(d.is_active());
        d.mark_superseded(now);
        assert_eq!(d.state, DeliveryState::Superseded);
        assert_eq!(d.next_sequence(), 1);
        assert_eq!(d.next_sequence(), 2);
    }

    #[test]
    fn unknown_alert_resolved_is_suppressed() {
        use crate::policy::NotificationPolicy;
        let mut p = NotificationPolicy::default_for(ChannelId::new());
        p.resolved_policy = ResolvedPolicy::UpdatePrevious;
        assert_eq!(
            decide(EventKind::Resolved, &p, false),
            DeliveryAction::Suppress
        );
    }
}
