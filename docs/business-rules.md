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

- **R1 卡片实体不可用时降级**：`UpdateInPlace` 需要 `delivery.card_entity_id`（或 `lark_message_id`）仍然有效。若不存在、已撤回或创建超过 14 天（CardKit 实体有效期），
  自动降级为 **SendNew** 并记录 `fallback=update_unavailable`；不返回错误。
- **R2 撤回失败不阻塞**：撤回/删除失败（超出撤回时限、消息已被删、权限不足）时
  Delivery 标记 `Recalled`→`RecallFailed` 交由审计，主流程仍返回 200。
- **R3 内容未变化则不重复更新**：`UpdateInPlace` 前比较 `rendered_digest`，
  相同则跳过更新调用（幂等，避免飞书限频与无意义的 `update_time` 变化）。
- **R4 未解决告警 Pin**（策略开关 `pin_unresolved`，默认关闭）：分组仍处于 firing 时，
  把**当前活跃卡片** Pin 到会话的 **Pin 列表**（`POST /open-apis/im/v1/pins`）；
  分组内全部告警恢复后，对同一条消息调用 `DELETE /open-apis/im/v1/pins/:message_id` 取消 Pin。
  > ⚠️ 这是飞书的 **Pin**（Pin 列表，可 Pin 多条，入口是客户端消息菜单的「Pin」/ Pin 标签页），
  > **不是**固定在会话顶部的「置顶消息 / 群置顶」——后者用客户端按钮只能置顶 1 条，且是另一套接口
  > （`im/v1/chats/:chat_id/top_notice/put_top_notice`），本项目未使用。
  > 详见 [`docs/research/feishu-api.md` §4.1](research/feishu-api.md)。
  - 目标是「当前活跃卡片」而不是固定某条：卡片被新消息取代（重复通知发送新消息、
    R1 降级、发送重试补发）时，先取消旧消息的 Pin，再 Pin 新消息。
  - 幂等：消息已被 Pin / 未 Pin / 已被撤回时，飞书都返回成功；`AlertGroup.pinned_message_id`
    记录当前 Pin 的消息，避免每次事件都重复调用，也保证关闭开关或恢复时能精确取消。
  - 失败不阻塞（与 R2 同款语义）：群设置只允许群主 / 群管理员 Pin、机器人权限不足（`230046` / `230027`）、
    单条消息 Pin/Unpin 超过 5 QPS（`230047`）等只记日志与指标，投递照常返回 200，
    状态保持不变，下一次事件会重试；取消 Pin 失败时不会继续 Pin 新消息。
  - 开关关闭后，该分组的下一个事件会取消已有 Pin（不依赖告警真正恢复）。
  - 关闭的渠道（R13）与静默期（R5）不发消息，但仍然会对齐 Pin 状态。
  - 该能力不需要额外权限：发送卡片用的 `im:message:send_as_bot` 与
    `im:message`、`im:message.pins:write_only` 三者任一即可调用 Pin / Unpin。
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

- **R15 （已废弃）**：原「应用消息流卡片只能发给用户」——该消息形态已整体移除，
  编号保留以避免与旧文档/审计日志串号。
- **R16 卡片实体只能发送一次**（官方明示）：`cardkit/v1/cards` 创建的 `card_id` 仅能用于一次
  `im/v1/messages` 发送，实体有效期 14 天。因此：
  - `SendNew` → 每次重新创建卡片实体；
  - `UpdateInPlace` → 复用已有实体的 `card_id` 调 `PUT /cardkit/v1/cards/:card_id` 全量更新，
    且 `sequence` 必须相比上一次操作**严格递增**（`Delivery.card_sequence` 自增）；
  - 卡片来源为 `builder_template` 时（**已停用，仅历史模板**），没有卡片实体，原地更新走
    `PATCH /im/v1/messages/:message_id`，`content = {"type":"template","data":{...}}`；
  - 两条更新路径均受"仅支持更新 14 天内发送的消息"限制；超期即触发 R1 降级为 `SendNew`。
