# REST API 契约

- Base path：`/api/v1`
- 请求/响应：`application/json; charset=utf-8`
- 管理接口鉴权：`Cookie: alkh_session=<token>`（HttpOnly）或 `Authorization: Bearer <token>`
- 错误信封：`{"error":{"code":"<machine_code>","message":"<human>"}}`
- 分页信封：`{"items":[...],"total":123,"limit":50,"offset":0}`
- 时间：RFC 3339 UTC（`2026-10-13T00:00:00Z`）；API 一律 UTC，只有**卡片内的展示字符串**按
  「设置 → 时间展示时区」换算（见 §2）
- ID：UUID v4 字符串

> **本文档描述的是真实线格式。** 端到端测试 `crates/alkh-server/tests/e2e.rs` 会断言这些形状，
> 因此**改后端形状必须同时改 e2e 与本文件**。历史上本文件与实现漂移过一次：
> 卡片模板的字段实际嵌在 `card_message` 下、`variables` 的类型字段是 `ty`，
> 而文档写成了顶层扁平字段与 `type`，导致前端照文档实现后模板编辑器白屏、保存静默丢数据。

## 0. 公共

| 方法 | 路径 | 鉴权 | 说明 |
|---|---|---|---|
| GET | `/healthz` | 无 | 存活探针，`200 {"status":"ok"}` |
| GET | `/readyz` | 无 | 就绪探针（DB 可写 + 迁移完成） |
| GET | `/api/v1/meta` | 无 | 枚举与内置变量清单，供前端渲染表单 |
| GET | `/metrics` | 无 | Prometheus 文本指标 |

`GET /api/v1/meta` →（**所有枚举列表统一为 `{value,label}` 对象数组**，不是字符串数组）

```json
{
  "version": "0.1.0",
  "repeat_policies": [{"value":"suppress","label":"不发送"},{"value":"send","label":"发送"},{"value":"send_and_revoke_previous","label":"发送并移除旧告警"}],
  "resolved_policies": [{"value":"send","label":"发送"},{"value":"send_and_delete_previous","label":"发送并删除旧告警"},{"value":"update_previous","label":"更新旧告警"}],
  "group_update_policies": [{"value":"update_previous","label":"更新旧告警"},{"value":"send","label":"发送新消息"},{"value":"suppress","label":"不发送"}],
  "match_ops": [{"value":"=","label":"等于"},{"value":"!=","label":"不等于"},{"value":"=~","label":"正则匹配"},{"value":"!~","label":"正则不匹配"}],
  "status_label_types": [{"value":"primary","label":"主类型"},{"value":"secondary","label":"次要类型"},{"value":"success","label":"成功类型"},{"value":"danger","label":"危险类型"}],
  "feed_button_types": [{"value":"default","label":"默认"},{"value":"primary","label":"主要"},{"value":"success","label":"成功"}],
  "card_sources": [{"value":"builder_json","label":"卡片搭建工具 JSON（JSON 2.0）"},{"value":"builder_template","label":"卡片搭建工具模板（template_id）"}],
  "channel_kinds": [{"value":"group","label":"群聊"},{"value":"p2p_user","label":"单聊用户"}],
  "receive_id_types": [{"value":"chat_id","label":"群 chat_id"},{"value":"open_id","label":"用户 open_id"},{"value":"user_id","label":"用户 user_id"},{"value":"email","label":"用户邮箱"}],
  "variables": [
    {"name":"alertname","group":"alert","type":"string","description":"告警名称"},
    {"name":"severity","group":"group","type":"string","description":"严重级别原始值"},
    {"name":"status_text","group":"group","type":"string","description":"未解决 / 已恢复"},
    {"name":"status_color","group":"group","type":"string","description":"状态对应卡片颜色（未恢复按严重级别，已恢复 green）"},
    {"name":"alerts_table","group":"block","type":"markdown","description":"预渲染告警表格"}
  ],
  "severity_colors": {"critical":"red","warning":"orange","info":"blue","default":"grey"},
  "token_prefix": "alkh_",
  "hook_path_template": "/api/v1/hooks/alertmanager/{token}"
}
```

