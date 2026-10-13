# 领域模型

术语统一（把需求中的口语词映射为标准术语，全文以此为准）：

| 需求用语 | 标准术语 | 说明 |
|---|---|---|
| 未解决告警 | **firing alert** | AlertManager 中 `status = "firing"` 的告警 |
| 解决告警 | **resolved alert** | AlertManager 中 `status = "resolved"` 的告警 |
| 重复告警 | **repeated notification**（重复通知） | AlertManager 按 `repeat_interval` 对同一分组再次投递 |
| 发送并移除旧告警 | **send + revoke previous**（撤回旧消息） | 撤回/删除上一条已发送消息 |
| 更新旧告警 | **update previous in place**（原地更新卡片） | 通过 CardKit `card_id` 全量更新（或 `PATCH /im/v1/messages/:id`）|
| 消息渠道 | **channel** | 机器人可触达的群聊（`chat_id`）或单聊用户（`open_id`） |
| 卡片消息 | **card message**（交互式卡片消息） | `im/v1/messages` + `msg_type=interactive` |

## 1. 聚合与实体总览

```
AlertGroup (聚合根) 1 ──── * Delivery (子实体)
     │  group_key 标识，归属一个 Hook(=渠道)
     │
Channel (聚合根) 1 ──── 0..1 NotificationPolicy (聚合内实体/值对象集)
     │
     └── 1 ──── * Hook (接入端点，聚合根，含密钥)
                  每个 Hook 对应一个 AlertManager receiver

CardTemplate (聚合根)
AppSettings (聚合根, 单例)
AdminUser (聚合根) 1 ──── * Session (子实体)
AuditLog (只追加记录)
```

## 2. Channel（渠道，聚合根）

```rust
pub struct Channel {
    pub id: ChannelId,
    pub name: String,                  // 展示名，可本地覆盖飞书群名
    pub receive_id: String,            // chat_id / open_id / user_id / email
    pub receive_id_type: ReceiveIdType,// ChatId | OpenId | UserId | Email
    pub kind: ChannelKind,             // Group | P2PUser
    pub avatar_url: Option<String>,
    pub external: bool,                // 外部群
    pub bot_can_send: bool,            // 最近一次探测结果
    pub enabled: bool,
    pub source: ChannelSource,         // FeishuSync | Manual
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
```

不变量：
- `receive_id` 非空；`receive_id_type == ChatId` ⇒ `kind == Group`。
- `enabled == false` ⇒ 该渠道下所有 Hook 的投递被拒（但仍记录审计）。
- 同 `(receive_id_type, receive_id)` 唯一。

## 3. Hook（接入端点，聚合根）

```rust
pub struct Hook {
    pub id: HookId,
    pub channel_id: ChannelId,
    pub name: String,             // 便于在 AlertManager 配置里区分
    pub token_hash: String,       // 鉴权用；明文 token 不再「只返回一次」
    pub token_prefix: String,     // 便于识别，如 "alkh_a1b2"
    pub token_enc: Option<EncryptedSecret>,  // token 密文，供后台随时回显完整 Hook 地址
    pub enabled: bool,
    pub last_called_at: Option<Timestamp>,
    pub call_count: u64,
    pub created_at: Timestamp,
    pub rotated_at: Option<Timestamp>,
}
```

不变量：
- 一个渠道可以有多个 Hook（例如区分 `severity` 不同的 receiver），但每个 Hook 只有一个渠道。
- `token` 明文 = `alkh_` + 32 字节 CSPRNG 的 base62；鉴权只比对 SHA-256 哈希。
- Hook 地址**不是**需要保密的秘密：token 以 AES-256-GCM 密文随 Hook 落库，后台可随时回显、
  复制完整地址，泄露后轮换即可（轮换先清空密文，避免展示已失效的旧地址）。
- Hook URL = `{public_base_url}/api/v1/hooks/alertmanager/{token}`；`public_base_url` 可填集群内
  DNS，与浏览器访问地址无关。

## 4. CardTemplate（卡片模板，聚合根）

