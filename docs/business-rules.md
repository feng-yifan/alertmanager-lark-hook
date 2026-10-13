# 业务规则（投递决策表）

本文把需求中的口语描述收敛为可执行规则。术语见 `domain-model.md` 开头的映射表。

## 1. 接收流水线（Hook Intake Pipeline）

```
POST /api/v1/hooks/alertmanager/{token}
  1. 校验 token（常量时间比较哈希）→ 否则 404
  2. Hook / Channel 是否 enabled                → 否：200 + audit(Skipped:disabled)
  3. 解析 AlertManager v4 payload，校验必填字段 → 失败：400
  4. 标签过滤 LabelMatcher 全不命中              → 200 + audit(Skipped:filtered)
  5. 静默期 mute_until 未过期                    → 200 + audit(Skipped:muted)
      （状态仍然推进：AlertGroup 快照/状态照常更新，只抑制发送）
  6. 幂等去重 event_hash                         → 命中：200 + audit(Skipped:duplicate)
  7. 事件分类（NewFiring / Repeated / Updated / Resolved）
  8. 决策表 → 动作（SendNew / UpdateInPlace / Revoke / Suppress）
  9. 聚合窗口 / 最小发送间隔（必要时延迟或合并）
 10. 渲染卡片 → 飞书 API → 落库 Delivery
 11. 200（只要状态已持久化成功；发送失败交给后台重试）
```

**HTTP 返回约定**

| 场景 | 状态码 | 说明 |
|---|---|---|
| 已受理（含被过滤/静默/去重跳过） | `200` | 响应体 `{"ok":true,"action":"...","skipped":"..."}` |
| payload 非法 / 缺失 alerts | `400` | AlertManager 视为不可重试 |
| token 未知或 Hook 已删除 | `404` | |
| 数据库写失败等内部错误 | `500` | AlertManager 会退避重试 |
| 渠道 enabled=false | `200` + `skipped: channel_disabled` | 避免无意义重试风暴 |

> **`ok:true` 不等于"消息已发出"。** 发送失败时状态已落库为 `pending`（待后台重试）或
> `failed`，响应中的 `delivery_state` 会如实反映。指标同样以 `delivery_state` 为准，
> 避免"决策为 send_new"被误统计成"发送成功"。

## 2. 输入契约（AlertManager v4）

```jsonc
{
  "version": "4",                 // 必须存在；未知版本仅告警不拒绝
  "groupKey": "{}:{alertname=\"HighCPU\"}",
  "truncatedAlerts": 0,
  "status": "firing",             // firing | resolved
  "receiver": "lark-ops",
  "groupLabels":       { "alertname": "HighCPU" },
  "commonLabels":      { "severity": "critical", "job": "node" },
  "commonAnnotations": { "summary": "CPU 过高" },
  "externalURL": "http://alertmanager:9093",
  "alerts": [{
    "status": "firing",           // firing | resolved
    "labels": { "alertname": "HighCPU", "instance": "10.0.0.1:9100" },
    "annotations": { "summary": "...", "description": "..." },
    "startsAt": "2026-10-13T00:00:00Z",
    "endsAt": "0001-01-01T00:00:00Z",   // 零值 = 未恢复
    "generatorURL": "http://prometheus:9090/graph?...",
    "fingerprint": "a1b2c3d4e5f6a7b8"
  }]
}
```

规则：
- `alerts` 为空数组时视为心跳，不投递，仅审计。
- 分组状态由子告警决定，**不信任** payload 顶层的 `status`（两者不一致时以子告警集合为准，
  并以 `status` 作为兜底）。
- `endsAt` 为零值（`0001-01-01T00:00:00Z`）表示未恢复。

## 3. 事件分类

| 分类 | 条件 | 含义 |
|---|---|---|
| `NewFiring` | 无历史分组，或历史为 `Resolved` 后再次出现活跃告警 | 新告警分组（重开） |
| `Repeated` | 存在 `Firing` 分组，且活跃 fingerprint 集合与内容哈希均未变化 | 重复通知（repeat_interval） |
| `Updated` | 存在 `Firing` 分组，活跃集合变化但仍 `firing` | 新增告警 / 部分恢复 |
| `Resolved` | 活跃集合为空 | 全部恢复 |