字段约定：

| 字段 | 说明 |
|---|---|
| 枚举列表 | 一律为 `[{"value":"…","label":"…"}]`；`value` 是**字符串**，可直接用作 `<option value>` / React `key` |
| `variables[].type` | `string` / `number` / `bool` / `datetime` / `url` / `json` / `markdown` |
| `variables[].group` | `alert` / `group` / `block` / `dynamic` / `item`（`item` = 只在「循环」指令内可用） |
| `severity_colors` | **对象映射**（`severity → 卡片 header 颜色`），不是枚举数组 |
| `card_components` | 飞书卡片 JSON 2.0 **组件目录**：驱动本地卡片构建工具的面板/属性面板，也是保存时结构校验的依据 |
| `card_presets` | **告警积木**：`{id,label,description,nodes_json}`，`nodes_json` 是可直接插入 `body.elements` 的节点数组 |

`card_components[]` 的元素：

```jsonc
{
  "tag": "markdown",
  "label": "富文本",
  "group": "display",              // container | display | interactive | element | directive | deprecated
  "description": "…",
  "children_key": null,            // 子节点所在字段：'elements' | 'columns' | null（叶组件）
  "children": [],                  // 允许的子 tag；空数组 = 任意
  "child_of": [],                  // 允许的父 tag；空数组 = 任意位置（如 column 只能是 ['column_set']）
  "palette": true,                 // 是否出现在组件面板
  "deprecated": "…",               // 仅 2.0 已废弃的组件（note / action）出现
  "default_json": "{\"tag\":\"markdown\",\"content\":\"\"}",   // 新建节点的默认 JSON
  "props": [
    { "name": "content", "label": "内容（Markdown）", "type": "markdown",
      "path": "content", "accessor": "field", "values": [], "max_chars": null,
      "placeholder": "${alerts_summary}", "hint": "…" }
  ]
}
```

> `group = "directive"` 的 `repeat`（循环）/ `if`（条件）不是飞书组件，而是**本地指令**：
> 保存进卡片 JSON 后由服务端在预览/发送时展开成真实组件
> （见 [`business-rules.md`](business-rules.md) §5.3）。

约定：`props[].type` ∈ `text|markdown|number|bool|enum|px|url`；`props[].accessor` ∈ `field|button_url`
（`button_url` 表示该字段读写 `behaviors[{type:"open_url",default_url}]`）；`path` 是点号路径
（如 `text.content` / `header.title.content`）。**前端不另立规则**，新增飞书组件只改后端目录。

## 1. 鉴权

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/auth/login` | body `{username,password}` → `200 {"username":"admin","must_change_password":false,"token":"..."}` + `Set-Cookie`；失败 `401 unauthenticated`（与「会话过期」同码，前端靠 `authProbe` 区分文案） |
| POST | `/auth/logout` | 清理会话 |
| GET | `/auth/session` | `200 {"authenticated":true,"username":"admin"}` / `401` |
| POST | `/auth/password` | body `{current_password,new_password}` |

## 2. 设置

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/settings` | 返回设置（`app_secret` 永不回显，仅 `app_secret_set: bool`） |
| PUT | `/settings` | 更新；`app_secret` 为 write-only，传 `null` 表示不变，传 `""` 表示清除；`timezone` 传 IANA 名（如 `Asia/Shanghai`），非法名返回 400，传 `null` 表示不变 |
| POST | `/settings/check` | 用当前凭据换取 `tenant_access_token` 并拉一次群列表，返回 `{"ok":true,"chat_count":12}` 或 `{"ok":false,"error_code":...,"message":"..."}`（**没有 `bot_name`**） |

```json
// GET /settings
{
  "lark_app_id": "cli_aa4645f83578dbef",
  "app_secret_set": true,
  "lark_base_url": "https://open.feishu.cn",
  "public_base_url": "https://alkh.example.com",
  "timezone": "Asia/Shanghai",
  "retention": {"alert_group_days":30,"delivery_days":30,"audit_days":14}
}
```

## 3. 渠道

`Channel` 表示：

