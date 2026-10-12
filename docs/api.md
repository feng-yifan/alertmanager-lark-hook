# REST API 契约

- Base path：`/api/v1`
- 请求/响应：`application/json; charset=utf-8`
- 管理接口鉴权：`Cookie: alkh_session=<token>`（HttpOnly）或 `Authorization: Bearer <token>`
- 错误信封：`{"error":{"code":"<machine_code>","message":"<human>"}}`
- 分页信封：`{"items":[...],"total":123,"limit":50,"offset":0}`
- 时间：RFC 3339 UTC（`2026-10-13T00:00:00Z`）
- ID：UUID v4 字符串

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
  "message_types": [{"value":"card_message","label":"卡片消息"},{"value":"app_feed_card","label":"应用消息流消息"}],
  "repeat_policies": [{"value":"suppress","label":"不发送"},{"value":"send","label":"发送"},{"value":"send_and_revoke_previous","label":"发送并移除旧告警"}],
  "resolved_policies": [{"value":"send","label":"发送"},{"value":"send_and_delete_previous","label":"发送并删除旧告警"},{"value":"update_previous","label":"更新旧告警"}],
  "group_update_policies": [{"value":"update_previous","label":"更新旧告警"},{"value":"send","label":"发送新消息"},{"value":"suppress","label":"不发送"}],
  "match_ops": [{"value":"=","label":"等于"},{"value":"!=","label":"不等于"},{"value":"=~","label":"正则匹配"},{"value":"!~","label":"正则不匹配"}],
  "status_label_types": [{"value":"primary","label":"主类型"},{"value":"secondary","label":"次要类型"},{"value":"success","label":"成功类型"},{"value":"danger","label":"危险类型"}],
  "feed_button_types": [{"value":"default","label":"默认"},{"value":"primary","label":"主要"},{"value":"success","label":"成功"}],
  "biz_id_sources": [{"value":"alert_group","label":"按告警分组（同分组复用同一张卡片，默认）"},{"value":"hook","label":"按 Hook + 告警分组"}],
  "card_sources": [{"value":"builder_json","label":"卡片搭建工具 JSON（JSON 2.0）"},{"value":"builder_template","label":"卡片搭建工具模板（template_id）"}],
  "channel_kinds": [{"value":"group","label":"群聊"},{"value":"p2p_user","label":"单聊用户"}],
  "receive_id_types": [{"value":"chat_id","label":"群 chat_id"},{"value":"open_id","label":"用户 open_id"},{"value":"user_id","label":"用户 user_id"},{"value":"email","label":"用户邮箱"}],
  "variables": [
    {"name":"alertname","group":"alert","type":"string","description":"告警名称"},
    {"name":"severity","group":"alert","type":"string","description":""},
    {"name":"status_text","group":"group","type":"string","description":"未解决 / 已恢复"},
    {"name":"alerts_table","group":"block","type":"markdown","description":"预渲染告警表格"}
  ],
  "severity_colors": {"critical":"red","warning":"orange","info":"blue","default":"grey"},
  "pin_supported_message_types": ["app_feed_card"],
  "token_prefix": "alkh_",
  "hook_path_template": "/api/v1/hooks/alertmanager/{token}"
}
```

字段约定：

| 字段 | 说明 |
|---|---|
| 枚举列表 | 一律为 `[{"value":"…","label":"…"}]`；`value` 是**字符串**，可直接用作 `<option value>` / React `key` |
| `variables[].type` | `string` / `number` / `bool` / `datetime` / `url` / `json` / `markdown` |
| `variables[].group` | `alert` / `group` / `block` / `dynamic` |
| `severity_colors` | **对象映射**（`severity → 卡片 header 颜色`），不是枚举数组 |
| `feed_button_types` | 应用消息流卡片按钮类型，与 `status_label_types` **不是**同一个枚举 |
| `pin_supported_message_types` | `pin_unresolved` 真正生效的消息类型（R4：仅 `app_feed_card`） |

## 1. 鉴权

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/auth/login` | body `{username,password}` → `200 {"username":"admin","must_change_password":false}` + `Set-Cookie`；失败 `401 invalid_credentials` |
| POST | `/auth/logout` | 清理会话 |
| GET | `/auth/session` | `200 {"authenticated":true,"username":"admin"}` / `401` |
| POST | `/auth/password` | body `{current_password,new_password}` |