- **R17 卡片必须是 `update_multi = true`**：原地更新要求更新前后 `config.update_multi` 均为
  `true`。模板校验阶段强制注入/校验该字段，否则拒绝保存（否则后续"更新旧告警"必然失败）。
- **R18 （已废弃）**：原「应用消息流卡片更新字段」。

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
  firing，否则第一条），而不是整组。**整组逐条展示请用「循环」指令 + `${item.*}`**，
  见 §5.3。
- 循环内变量：`item.*`（`item.title` / `item.alertname` / `item.summary` …）只在
  `repeat` 指令的子组件里有值；标签/注解用 `${item.label:x}` / `${item.labels.x}` 取。
- 时间变量统一为 `%Y-%m-%d %H:%M:%S UTC` 字符串。
- 严重级别映射：`critical|fatal|emergency`→`red`/`严重`，`warning|warn|major`→`orange`/`警告`，
  `info|informational|notice`→`blue`/`提示`，其余→`grey`/`未知`。
- 状态颜色映射：`status_color` 在分组仍有未解决告警时等于 `severity_color`；**全部恢复后固定为
  `green`**。内置默认卡片的标题颜色用 `${status_color}`（而不是只按严重级别取色的
  `${severity_color}`），否则 warning 卡片恢复后标题仍是橙色。

命名沿革（避免误解）：本项目用 `${...}` 而非 AlertManager 的 Go 模板 `{{ ... }}`；
`${...}` 也**不是**飞书官方文档化的语法（官方只有搭建工具的 `template_variable` 键值映射）。
卡片 JSON 在渲染后会做一次体积收口与长字符串截断，避免超过 30KB 导致整条告警被丢弃。

### 5.1 mock 数据（预览 / 测试发送，P1）

管理后台编辑卡片时用 mock 数据渲染，实现见
[`crates/alkh-app/src/mock.rs`](../crates/alkh-app/src/mock.rs)。三层语义，优先级从低到高：

1. **基线**：内置样例分组（`firing` / `resolved`），或一条**历史告警分组**（`alert_group_id`）；
2. **覆盖**：`variables` 逐项覆盖，支持 `label:` / `annotation:` 动态 key；
3. 覆盖项里的未知 key 也会写入上下文（便于试写新标签），不报错。

- mock 只影响**渲染**：历史分组只作数据源、**绝不回写**。
- **两种测试发送的落库语义不同**：
  - 卡片编辑器的「发送测试」（`POST /card-templates/send-test`、`POST /card-templates/{id}/send-test`）
    是**样式验证**：只把卡片发出去，**不写 `alert_groups` / `deliveries`**，返回的内存回执没有
    重试 / 撤回语义。可以反复发送检查样式，也不会撞 `alert_groups(hook_id, group_key)` 唯一索引。
  - 渠道页的「发送测试消息」（`POST /channels/{id}/test-message`）走**完整投递链路并落库**：
    消息写入该渠道的「测试分组」行（按自然键复用，重复发送不会冲突），投递记录为 `kind=test`，
    可重试 / 撤回；它验证的是整条链路，不只是卡片样式。
- 指定 `alert_group_id` 时以其状态为准（`firing` / `resolved`）。
- 两类测试发送都会写审计（`TEST_MESSAGE`）。
- 规模上限：`variables` ≤ 200 项、值 ≤ 4000 字符、key ≤ 128 字符。

### 5.2 本地卡片构建工具与组件目录（P0 / P2）

**不再依赖飞书搭建工具**：卡片在本地可视化构建，产出 CardKit JSON 2.0 并随模板落库。
契约与实现：

- **组件目录**是唯一事实来源，位于
  [`crates/alkh-domain/src/card_catalog.rs`](../crates/alkh-domain/src/card_catalog.rs)，
  通过 `GET /api/v1/meta` 的 `card_components` / `card_presets` 下发给管理后台。
  前端不另立规则：新增飞书组件只改后端目录。