```json
{
  "id": "3f6c...",
  "name": "运维告警群",
  "receive_id": "oc_a0553eda9014c201e6969b478895c230",
  "receive_id_type": "chat_id",
  "kind": "group",
  "avatar_url": "https://...",
  "external": false,
  "enabled": true,
  "source": "feishu_sync",
  "bot_can_send": true,
  "hook_count": 1,
  "policy": { "...NotificationPolicy..." },
  "hooks": [{"id":"...","name":"ops","token_prefix":"alkh_a1b2","hook_path":"/api/v1/hooks/alertmanager/alkh_a1b2...","hook_url":"https://alkh.example.com/api/v1/hooks/alertmanager/alkh_a1b2...","hook_url_available":true,"enabled":true,"last_called_at":null,"call_count":0,"created_at":"..."}],
  "created_at": "2026-10-13T00:00:00Z",
  "updated_at": "2026-10-13T00:00:00Z"
}
```

> Hook 地址**不是**只显示一次的秘密：列表接口就返回完整 `hook_path` / `hook_url`（含 token），
> 供运维随时复制，泄露后轮换即可。地址的主机部分来自 `public_base_url`（可填集群内 DNS），
> 与浏览器访问地址无关。`token_hash` 仍只用于鉴权，数据库中保存的 token 为密文（AES-256-GCM）。
> 0003 迁移前创建的旧 Hook 无法回显，`hook_url_available=false` 且 `hook_url` 为遮蔽形式（含 `…`），
> 轮换一次即可恢复。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/channels?enabled=&q=&limit=&offset=` | 列表 |
| POST | `/channels` | body `{name,receive_id,receive_id_type,kind?,enabled?}` → `201` 渠道（自动创建默认策略与 1 个默认 Hook，响应附带 `hook_token`） |
| GET | `/channels/:id` | 详情 |
| PATCH | `/channels/:id` | body 任意子集 `{name,enabled}` |
| DELETE | `/channels/:id` | 级联删除 hooks/policies；已有投递记录时改为软删（`enabled=false`）并返回 `409 conflict`（**没有细分 code**），除非 `?force=true` |
| POST | `/channels/sync` | body `{create_missing:true,"import_names":true}` → `{"discovered":12,"created":3,"updated":9,"skipped":0}` |
| GET | `/channels/:id/policy` | 返回 NotificationPolicy |
| PUT | `/channels/:id/policy` | 全量替换策略（校验见 business-rules §6） |
| POST | `/channels/:id/test-message` | body `{card_template_id?,sample?:"firing"|"resolved",mock?}` → 发送测试消息，返回 **`Delivery`**（看 `state`，没有 `ok` 字段） |

`NotificationPolicy` 表示：

```json
{
  "card_template_id": "b1d2...",
  "pin_unresolved": false,
  "unpin_on_resolve": true,
  "pin_resolved": false,
  "repeat_policy": "suppress",
  "resolved_policy": "send",
  "group_update_policy": "update_previous",
  "label_matchers": [{"name":"severity","op":"=~","value":"critical|warning"}],
  "aggregation_window_secs": 0,
  "min_send_interval_secs": 0,
  "max_alerts_per_message": 20,
  "mute_until": null,
  "updated_at": "2026-10-13T00:00:00Z"
}
```

## 4. 接收端点（Hook）

| 方法 | 路径 | 鉴权 | 说明 |
|---|---|---|---|
| POST | `/hooks/alertmanager/:token` | token 即凭证 | AlertManager webhook 接收 |
| GET | `/hooks` | 会话 | 所有 hook 列表（跨渠道），返回**裸数组**（不是分页信封） |
| POST | `/hooks` | 会话 | body `{channel_id,name}` → `201` + 明文 **`hook_token`**/`hook_url`。注意轮换用的是另一个字段名 `token` |
| PATCH | `/hooks/:id` | 会话 | `{name?,enabled?}` |
| POST | `/hooks/:id/rotate` | 会话 | 轮换 token → `{"token":"alkh_...","hook_url":"...","hook_path":"..."}`；旧 token 立即失效，新地址在列表中可随时回看 |
| DELETE | `/hooks/:id` | 会话 | 删除 |

`POST /hooks/alertmanager/:token` 响应：

```json
{"ok": true, "event": "new_firing", "action": "send_new", "alert_group_id": "..."}
{"ok": true, "event": "repeated", "action": "suppress", "alert_group_id": "..."}
{"ok": true, "skipped": "filtered"}
```

字段说明：

| 字段 | 说明 |
|---|---|
| `ok` | 请求已被受理（**不代表消息已发出**） |
| `event` | `new_firing` / `repeated` / `updated` / `resolved` |
| `action` | 决策动作：`send_new` / `send_new_then_revoke` / `update_in_place` / `suppress` |
| `alert_group_id` / `delivery_id` | 落库后的分组与投递 ID |
| `delivery_state` | 投递的实际落库状态：`active`（已发出）/ `pending`（待后台重试）/ `failed` |
| `skipped` | 被跳过的原因：`hook_disabled` / `channel_disabled` / `filtered` / `muted` / `duplicate` / `no_alerts` / `policy_suppress` |
| `deferred` | 因聚合窗口或最小发送间隔被延后，稍后由后台任务冲刷 |

> 指标与告警排查请以 `delivery_state` 为准，而不是 `action`——`action` 只是决策意图。

## 5. 卡片模板

> ⚠️ **卡片内容嵌在 `card_message` 子对象里，不是顶层扁平字段**；声明变量的类型字段名是 `ty`（不是 `type`）。
> 写成扁平字段时后端会因为 `card_message` 缺失而**静默回退成内置默认卡片**，用户的内容被丢弃。
>
> `card_message.builder_meta` 是**编辑器元数据**（目前只有 `labels`：`element_id → 节点名称`）：
> 与 `card_json` 平级、随模板落库、原样回传，但**不参与渲染**也**不会发给飞书**；
> 它只为「画布上给组件起个名字」服务。不带该字段时表示清空（旧客户端不受影响）。
> 这些形状由 `crates/alkh-server/tests/e2e.rs` 钉住，改动必须同步 e2e 与本文件。

```json
// 卡片模板
{
  "id": "b1d2...",
  "name": "告警卡片-默认",
  "is_default": true,
  "card_message": {
    "source": "builder_json",
    "card_json": {"schema":"2.0","config":{},"header":{},"body":{"elements":[]}},
    "template_id": null,
    "template_version_name": null,
    "variables": [{"name":"status_text","ty":"string","default":"","builtin":true}],
    "builder_meta": {"labels": {"summary": "总体状态"}}
  },
  "detected_variables": ["alertname","alerts_table"],
  "unknown_variables": [],
  "created_at": "...",
  "updated_at": "..."
}