## 4. 投递决策表

| 事件 | 策略字段 | 取值 | 动作 |
|---|---|---|---|
| `NewFiring` | （无开关，始终投递） | — | **SendNew**：创建卡片实体 → 发送 → Delivery=Active |
| `Repeated` | `repeat_policy` | `send` | **SendNew**，旧 Delivery → `Superseded` |
| | | `suppress`（默认） | **Suppress**：不发送，仅更新 `last_seen_at` |
| | | `send_and_revoke_previous` | **SendNewThenRevoke**：发送新消息后撤回旧消息，旧 → `Recalled` |
| `Updated` | `group_update_policy` | `update_previous`（默认） | **UpdateInPlace**：原地更新当前 Active 卡片内容 |
| | | `send` | **SendNew**，旧 → `Superseded` |
| | | `suppress` | **Suppress** |
| `Resolved` | `resolved_policy` | `send`（默认） | **SendNew**（恢复卡片），旧 → `Superseded` |
| | | `send_and_delete_previous` | **SendNewThenRevoke**，旧 → `Recalled` |
| | | `update_previous` | **UpdateInPlace**：把旧卡片原地更新为已恢复，旧 → `Resolved` |

补充规则：

- **R1 卡片实体不可用时降级**：`UpdateInPlace` 需要 `delivery.card_entity_id`（或应用消息流
  `app_feed_biz_id`）仍然有效。若不存在、已撤回或创建超过 14 天（CardKit 实体有效期），
  自动降级为 **SendNew** 并记录 `fallback=update_unavailable`；不返回错误。
- **R2 撤回失败不阻塞**：撤回/删除失败（超出撤回时限、消息已被删、权限不足）时
  Delivery 标记 `Recalled`→`RecallFailed` 交由审计，主流程仍返回 200。
- **R3 内容未变化则不重复更新**：`UpdateInPlace` 前比较 `rendered_digest`，
  相同则跳过更新调用（幂等，避免飞书限频与无意义的 `update_time` 变化）。
- **R4 置顶（pin）**：`pin_unresolved = true` 时，**仅当**本次消息包含至少一条 firing 告警，
  对**应用消息流卡片**设置 `time_sensitive = true`；恢复消息始终 `false`。
  卡片消息（`im/v1/messages`）无对应官方字段，此开关对卡片消息记为 no-op，
  并在管理后台显式提示。
- **R5 静默期**：`mute_until` 未过期时抑制所有发送类动作，但事件分类与分组状态照常推进
  （即解除静默后不会补发历史消息，只反映最新状态）。
- **R6 聚合窗口**：`aggregation_window_secs > 0` 时，同一 `(hook, group_key)` 在窗口内的事件
  合并为一次决策，采用窗口内**最后一个事件的整份快照**（last-wins，不做并集）。
  刻意不取并集：AlertManager 通知成功后会从分组里删除已 resolved 的告警，
  因此「最后一个快照」才是它眼中的权威状态，取并集反而会把已消失的告警带回来。
  窗口内无新事件时冲刷。
  已知限制：桶只在进程内存中，冲刷失败或进程重启会**丢失**该事件（此时 200 已返回、
  幂等键也已写入）。
- **R7 最小发送间隔**：`min_send_interval_secs > 0`，同一渠道两次发送的最小间隔；未满足时
  本次动作延迟到窗口末端合并执行（不丢弃）。
- **R8 告警数量截断**：超过 `max_alerts_per_message` 的告警折叠为 `+N more`；
  同时透传 AlertManager 的 `truncatedAlerts`（`agent` 侧已截断时提示总数为下界）。