- **文档就是 CardKit JSON 2.0 本身**，不存在"本地模型 → 编译成 JSON"这一步，
  因此画布、`/card-templates/preview`、真实投递三者必然是同一份内容。
- 硬限制对齐飞书：`schema` 必须 `"2.0"`；`config.update_multi` 强制 `true`；
  组件 ≤ 200；卡片 ≤ 30KB；`element_id` 全局唯一、字母开头、≤20 字符；
  `column` 只能位于 `column_set.columns`。
- **JSON 2.0 传入不支持的属性会直接报错**（1.0 是忽略），所以目录校验同时给出：
  - **errors（阻断保存）**：废弃组件、结构非法、`element_id` 非法/重复、超限；
  - **warnings（不阻断）**：未知组件、未知属性、`header.template` 非枚举值。
  未知组件不阻断是有意的：用户从搭建工具导入的卡片可能含 `chart` / `table` 等
  本工具未收录的组件，它们会原样保留、只在画布上标为"不可可视化编辑"。
- **告警积木**（`card_presets`）与变量清单同源，有单测保证积木只引用真实存在的变量
  （`alkh-app::meta::alert_presets_use_only_builtin_variables`）。
- **P2：停用 `builder_template`**。新建/更新一律拒绝（`400 validation`）；
  历史模板仍可读取与预览，编辑器提供「转换为本地卡片」。

### 5.3 本地指令：循环（`repeat`）与条件（`if`）

飞书 JSON 2.0 **没有**循环容器（搭建工具的循环容器不能嵌套、JSON 导入也不支持），
所以「一组告警逐条渲染」由本工具的**本地指令**实现：卡片模板里保存的是指令本身，
只有在**预览与真实发送时**才由 `alkh_app::render::expand_card_json` 展开成飞书认识的
组件（展开结果里不会残留指令 tag）。

> ⚠️ `repeat` / `if` 是 `card_catalog` 里 `group = "directive"` 的目录项，因此结构校验
> 不会把它们当未知组件。但任何**绕过 `render_card_message` 直接发送 card_json 的路径**
> 都会把指令原样发给飞书并报错。

| 指令 | 语义 |
|---|---|
| `repeat` | 对 `source`（目前只有 `alerts`）里的每一项渲染一遍 `elements`；`max_items` 是上限，`empty_content` 在数据为空时渲染一段 markdown |
| `if` | `when` 渲染后为空 / `false` / `0` / `no` / `off` 视为假（`equals` 改为精确相等，`negate` 反转）；为真才渲染 `elements` |

- 两者可任意嵌套（循环里放条件、容器里放循环），未识别的位置按普通对象递归展开。
- **循环次数**：`min(数据条数, 策略 max_alerts_per_message, max_items)`。
- **组件总数收口**：展开全程共享一个 200 的名额预算（容器也占名额），每渲染出一个
  组件就扣一个名额；名额耗尽后**按文档顺序**丢弃剩余组件。这样即使卡片里有多个循环，
  展开结果也一定是飞书能接受的组件数。取舍与体积截断一致：**宁可少显示，也不让整条
  告警因超限而丢失**；代价是靠后的组件（脚注、按钮）可能被前面的长循环挤掉，
  编辑器顶部的「展开约 N」会提示量级。
- `empty_content` 只在**一次都没有循环**（数据为空 / 策略上限为 0）时渲染；
  循环跑过但条件全部不成立时不会谎报「无告警」。
- **`element_id` 与循环**：同一段模板会被复制 N 份，模板里的 id 必然重复，而飞书对
  重复 id 返回 `300301` 并拒收整张卡片。因此：编辑器**不会给循环体内的节点分配 id**
  （拖进循环体时即剥掉，复制循环时也会清掉副本内的 id）；服务端展开循环副本时再统一
  剥一次 `element_id`（`strip_element_ids`），循环体外的 id 保留。
  这也是「循环体里写 `element_id` 没有意义」的原因——它无法按条定位。