```

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/card-templates` | 列表，返回**裸数组**（不是分页信封） |
| POST | `/card-templates` | 创建。body `{name,card_message?,is_default?}`，`name` 必填；缺 `card_message` 时回退内置默认卡片 |
| GET | `/card-templates/:id` | 详情 |
| PATCH | `/card-templates/:id` | 更新 |
| DELETE | `/card-templates/:id` | 被策略引用时 `409 conflict`（**没有细分 code**） |
| POST | `/card-templates/validate` | body `{name,card_message?,is_default?}`（**`name` 必填，缺了是 422**）→ `{valid,errors:[],warnings:[],detected_variables:[],unknown_variables:[]}`；errors 含**组件目录**的结构错误（废弃组件、`column` 不在 `column_set` 内、`element_id` 重复/非法、超 200 组件…），warnings 含未知组件/未知属性 |
| GET | `/card-templates/default` | 内置默认模板内容 `{card_message}`；卡片编辑器用它作「起步卡片」和旧来源转换的落点（**唯一事实来源在后端**） |
| POST | `/card-templates/preview` | body `{name,card_message?,sample:"firing"\|"resolved",channel_id?,mock?,is_default?}` → `{rendered_card_json,variables:{...},card_content}` |
| GET | `/card-templates/mock?channel_id=&sample=&alert_group_id=` | mock 基线：返回该数据源渲染出的**全部**变量，供编辑器 mock 面板列出（P1） |
| POST | `/card-templates/send-test` | **样式验证发送**（模板与投递都**不落库**）：body `{channel_id,name,card_message?,sample?,mock?}` → 真实发送一次，返回**内存回执** `Delivery`（`alert_group_id` 是合成值，不对应数据库行；无重试 / 撤回） |
| POST | `/card-templates/:id/send-test` | body `{channel_id,sample?,mock?}` → 用已保存模板做样式验证发送，同样**不写 alert_groups / deliveries** |
| POST | `/channels/:id/test-message` | body `{card_template_id?,sample?,mock?}` → 渠道级测试消息：**走完整链路并落库**（写「测试分组」+ `kind=test` 投递，可重试 / 撤回） |