## 2. 设置

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/settings` | 返回设置（`app_secret` 永不回显，仅 `app_secret_set: bool`） |
| PUT | `/settings` | 更新；`app_secret` 为 write-only，传 `null` 表示不变，传 `""` 表示清除 |
| POST | `/settings/check` | 用当前凭据换取 `tenant_access_token` 并拉一次群列表，返回 `{"ok":true,"bot_name":"...","chat_count":12}` 或 `{"ok":false,"error_code":...,"message":"..."}` |

```json
// GET /settings
{
  "lark_app_id": "cli_aa4645f83578dbef",
  "app_secret_set": true,
  "lark_base_url": "https://open.feishu.cn",
  "public_base_url": "https://alkh.example.com",
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
  "hooks": [{"id":"...","name":"ops","token_prefix":"alkh_a1b2","hook_path":"/api/v1/hooks/alertmanager/<token>","hook_url":"https://alkh.example.com/api/v1/hooks/alertmanager/<token>","enabled":true,"last_called_at":null,"call_count":0,"created_at":"..."}],
  "created_at": "2026-10-13T00:00:00Z",
  "updated_at": "2026-10-13T00:00:00Z"
}
```

> `hook_url` 中的明文 token **仅在创建/轮换该 hook 的响应中出现一次**；
> 列表接口返回 `hook_path` 的占位形式（`<token>`）与 `token_prefix`。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/channels?enabled=&q=&limit=&offset=` | 列表 |
| POST | `/channels` | body `{name,receive_id,receive_id_type,kind?,enabled?}` → `201` 渠道（自动创建默认策略与 1 个默认 Hook，响应附带 `hook_token` 明文一次） |
| GET | `/channels/:id` | 详情 |
| PATCH | `/channels/:id` | body 任意子集 `{name,enabled}` |
| DELETE | `/channels/:id` | 级联删除 hooks/policies；已有投递记录时改为软删（`enabled=false`）并返回 `409 channel_has_deliveries`，除非 `?force=true` |
| POST | `/channels/sync` | body `{create_missing:true,"import_names":true}` → `{"discovered":12,"created":3,"updated":9,"skipped":0}` |
| GET | `/channels/:id/policy` | 返回 NotificationPolicy |
| PUT | `/channels/:id/policy` | 全量替换策略（校验见 business-rules §6） |
| POST | `/channels/:id/test-message` | body `{message_type?,card_template_id?,kind?:"firing"|"resolved"}` → 发送测试消息，返回 `{ok,delivery_id,message_id}` |

`NotificationPolicy` 表示：

```json
{
  "message_type": "card_message",
  "card_template_id": "b1d2...",
  "pin_unresolved": false,
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
| GET | `/hooks` | 会话 | 所有 hook 列表（跨渠道） |
| POST | `/hooks` | 会话 | body `{channel_id,name}` → `201` + 明文 `token`/`hook_url`（仅此一次） |
| PATCH | `/hooks/:id` | 会话 | `{name?,enabled?}` |
| POST | `/hooks/:id/rotate` | 会话 | 轮换 token → `{"token":"alkh_...","hook_url":"..."}` |
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

```json
// kind = card_message
{
  "id": "b1d2...",
  "kind": "card_message",
  "name": "告警卡片-默认",
  "source": "builder_json",
  "card_json": {"schema":"2.0","config":{},"header":{},"body":{"elements":[]}},
  "template_id": null,
  "template_version_name": null,
  "variables": [{"name":"status_text","type":"string","default":"","builtin":true}],
  "detected_variables": ["alertname","alerts_table"],
  "is_default": true,
  "created_at": "...",
  "updated_at": "..."
}