- **体积**：渲染后再由 `shrink_json_to_bytes` 按字节截断字符串叶子，覆盖绝大多数
  超限场景（例如 200 个组件、每个 2KB 文本 → 407KB 压到 30KB）；但它的策略是
  「砍最长的字符串」，无法压缩**结构本身**就超限的 JSON，所以 30KB 不是绝对保证。
- 循环体上下文 = 分组级变量 + 当前告警的 `item.*`（`item.index` 从 1 开始，
  标签/注解可用 `${item.label:x}` / `${item.labels.x}`）。
- 兼容性：旧的 `${alerts_list}` / `${alerts_table}` 变量与「服务端拍平」逻辑**保留**，
  老模板不需要迁移；新模板与内置默认卡片改用结构化循环。


> **已修复的历史缺陷**：内置默认卡片原先使用 `note` 组件——这是 JSON 1.0 的组件，
> **2.0 已废弃**（官方「卡片 JSON 2.0 版本更新说明」的「废弃备注组件 & 交互模块」）。
> 默认卡片是"模板缺失时的兜底"，一旦发送失败就等于告警丢失。现改为普通文本组件
> （`div` + `text_size=notation` + `text_color=grey`），并加了
> `defaults::tests::default_card_uses_no_deprecated_component` 锁死。

## 6. 校验规则汇总（管理后台输入校验）

| 对象 | 规则 |
|---|---|
| 渠道 | 名称 ≤ 64；`receive_id` 非空；同 `(type,id)` 唯一 |
| Hook | 名称 ≤ 64；token 轮换后旧的立即失效；地址不做保密，列表可随时回显（密文落库） |
| 卡片模板（卡片消息） | `schema` 必须 `"2.0"`；JSON 合法；组件数 ≤ 200；体积 ≤ 30KB（发送前预估）；组件目录校验无 errors（见 §5.2） |
| 通知策略 | 正则 matcher 可编译；聚合窗口 0–3600s；最小间隔 0–3600s；告警上限 1–200 |
| mock 数据 | 变量 ≤ 200 项；单值 ≤ 4000 字符；单 key ≤ 128 字符；`alert_group_id` 必须存在（否则 404） |
| 卡片来源 | `builder_template` 已停用：新建/更新 `400 validation`（历史模板只读可预览） |
| 应用设置 | app_id 形如 `cli_*`（仅提示）；app_secret 写入后不再回显 |

## 7. 与需求条目的对应关系

| 需求 | 落地 |
|---|---|
| 1. 卡片模板 | 本地可视化/JSON 构建 `CardTemplate.card_message.source = builder_json`（CardKit JSON 2.0），含组件目录校验、变量识别、mock 预览与草稿测试发送（§5.1/§5.2）；`builder_template` 已停用 |
| 2. 通过飞书机器人发送消息 | `LarkGateway` 使用 `tenant_access_token` 调用 `im/v1/messages`（卡片实体 + `msg_type=interactive`）|
| 3. API 读取可接收消息的渠道 + 每渠道独立 hook | `im/v1/chats` 同步 → Channel；每个 Channel 签发独立 Hook token/URL，可多个 |
| 4. 发送规则配置 | 第 4 节决策表 + NotificationPolicy 字段 |
| 5. Rust + TS 管理后台 + 容器化 | `crates/*` + `web/*` + `deploy/*` |
| 6. 嵌入式 SQLite | `sqlx` bundled + `migrations/` |
| 7. API 均出自开放平台文档 | `docs/research/feishu-api.md` 记录逐个端点与出处 |
| 8. 补齐业务细节 | R1–R14、审计、指标、保留策略、重试、幂等、静默、限流 |