> **`source = builder_template` 已停用**（P2）：新建 / 更新时返回 `400 validation`
> （「已停用来源 builder_template…」）。历史模板仍可 `GET` / `preview` 读取，
> 编辑器会给出一键「转换为本地卡片」（切到 `builder_json` + 载入 `/card-templates/default` 的内容）。
> 理由：JSON 导入的卡片不支持循环容器，而搭建工具模板又无法在本地校验/预览/版本化，
> 两条来源混用会让"预览通过但发送失败"难以排查。

### 5.1 mock 数据（P1）

预览与测试发送共用同一份 mock 契约。三层语义，优先级从低到高：

1. **基线**：`sample=firing|resolved` 的内置样例，或 `alert_group_id` 指定的一条**历史告警分组**；
2. **覆盖**：`mock.variables` 逐项覆盖，支持 `label:` / `annotation:` 动态 key；
3. 覆盖里的未知 key 也会写入渲染上下文（用于试写新标签），不会报错。

```jsonc
// 请求片段（preview / send-test / test-message 通用）
{
  "sample": "firing",
  "channel_id": "…",
  "mock": {
    "alert_group_id": null,                       // 设置后作为基线数据源，sample 仅作展示
    "variables": { "summary": "自定义概要", "label:team": "sre" }
  }
}
```

规则：

- `mock` 只影响**渲染**。历史分组**只作数据源、绝不回写**。
- 两种测试发送的落库语义不同：
  - **卡片编辑器**（`/card-templates/send-test`、`/card-templates/:id/send-test`）是**样式验证**：
    只发送，不写 `alert_groups` / `deliveries`，返回的 `Delivery` 只是内存回执（无重试 / 撤回）；
    因此可以反复发送检查卡片长什么样，也不会撞 `alert_groups(hook_id, group_key)` 的唯一索引。
  - **渠道页**（`/channels/:id/test-message`）走**完整链路并落库**：消息归属该渠道的「测试分组」，
    投递记录里是 `kind=test`，可重试 / 撤回。
- 指定 `alert_group_id` 时以该分组状态为准（`firing` / `resolved`）。
- `POST /card-templates/preview` 返回的 `variables` 是**覆盖后**的有效上下文。
- 规模上限：`variables` 最多 200 项、单个值最多 4000 字符、单个 key 最多 128 字符，超限 `400 validation`。

`GET /card-templates/mock` → 

```json
{
  "source": "builtin",
  "sample": "firing",
  "alert_group_id": null,
  "alert_group_label": null,
  "variables": { "alertname": "HighCPUUsage", "status_text": "未解决", "alerts_list": "…", "label:team": "sre" }
}
```

`source=alert_group` 时 `alert_group_id` 非空、`alert_group_label` 给出 `告警名 · 状态 · 条数` 摘要；
`alert_group_id` 不存在时返回 `404 not_found`（**不会**静默回退到内置样例）。

## 6. 告警分组与投递

> ⚠️ `channel` / `hook` **只有 `GET /alert-groups/:id` 详情返回**；`GET /alert-groups` 列表没有这两个字段。