- **R9 幂等**：`event_hash = sha256(hook_id | groupKey | status | sorted(fingerprint:status:startsAt))`。
  去重是**原子认领 + 失败归还**（`EventDedup::claim` / `release`）：认领发生在处理**之前**
  （否则并发的相同请求会同时通过）；窗口固定 **60 秒**（`DEFAULT_DEDUP_WINDOW_SECS`，暂不可配）；
  **处理失败（5xx）时必须归还认领** —— AlertManager 会在秒级退避重试，若不归还，
  重试会被判成 `duplicate` 并收到 200，AM 认为投递成功，这条告警**永久丢失**。
  发送时附飞书 `uuid = "alkh:" + event_hash`，飞书侧 1 小时内至多成功一条，双保险。
- **R10 重开（reopen）**：分组恢复后再次 firing，视为 `NewFiring`，发送全新消息；
  此前的恢复消息保持不动（`Superseded`）。
- **R11 标签过滤语义**：多个 matcher 为 AND；matcher 集合为空表示全部通过。
  匹配对象为**每条告警的 labels ∪ commonLabels**（告警自身 label 优先）。
  分组内只要**至少一条告警**命中即投递，未命中的告警从消息中剔除。
  若过滤后为空 → `skipped:filtered`。
- **R12 模板缺失**：渠道策略引用的模板被删或未配置 → 拒绝保存策略；运行时若模板失效则
  使用内置默认模板并记录 `fallback=default_template`，保证告警不丢。
- **R13 渠道禁用**：`channel.enabled=false` 或 `hook.enabled=false` → 不发送，仅审计。
- **R14 失败重试**：飞书调用失败按**固定档位阶梯**重试（1s / 5s / 30s / 2m / 10m，共 5 次；不是指数退避，比例并不固定）；
  限频错误（`230020` / HTTP 429）读取 `Retry-After` 优先；`MAX_ATTEMPTS` 为「1 次首发 + 5 次重试」，档位用尽后才置 `Failed`；
  审计并计入指标，可在管理后台手动重试。

## 4.1 飞书 API 约束带来的硬性规则

- **R15 应用消息流卡片只能发给用户**：`im/v2/app_feed_card` 的接收者是 `user_ids`（1–20），
  没有群维度。因此 `message_type = app_feed_card` 的渠道必须是用户级渠道
  （`receive_id_type ∈ {open_id, user_id}`）。群聊渠道选用应用消息流卡片时，策略保存即被拒绝。
- **R16 卡片实体只能发送一次**（官方明示）：`cardkit/v1/cards` 创建的 `card_id` 仅能用于一次
  `im/v1/messages` 发送，实体有效期 14 天。因此：
  - `SendNew` → 每次重新创建卡片实体；
  - `UpdateInPlace` → 复用已有实体的 `card_id` 调 `PUT /cardkit/v1/cards/:card_id` 全量更新，
    且 `sequence` 必须相比上一次操作**严格递增**（`Delivery.card_sequence` 自增）；
  - 卡片来源为 `builder_template` 时，没有卡片实体，原地更新走
    `PATCH /im/v1/messages/:message_id`，`content = {"type":"template","data":{...}}`；
  - 两条更新路径均受"仅支持更新 14 天内发送的消息"限制；超期即触发 R1 降级为 `SendNew`。
- **R17 卡片必须是 `update_multi = true`**：原地更新要求更新前后 `config.update_multi` 均为
  `true`。模板校验阶段强制注入/校验该字段，否则拒绝保存（否则后续"更新旧告警"必然失败）。
- **R18 应用消息流卡片更新字段**：更新走 `PUT /im/v2/app_feed_card/batch`，`update_fields` 取值
  `1`(标题) `3`(预览) `10`(状态标签) `11`(按钮) `12`(跳转链接) `13`(即时提醒) `103`(进行通知)；
  删除走 `DELETE /im/v2/app_feed_card/batch`，以 `{biz_id, user_id}` 定位。

## 5. 变量与渲染

卡片模板中的 `${var}` 由**渲染上下文**替换。实现见
[`crates/alkh-app/src/render.rs`](../crates/alkh-app/src/render.rs) 的 `build_context`
与 `crates/alkh-app/src/meta.rs` 的 `render_variables()`（后者是变量清单的**唯一事实来源**，
同时通过 `GET /api/v1/meta` 提供给管理后台）。