只有**一种**形态：卡片模板（本地 CardKit JSON 2.0）。

```rust
pub struct CardTemplate {
    pub id: CardTemplateId,
    pub name: String,
    pub card_message: CardMessageSpec,
    pub is_default: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub struct CardMessageSpec {
    pub source: CardSource,                   // BuilderJson | BuilderTemplate
    pub card_json: Option<serde_json::Value>, // source=BuilderJson：卡片 JSON 2.0
    pub template_id: Option<String>,          // source=BuilderTemplate：搭建工具模板 ID（已停用）
    pub template_version_name: Option<String>,
    pub variables: Vec<TemplateVariable>,     // 声明的变量（名称/类型/默认值/内置来源）
}
```

不变量：
- `card_json` 必须 `schema == "2.0"`，且 `config.update_multi` 强制为 `true`（CardKit 原地更新的前提）。
- 组件结构必须通过**组件目录**校验（`card_catalog`：废弃组件、`column` 越级、`element_id` 重复等）。
- `source = BuilderTemplate` 已停用：新建/更新返回 `400`，历史模板只读可预览。
- 全局至多一个 `is_default = true`。
- 删除被策略引用的模板 → 拒绝（`TemplateInUse`）。

## 5. NotificationPolicy（通知策略，Channel 聚合内实体）

```rust
pub struct NotificationPolicy {
    pub channel_id: ChannelId,
    pub card_template_id: Option<CardTemplateId>,     // 空时回退内置默认卡片（R12）
    pub pin_unresolved: bool,                          // 未解决时 Pin 当前卡片到 Pin 列表（R4，默认关闭）
    pub repeat_policy: RepeatPolicy,                   // 重复告警
    pub resolved_policy: ResolvedPolicy,               // 解决告警
    pub group_update_policy: GroupUpdatePolicy,        // 分组内容变化（新增/部分恢复）
    pub label_matchers: Vec<LabelMatcher>,             // 只投递命中的告警
    pub aggregation_window_secs: u32,                  // 0 = 关闭
    pub min_send_interval_secs: u32,                   // 0 = 关闭
    pub max_alerts_per_message: u32,                   // 超出折叠为 “+N more”
    pub mute_until: Option<Timestamp>,                 // 静默期，到点自动解除
    pub updated_at: Timestamp,
}

pub enum RepeatPolicy {
    Send,                     // 发送：重复通知照发
    Suppress,                 // 不发送（默认）
    SendAndRevokePrevious,    // 发送并移除旧告警
}

pub enum ResolvedPolicy {
    Send,                     // 发送恢复通知（默认）
    SendAndDeletePrevious,    // 发送并删除旧告警
    UpdatePrevious,           // 更新旧告警（原地更新为已恢复）
}

pub enum GroupUpdatePolicy {
    UpdatePrevious,           // 默认：原地更新
    Send,
    Suppress,
}

pub struct LabelMatcher { pub name: String, pub op: MatchOp, pub value: String }
pub enum MatchOp { Eq, Neq, Regex, NotRegex }   // = != =~ !~
```

不变量：
- `card_template_id` 非空时，保存前校验引用的模板存在（R12）。
- `LabelMatcher` 的正则在保存时编译校验，非法正则拒绝。
- `mute_until` 过期后读取时视为 `None`。

## 6. AlertGroup（告警分组，聚合根）

以 AlertManager 的 `groupKey` 为自然标识，按 `(hook_id, group_key)` 唯一。