```json
// AlertGroup（详情；列表不含 channel / hook）
{
  "id": "...", "group_key": "{}:{alertname=\"HighCPU\"}", "receiver": "lark-ops",
  "channel": {"id":"...","name":"运维告警群"},
  "hook": {"id":"...","name":"ops"},
  "status": "firing", "alertname": "HighCPU", "severity": "critical",
  "group_labels": {}, "common_labels": {}, "common_annotations": {},
  "alerts": [{"fingerprint":"a1b2","status":"firing","labels":{},"annotations":{},"starts_at":"...","ends_at":null,"generator_url":"..."}],
  "firing_count": 3, "resolved_count": 1, "truncated_alerts": 0, "event_count": 7,
  "first_seen_at": "...", "last_seen_at": "...", "last_notified_at": "...", "resolved_at": null,
  "active_delivery_id": "...", "pinned_messages": [{"message_id":"om_...","kind":"unresolved"}]
}
```

```json
// Delivery
{
  "id": "...", "alert_group_id": "...", "kind": "firing_created",
  "state": "active",
  "card_template_id": "...", "lark_message_id": "om_...", "card_entity_id": "7355...",
  "attempt": 1, "last_error": null,
  "created_at": "...", "updated_at": "..."
}
```

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/alert-groups?status=&channel_id=&severity=&q=&limit=&offset=` | 分组列表（按 `last_seen_at` 倒序） |
| GET | `/alert-groups/:id` | 详情（含 deliveries） |
| DELETE | `/alert-groups/:id` | 删除记录（不影响已发消息） |
| GET | `/deliveries?alert_group_id=&channel_id=&state=&limit=&offset=` | 投递列表 |
| POST | `/deliveries/:id/retry` | 重试失败投递（**同步重发一次**，返回本次结果，见下） |
| POST | `/deliveries/:id/revoke` | 手动撤回对应飞书消息 |

> **`POST /deliveries/:id/retry` 的语义**：它不是「入队」，而是**当场同步调用飞书重发一次**，
> 返回的 `Delivery.state` 就是本次尝试的真实落库结果：
> - `active` / `resolved` → 消息已发出；
> - `failed` / `pending` → 这次仍然没发出去，HTTP 仍是 **200**，失败原因在 `last_error`。
>
> 后台重试任务只处理 `state = pending`（`attempt < MAX_ATTEMPTS`）；`failed` 是终态，
> 只能靠手动重试。若导致失败的原因未修复（飞书权限不足、机器人不在群、凭据错误、模板渲染失败等），
> 每次重试都会立刻再次失败——表现为「点了重试但一直收不到」。排查请看 `last_error`：
> 已知的飞书配置类错误码（`230002` 机器人不在群、`230006` 未开启机器人能力、`230013` 不在可用范围、
> `99991672` 缺少权限 scope、`200380` 卡片模板未发布）会在 `last_error` 里附上中文修复提示。
> 投递所属的告警分组已被删除时，重试会返回 **404**（`alert_group_id` 悬空，无法重建投递上下文）。

## 7. 概览与审计

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/overview` | 仪表盘：渠道数、活跃 Hook 数、firing/resolved 分组数、近 24h 投递成功/失败、最近 10 条事件 |
| GET | `/audit-logs?action=&outcome=&limit=&offset=` | 审计日志（`detail` 为任意 JSON） |

```json
// GET /overview
{
  "channels": {"total": 12, "enabled": 10},
  "hooks": {"total": 13, "enabled": 13},
  "alert_groups": {"firing": 4, "resolved": 31},
  "deliveries_24h": {"sent": 128, "failed": 2, "suppressed": 40},
  "recent_events": [{"at":"...","action":"message.sent","target":"...","outcome":{"type":"ok"}}]
}
```

## 8. 状态码与错误码

| HTTP | code | 场景 |
|---|---|---|
| 400 | `validation_failed` | 字段校验失败（`details` 给出字段级错误） |
| 401 | `unauthenticated`（登录失败与会话过期同码） | |
| 403 | `forbidden` | |
| 404 | `not_found` / `unknown_hook` | |
| 409 | `conflict`（**只有这一个 code**，不区分模板引用 / 渠道有投递 / 重复 receive_id） | |
| 502 | `lark_api_error` | 飞书返回非 0 code（`details` 含 `code`/`log_id`） |
| 500 | `internal_error` | |