> **变量清单请勿在文档里另抄一份**——用户向速查表在
> [README §3.3](../README.md#33-变量速查表)，并有单测
> `alkh_app::meta::tests::readme_documents_every_variable` 保证它与代码同步。
> 这里只描述**渲染规则**。

渲染规则：

- 变量替换发生在**字符串叶子节点**，不改变 JSON 结构；渲染后校验 JSON 仍合法。
- 支持 `${a.b.c}` 点号取值与 `${label:foo}` / `${annotation:bar}` 显式取值。
- **任意标签 / 注解可直接用原名引用**；与内置变量同名时内置优先（内置先写入上下文），
  此时必须用 `${label:x}` 拿到标签原值。这是取舍而非缺陷，但**新增内置变量名时要注意
  不要侵占常用的标签名**。
- 未知变量渲染为空串，并在 `validate` 接口以 warning 返回（不阻塞保存）。
- 代表告警语义：`starts_at` / `ends_at` / `summary` / `description` / `instance` /
  `fingerprint` / `generator_url` / `alert_status` 取自**一条代表告警**（优先第一条
  firing，否则第一条），而不是整组。这与 AlertManager 中「必须写在 `{{ range .Alerts }}`
  内」的语义不同，是「一张卡片装一组告警」的必要扁平化。
- 时间变量统一为 `%Y-%m-%d %H:%M:%S UTC` 字符串。
- 严重级别映射：`critical|fatal|emergency`→`red`/`严重`，`warning|warn|major`→`orange`/`警告`，
  `info|informational|notice`→`blue`/`提示`，其余→`grey`/`未知`。

命名沿革（避免误解）：本项目用 `${...}` 而非 AlertManager 的 Go 模板 `{{ ... }}`；
`${...}` 也**不是**飞书官方文档化的语法（官方只有搭建工具的 `template_variable` 键值映射）。
应用消息流卡片的字段长度上限在渲染时**截断**而非报错以保证告警不丢。

## 6. 校验规则汇总（管理后台输入校验）

| 对象 | 规则 |
|---|---|
| 渠道 | 名称 ≤ 64；`receive_id` 非空；同 `(type,id)` 唯一 |
| Hook | 名称 ≤ 64；token 轮换后旧的立即失效 |
| 卡片模板（卡片消息） | `schema` 必须 `"2.0"`；JSON 合法；组件数 ≤ 200；体积 ≤ 30KB（发送前预估） |
| 卡片模板（应用消息流） | title ≤ 60；preview ≤ 120；status_label.text ≤ 20；按钮 ≤ 2、文案 ≤ 30；link 必须 `https` |
| 通知策略 | 正则 matcher 可编译；聚合窗口 0–3600s；最小间隔 0–3600s；告警上限 1–200 |
| 应用设置 | app_id 形如 `cli_*`（仅提示）；app_secret 写入后不再回显 |

## 7. 与需求条目的对应关系

| 需求 | 落地 |
|---|---|
| 1. 导入卡片搭建工具创建的卡片 | `CardTemplate.source = builder_json`（导入 JSON 2.0）或 `builder_template`（导入 template_id + 版本），含变量识别与预览 |
| 2. 通过飞书机器人发送消息 | `LarkGateway` 使用 `tenant_access_token` 调用 `im/v1/messages` 与 `im/v2/app_feed_card` |
| 3. API 读取可接收消息的渠道 + 每渠道独立 hook | `im/v1/chats` 同步 → Channel；每个 Channel 签发独立 Hook token/URL，可多个 |
| 4. 发送规则配置 | 第 4 节决策表 + NotificationPolicy 字段 |
| 5. Rust + TS 管理后台 + 容器化 | `crates/*` + `web/*` + `deploy/*` |
| 6. 嵌入式 SQLite | `sqlx` bundled + `migrations/` |
| 7. API 均出自开放平台文档 | `docs/research/feishu-api.md` 记录逐个端点与出处 |
| 8. 补齐业务细节 | R1–R14、审计、指标、保留策略、重试、幂等、静默、限流 |