```rust
pub struct AlertGroup {
    pub id: AlertGroupId,
    pub hook_id: HookId,
    pub channel_id: ChannelId,
    pub group_key: String,              // AlertManager groupKey
    pub receiver: String,
    pub status: GroupStatus,            // Firing | Resolved
    pub alertname: Option<String>,
    pub severity: Option<String>,
    pub group_labels: Labels,
    pub common_labels: Labels,
    pub common_annotations: Annotations,
    pub alerts: Vec<AlertInstance>,     // 最近一次投递的告警快照
    pub firing_count: u32,
    pub resolved_count: u32,
    pub truncated_alerts: u64,
    pub event_count: u64,               // 累计接收事件数
    pub last_event_hash: Option<String>,
    pub first_seen_at: Timestamp,
    pub last_seen_at: Timestamp,
    pub last_notified_at: Option<Timestamp>,
    pub resolved_at: Option<Timestamp>,
    pub active_delivery_id: Option<DeliveryId>, // 当前"活"的消息，供更新/撤回定位
    pub pinned_message_id: Option<String>,      // 当前被 Pin 的卡片消息（R4）
}

pub struct AlertInstance {
    pub fingerprint: String,
    pub status: AlertStatus,           // Firing | Resolved
    pub labels: Labels,
    pub annotations: Annotations,
    pub starts_at: Timestamp,
    pub ends_at: Option<Timestamp>,
    pub generator_url: Option<String>,
}
```

### 6.1 状态机

```
            ┌───────────────────────────┐
            │                           │  新告警再次出现
            ▼                           │  (REOPENED → 视为 NEW_FIRING)
     ┌────────────┐  全部恢复   ┌──────────────┐
     │  Firing    │────────────▶│  Resolved    │
     └─────┬──────┘             └──────────────┘
           │ 重复通知 / 分组更新（状态不变，仅计数与快照变化）
           └──────────────▶ Firing
```

不变量：
- `status == Resolved` ⇔ `firing_count == 0` 且 `alerts` 中每条都是 `Resolved`。
- `resolved_at` 非空 ⇔ `status == Resolved`。
- `active_delivery_id` 必须指向本分组自己的 Delivery。
- `pinned_message_id` 非空 ⇔ 该分组当前有一条被 Pin 的卡片，且它由本服务在
  `pin_unresolved` 开启时 Pin；分组恢复或开关关闭后必须清空（R4）。

### 6.2 事件分类（领域服务 `EventClassifier`）

输入一次 hook 请求 + 已存在的 AlertGroup（可能不存在）→ 输出事件类型：

| 分类 | 条件 |
|---|---|
| `NewFiring` | 分组不存在，或分组为 `Resolved` 后重新 `firing` |
| `Repeated` | 分组存在且 `Firing`，活跃 fingerprint 集合与内容哈希**均未变化** |
| `Updated` | 分组存在且 `Firing`，活跃集合变化但仍为 `firing`（新增告警 / 部分恢复） |
| `Resolved` | 本次 `status = resolved` 且活跃集合为空 |

`content_hash = sha256(sorted(fingerprint:status) + 活跃告警 labels 的稳定序列化)`。

## 7. Delivery（投递记录，AlertGroup 子实体）

```rust
pub struct Delivery {
    pub id: DeliveryId,
    pub alert_group_id: AlertGroupId,
    pub channel_id: ChannelId,
    pub kind: DeliveryKind,        // FiringCreated | Repeated | Updated | Resolved | Test
    pub state: DeliveryState,
    pub card_template_id: Option<CardTemplateId>,
    pub lark_message_id: Option<String>,   // om_xxx
    pub card_entity_id: Option<String>,    // CardKit card_id（用于原地更新）
    pub card_sequence: i64,                // CardKit 卡片实体的更新序号（严格递增，见 Adapter 说明）
    pub rendered_digest: String,           // 渲染后卡片内容摘要，用于判断"是否需要真的更新"
    pub attempt: u32,
    pub last_error: Option<String>,        // 含飞书 error code / log_id
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub enum DeliveryState {
    Pending,     // 已决策待发送
    Active,      // 已发送且为当前"活"消息
    Superseded,  // 被新消息取代（旧消息保留在会话里）
    Recalled,    // 已撤回/删除
    Resolved,    // 已被原地更新为已恢复
    Failed,      // 重试耗尽
}
```

状态迁移：

```
Pending ──send ok──▶ Active ──新消息取代──▶ Superseded
   │                   │  ├──撤回/删除成功──▶ Recalled
   │                   │  └──原地更新为恢复──▶ Resolved
   └──重试耗尽────────▶ Failed ──手动重试──▶ Pending
```