// kind = app_feed_card
{
  "id": "c9f8...",
  "kind": "app_feed_card",
  "name": "应用消息流-默认",
  "app_feed_spec": {
    "title": "${alertname} ${status_text}",
    "preview": "${summary}",
    "status_label": {"text":"${severity_text}","type_firing":"danger","type_resolved":"success"},
    "link": {"enabled": true, "url": "${external_url}"},
    "buttons": [{"text":"查看详情","url":"${external_url}","button_type":"primary"}],
    "close_notify": false,
    "biz_id_source": "alert_group"
  },
  "detected_variables": ["alertname","status_text","summary","severity_text","external_url"],
  "is_default": true,
  "created_at": "...",
  "updated_at": "..."
}
```

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/card-templates?kind=` | 列表 |
| POST | `/card-templates` | 创建（`card_message` 或 `app_feed_card` 判别式 body） |
| GET | `/card-templates/:id` | 详情 |
| PATCH | `/card-templates/:id` | 更新 |
| DELETE | `/card-templates/:id` | 被策略引用时 `409 template_in_use` |
| POST | `/card-templates/validate` | body `{kind,card_json?,app_feed_spec?}` → `{valid,errors:[],warnings:[],detected_variables:[],unknown_variables:[]}` |
| POST | `/card-templates/preview` | body `{kind,card_json?,app_feed_spec?,sample:"firing"|"resolved",channel_id?}` → `{rendered_card_json, rendered_app_feed_spec, variables:{...}}` |
| POST | `/card-templates/:id/send-test` | body `{channel_id,sample?}` → 真实发送一次 |

## 6. 告警分组与投递

```json
// AlertGroup
{
  "id": "...", "group_key": "{}:{alertname=\"HighCPU\"}", "receiver": "lark-ops",
  "channel": {"id":"...","name":"运维告警群"},
  "hook": {"id":"...","name":"ops"},
  "status": "firing", "alertname": "HighCPU", "severity": "critical",
  "group_labels": {}, "common_labels": {}, "common_annotations": {},
  "alerts": [{"fingerprint":"a1b2","status":"firing","labels":{},"annotations":{},"starts_at":"...","ends_at":null,"generator_url":"..."}],
  "firing_count": 3, "resolved_count": 1, "truncated_alerts": 0, "event_count": 7,
  "first_seen_at": "...", "last_seen_at": "...", "last_notified_at": "...", "resolved_at": null,
  "active_delivery_id": "..."
}
```

```json
// Delivery
{
  "id": "...", "alert_group_id": "...", "kind": "firing_created",
  "message_type": "card_message", "state": "active",
  "card_template_id": "...", "lark_message_id": "om_...", "card_entity_id": "7355...",
  "app_feed_biz_id": null, "attempt": 1, "last_error": null,
  "created_at": "...", "updated_at": "..."
}
```

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/alert-groups?status=&channel_id=&severity=&q=&limit=&offset=` | 分组列表（按 `last_seen_at` 倒序） |
| GET | `/alert-groups/:id` | 详情（含 deliveries） |
| DELETE | `/alert-groups/:id` | 删除记录（不影响已发消息） |
| GET | `/deliveries?alert_group_id=&channel_id=&state=&limit=&offset=` | 投递列表 |
| POST | `/deliveries/:id/retry` | 重试失败投递 |
| POST | `/deliveries/:id/revoke` | 手动撤回对应飞书消息 |

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
  "recent_events": [{"at":"...","action":"message.sent","target":"...","outcome":"ok"}]
}
```

## 8. 状态码与错误码

| HTTP | code | 场景 |
|---|---|---|
| 400 | `validation_failed` | 字段校验失败（`details` 给出字段级错误） |
| 401 | `unauthenticated` / `invalid_credentials` | |
| 403 | `forbidden` | |
| 404 | `not_found` / `unknown_hook` | |
| 409 | `template_in_use` / `channel_has_deliveries` / `duplicate_receive_id` | |
| 502 | `lark_api_error` | 飞书返回非 0 code（`details` 含 `code`/`log_id`） |
| 500 | `internal_error` | |