## 8. AppSettings（应用设置，单例聚合根）

```rust
pub struct AppSettings {
    pub lark_app_id: Option<String>,
    pub lark_app_secret_enc: Option<EncryptedSecret>, // AES-256-GCM
    pub lark_base_url: String,
    pub public_base_url: Option<String>,
    pub retention: RetentionPolicy,   // alert_group_days / delivery_days / audit_days
    pub updated_at: Timestamp,
}
```

## 9. AdminUser / Session

```rust
pub struct AdminUser {
    pub id: AdminUserId,
    pub username: String,
    pub password_hash: String,     // Argon2id
    pub must_change_password: bool,
    pub created_at: Timestamp,
    pub last_login_at: Option<Timestamp>,
}

pub struct Session {
    pub id: SessionId,
    pub user_id: AdminUserId,
    pub token_hash: String,        // 会话 token 只存哈希
    pub expires_at: Timestamp,
    pub created_at: Timestamp,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
}
```

## 10. AuditLog（审计，只追加）

```rust
pub struct AuditLog {
    pub id: i64,
    pub at: Timestamp,
    pub actor: AuditActor,        // Admin{user} | AlertManager{hook} | System{job}
    pub action: String,           // e.g. "hook.received", "message.sent", "channel.sync"
    pub target: Option<String>,
    pub outcome: Outcome,         // Ok | Skipped{reason} | Failed{error}
    pub detail: serde_json::Value,
}
```

## 11. 领域事件（进程内，供审计 / 指标 / 后续扩展）

`HookReceived`、`EventClassified`、`MessageSent`、`MessageRecalled`、`MessageUpdated`、
`DeliveryFailed`、`ChannelSynced`、`ChannelDisabled`、`PolicyChanged`。

## 12. 数据库表（SQLite）

```sql
channels(id PK, name, receive_id, receive_id_type, kind, avatar_url, external,
         bot_can_send, enabled, source, created_at, updated_at,
         UNIQUE(receive_id_type, receive_id))

hooks(id PK, channel_id FK, name, token_hash UNIQUE, token_prefix,
      token_nonce, token_ciphertext, enabled,
      last_called_at, call_count, created_at, rotated_at)

card_templates(id PK, name, card_message JSON, is_default,
               created_at, updated_at)

notification_policies(channel_id PK FK, card_template_id, pin_unresolved,
                      repeat_policy, resolved_policy,
                      group_update_policy, label_matchers,
                      aggregation_window_secs, min_send_interval_secs,
                      max_alerts_per_message, mute_until, updated_at)

alert_groups(id PK, hook_id FK, channel_id FK, group_key, receiver, status,
             alertname, severity, group_labels, common_labels,
             common_annotations, alerts, firing_count, resolved_count,
             truncated_alerts, event_count, last_event_hash,
             first_seen_at, last_seen_at, last_notified_at, resolved_at,
             active_delivery_id, pinned_message_id, UNIQUE(hook_id, group_key))
  + INDEX(status), INDEX(channel_id, last_seen_at)

deliveries(id PK, alert_group_id FK, channel_id FK, kind, state,
           card_template_id, lark_message_id, card_entity_id, card_sequence,
           rendered_digest, attempt, last_error, created_at, updated_at)
  + INDEX(alert_group_id), INDEX(state)

app_settings(id PK CHECK(id=1), ...)
admin_users(id PK, username UNIQUE, password_hash, must_change_password,
            created_at, last_login_at)
sessions(id PK, user_id FK, token_hash UNIQUE, expires_at, created_at,
         user_agent, ip)
audit_logs(id INTEGER PK AUTOINCREMENT, at, actor, action, target, outcome, detail)
  + INDEX(at), INDEX(action)

event_dedup(event_hash PK, hook_id, seen_at)   -- 幂等去重窗口
```

JSON 列（`group_labels`、`alerts`、`card_message`、`label_matchers`、`detail`）以 `TEXT` 存储
序列化 JSON；仓储层负责编解码，领域层只见值对象。
