# Feishu / Lark OpenAPI reference for an AlertManager → Feishu message forwarder

Implementation-grade reference for the endpoints needed by this project (Rust service that turns
Prometheus AlertManager webhooks into Feishu/Lark messages, cards and app-feed cards).

- Base URL (China / `open.feishu.cn`): `https://open.feishu.cn`
- Base URL (international / Lark): `https://open.larksuite.com` (same paths; not re-verified here)
- All request bodies are JSON (`Content-Type: application/json; charset=utf-8`).
- All authenticated calls use `Authorization: Bearer <access_token>`.
- Field names below are copied **verbatim** from the official docs, including the Chinese
  descriptions and every listed enum value. Do not rename them.
- Everything marked **VERIFIED** was read from the official page whose URL is given next to the
  section. Anything not verified is listed in [Gaps / uncertainties](#gaps--uncertainties).

Doc URLs use the markdown trick `….md?lang=zh-CN`; the canonical human page is the same path
without `.md`.

---

## 1. Access tokens (自建应用 / internal app)

### 1.1 `tenant_access_token` (the token this project should use) — VERIFIED

Source: <https://open.feishu.cn/document/server-docs/authentication-management/access-token/tenant_access_token_internal.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal` |
| HTTP Method | `POST` |
| 权限要求 | 无 (no scope required) |
| 支持的应用类型 | Custom App (自建应用) |

Request body:

| name | type | required | description |
|---|---|---|---|
| `app_id` | string | **yes** | 应用唯一标识, e.g. `"cli_slkdjalasdkjasd"` |
| `app_secret` | string | **yes** | 应用秘钥, e.g. `"dskLLdkasdjlasdKK"` |

```json
{
    "app_id": "cli_slkdjalasdkjasd",
    "app_secret": "dskLLdkasdjlasdKK"
}
```

Response body:

| name | type | description |
|---|---|---|
| `code` | int | 错误码，非 0 取值表示失败 |
| `msg` | string | 错误描述 |
| `tenant_access_token` | string | 租户访问凭证 |
| `expire` | int | `tenant_access_token` 的过期时间，单位为秒 |

```json
{
    "code": 0,
    "msg": "ok",
    "tenant_access_token": "t-caecc734c2e3328a62489fe0648c4b98779515d3",
    "expire": 7200
}
```

Expiry / refresh semantics (verbatim from the "注意事项" section):

- `tenant_access_token` 的最大有效期是 **2 小时**.
- 剩余有效期小于 30 分钟时，调用本接口会返回一个**新的** `tenant_access_token`，这会同时存在两个有效的 `tenant_access_token`.
- 剩余有效期大于等于 30 分钟时，调用本接口会返回**原有的** `tenant_access_token`.

Practical implication for this service: cache the token in memory, treat `expire` as seconds, and
re-fetch when `now >= issued_at + expire - 300` (5 min safety margin). The API is safe to call
"early". `code != 0` means failure.

### 1.2 `app_access_token` variant (brief) — VERIFIED

Source: <https://open.feishu.cn/document/server-docs/authentication-management/access-token/app_access_token_internal.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/auth/v3/app_access_token/internal` |
| HTTP Method | `POST` |
| 权限要求 | 无 |

Same request body (`app_id`, `app_secret`). Response:

| name | type | description |
|---|---|---|
| `code` | int | 错误码，非 0 取值表示失败 |
| `msg` | string | 错误描述 |
| `app_access_token` | string | 应用访问凭证 |
| `expire` | int | `app_access_token` 的过期时间，单位为秒 |
| `tenant_access_token` | string | 租户访问凭证 |

Same 2-hour / 30-minute refresh rule. For IM and CardKit calls the docs require a
**`tenant_access_token`**; use 1.1 for this project.

---

## 2. Send message — VERIFIED

Source: <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v1/messages` |
| HTTP Method | `POST` |
| 接口频率限制 | 1000 次/分钟、50 次/秒 |
| 支持的应用类型 | Custom App、Store App |

Prerequisites (前提条件):

- 应用需要开启机器人能力. 开启能力后需要发布版本才能生效.
- 给用户发送消息时，用户需要在机器人的**可用范围**内.
- 给群组发送消息时，机器人需要在该群组中，且在群组内拥有发言权限.

使用限制:

- 向同一用户发送消息的限频为 **5 QPS**; 向同一群组发送消息的限频为群内机器人共享 **5 QPS**.
- 该接口仅支持在开发者后台创建的应用机器人调用，群自定义机器人无法调用该接口.

### 2.1 权限 (scopes)

| kind | scopes (开启任一权限即可) |
|---|---|
| 权限要求 | 获取与发送单聊、群组消息(`im:message`); 以应用的身份发消息(`im:message:send_as_bot`); 发送消息V2(`im:message:send`) |
| 字段权限要求 | 获取用户 user ID(`contact:user.employee_id:readonly`) — only if you use/return `user_id` |

The doc's note:

- **应用身份**发消息: 需申请 `im:message`, `im:message:send_as_bot`, `im:message:send` 三者之一.
- **用户身份**发消息: 需同时申请 `im:message` + 以用户身份发送消息(`im:message.send_as_user`).
- System messages (`msg_type=system`) additionally need 发送特定模板系统消息(`im:message:send_sys_msg`).

### 2.2 Query param

| name | type | required | allowed values |
|---|---|---|---|
| `receive_id_type` | string | **yes** | `open_id`, `union_id`, `user_id`, `email`, `chat_id` |

`user_id` additionally needs `contact:user.employee_id:readonly`. Recommended: `open_id` for users,
`chat_id` for groups.

### 2.3 Request body

| name | type | required | description |
|---|---|---|---|
| `receive_id` | string | **yes** | 消息接收者的 ID，类型与 `receive_id_type` 一致. 例 `"ou_7d8a6e6df7621556ce0d21922b676706ccs"` |
| `msg_type` | string | **yes** | see enum below |
| `content` | string | **yes** | 消息内容，JSON 结构序列化后的字符串 |
| `uuid` | string | no | 请求去重. 持有相同 `uuid` 的请求，在 **1 小时内至多成功发送一条消息**. 最大长度 `50` 字符 |

`msg_type` allowed values (all of them):

- `text` 文本
- `post` 富文本
- `image` 图片
- `file` 文件
- `audio` 语音
- `media` 视频
- `sticker` 表情包
- `interactive` 卡片
- `share_chat` 分享群名片（被分享的群名片有效期为 7 天）
- `share_user` 分享个人名片
- `system` 系统消息（仅支持在机器人单聊内推送，不支持群聊）

`content` size limits: 文本消息 ≤ **150 KB**; 卡片/富文本消息 ≤ **30 KB** (template data counts
towards the limit). `content` must be escaped JSON, e.g. `"{\"text\":\"test content\"}"`.

Base request example:

```json
{
    "receive_id": "oc_84983ff6516d731e5b5f68d4ea2e1da5",
    "msg_type": "text",
    "content": "{\"text\":\"test content\"}",
    "uuid": "a0d69e20-1dd1-458b-k525-dfeca4015204"
}
```

```bash
curl --location --request POST 'https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id' \
  --header 'Authorization: Bearer t-XXX' \
  --header 'Content-Type: application/json; charset=utf-8' \
  --data-raw '{
    "receive_id": "oc_84983ff6516d731e5b5f68d4ea2e1da5",
    "msg_type": "text",
    "content": "{\"text\":\"test content\"}"
  }'
```

### 2.4 Card variants for `msg_type: "interactive"`

Sources: <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/im-v1/message/create_json.md?lang=zh-CN>
(发送消息内容结构, section 卡片 interactive) and
<https://open.feishu.cn/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/streaming-updates-openapi-overview.md?lang=zh-CN>.

There are exactly **three documented** ways to put a card into `content`. All are VERIFIED.

**(a) Card entity ID — `{"type":"card","data":{"card_id":"…"}}` — VERIFIED (this is real).**

```json
{
    "receive_id": "ou_449b53ad6aee526f7ed311b216aabcef",
    "msg_type": "interactive",
    "content": "{\"type\":\"card\",\"data\":{\"card_id\":\"7371713483664506900\"}}"
}
```

Unescaped inner structure (from the streaming-updates guide):

```json
{
    "type": "card",
    "data": {
        "card_id": "7371713483664506900"
    }
}
```

Rules: 卡片实体仅支持发送一次; 发送卡片实体的应用必须是该卡片实体的创建应用.

**(b) Card builder template — `{"type":"template","data":{…}}` — VERIFIED.**

```json
{
    "receive_id": "ou_449b53ad6aee526f7ed311b216aabcef",
    "msg_type": "interactive",
    "content": "{\"type\":\"template\",\"data\":{\"template_id\":\"xxxxxxxxxxxx\",\"template_version_name\":\"1.0.0\",\"template_variable\":{\"key1\":\"value1\",\"key2\":\"value2\"}}}"
}
```

| field | type | required | description |
|---|---|---|---|
| `type` | string | no | 固定取值为 `template` |
| `data` | object | no | 卡片模板的数据 |
| `data.template_id` | string | **yes** | 搭建工具中的卡片 ID, e.g. `AAqigYkzabcef` |
| `data.template_version_name` | string | no | 版本号, e.g. `1.0.0`; 不填则默认使用最新版本 |
| `data.template_variable` | object | no | 变量名 → 值 (`{key:value}`), key 为搭建工具中声明的变量名称 |

**(c) Inline card JSON — the card JSON is placed directly in `content` (no wrapper) — VERIFIED.**

```json
{
  "receive_id": "ou_449b53ad6aee526f7ed311b216aabcef",
  "msg_type": "interactive",
  "content": "{\"schema\":\"2.0\",\"config\":{\"update_multi\":true,...},\"body\":{\"direction\":\"vertical\",\"elements\":[...]},\"header\":{...}}"
}
```

> **Important:** the shape `content = {"type":"card","data":{"card":{...}}}` (raw card JSON wrapped in
> `type:card`) is **NOT documented anywhere I could find**. The documented raw-JSON form is (c)
> above, i.e. the `schema`/`body` object serialized directly into `content`. See
> [Gaps](#gaps--uncertainties).

Other simple `content` payloads (VERIFIED): `text` → `{"text":"…"}`; `image` →
`{"image_key":"img_…"}`; `file`/`audio`/`sticker` → `{"file_key":"…"}`; `media` →
`{"file_key":"…","image_key":"…"}`; `share_chat` → `{"chat_id":"oc_…"}`; `share_user` →
`{"user_id":"ou_…"}`; `system` → `{"type":"divider","params":{...},"options":{...}}`.

### 2.5 Response body — VERIFIED

| name | type | description |
|---|---|---|
| `code` | int | 错误码，非 0 表示失败 |
| `msg` | string | 错误描述 |
| `data` | message | – |
| `data.message_id` | string | 消息 ID（后续管理消息都要用它） |
| `data.root_id` | string | 根消息 ID（仅回复场景） |
| `data.parent_id` | string | 父消息 ID（仅回复场景） |
| `data.thread_id` | string | 话题 ID（仅话题场景） |
| `data.msg_type` | string | 消息类型（同请求枚举 + 可能值） |
| `data.create_time` | string | 毫秒时间戳 |
| `data.update_time` | string | 毫秒时间戳 |
| `data.deleted` | boolean | 发送时恒为 `false` |
| `data.updated` | boolean | 发送时恒为 `false` |
| `data.chat_id` | string | 消息所属群 ID |
| `data.sender.id` | string | 发送者 ID |
| `data.sender.id_type` | string | `open_id`（用户）或 `app_id`（应用） |
| `data.sender.sender_type` | string | `user` / `app` / `anonymous`（未生效）/ `unknown`（未生效） |
| `data.sender.tenant_key` | string | 租户唯一标识 |
| `data.body.content` | string | 消息内容（JSON 序列化字符串） |
| `data.mentions[]` | mention[] | 被 @ 的用户 |
| `data.mentions[].key` | string | 如 `@_user_1` |
| `data.mentions[].id` | string | 被 @ 用户 open_id |
| `data.mentions[].id_type` | string | 目前仅 `open_id` |
| `data.mentions[].name` | string | 姓名 |
| `data.mentions[].tenant_key` | string | 租户唯一标识 |
| `data.upper_message_id` | string | 合并转发中上一层级消息 ID |

```json
{
    "code": 0,
    "msg": "success",
    "data": {
        "message_id": "om_dc13264520392913993dd051dba21dcf",
        "msg_type": "interactive",
        "create_time": "1615380573411",
        "update_time": "1615380573411",
        "deleted": false,
        "updated": false,
        "chat_id": "oc_5ad11d72b830411d72b836c20",
        "sender": { "id": "cli_9f427eec54ae901b", "id_type": "app_id", "sender_type": "app", "tenant_key": "736588c9260f175e" },
        "body": { "content": "{\"text\":\"@_user_1 test content\"}" }
    }
}
```

Notable error codes: `230002` bot not in group; `230006` bot ability not activated; `230013`
target user outside bot availability (or resigned); `230020` frequency limit; `230025` body too
long; `230034` invalid `receive_id`; `230035` no send permission (muted/blocked); `230099` card
content failed (see sub-codes 100290, 11310, 200380, 200381, 200621, 200732, 200737, 200861, …).

---

## 3. Update an already-sent card message — VERIFIED

Source: <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/patch.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v1/messages/:message_id` |
| HTTP Method | `PATCH` |
| 权限要求 (任一) | `im:message`; `im:message:send_as_bot`; 更新消息(`im:message:update`) |
| 路径参数 | `message_id` |

Request body:

| name | type | required | description |
|---|---|---|---|
| `content` | string | **yes** | 消息卡片的内容，支持卡片 JSON 或搭建工具构建的卡片，JSON 序列化后的字符串（≤ 30 KB） |

```json
{ "content": "{\"elements\":[{\"tag\":\"div\",\"text\":{\"content\":\"This is the plain text\",\"tag\":\"plain_text\"}}],\"header\":{\"template\":\"blue\",\"title\":{\"content\":\"This is the title\",\"tag\":\"plain_text\"}}}" }
```

```json
{ "content": "{\"type\":\"template\",\"data\":{\"template_id\":\"AAqyjwfhabcef\",\"template_version_name\":\"1.0.0\"}}" }
```

使用限制: 调用身份须与发送身份一致; 仅支持更新未撤回的卡片; 更新前后 `config.update_multi`
都必须显式为 `true`; 不支持更新批量发送的消息; 不支持仅特定人可见的卡片; **仅支持更新 14 天内
发送的消息**; 单条消息更新频控 5 QPS.

---

## 4. Recall / delete message — VERIFIED

Source: <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/delete.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v1/messages/:message_id` |
| HTTP Method | `DELETE` |
| 接口频率限制 | 1000 次/分钟、50 次/秒 |
| 权限要求 (开启任一权限即可) | 获取与发送单聊、群组消息(`im:message`); 撤回消息(`im:message:recall`); 以应用的身份发消息(`im:message:send_as_bot`) |

Path param: `message_id` (e.g. `om_dc13264520392913993dd051dba21dcf`).

Prerequisites: 应用需开启机器人能力; 撤回用户单聊内的消息时，用户需要在机器人的可用范围内;
撤回群组内的消息时，机器人需要在该群组中.

权限/时限 rules (verbatim):

- 机器人可以撤回该机器人自己发送的消息.
- 群聊的群主可以撤回群内指定的消息; 在群聊内的机器人如需撤回他人发送的消息，则该机器人必须是该
  群的群主、管理员或者创建者，**且消息发送时间不超过 1 年**.
- 无法撤回通过批量发送消息接口发送的消息（需用批量撤回消息接口）.
- 撤回的消息需要符合**由企业管理员设置的撤回时限**（this is the actual "time limit": it is a
  tenant admin policy, not a hard-coded API constant; see the linked Feishu help article
  "管理员设置撤回和编辑消息权限").

Response: `code`, `msg`, `data` (empty object on success).

```json
{ "code": 0, "msg": "success", "data": {} }
```

Errors: `230001` invalid param; `230002` bot not in group; `230006` bot ability off; `230009`
Message has expired when recall message (发出时间过长); `230013` bot has no availability to user;
`230026` no permission to recall this message (only own messages); `230027` lack of permissions;
`230050` message invisible; `230054` unsupported message type; `230110` message already deleted;
`232009` chat dissolved.

---

## 4.1 Pin / unpin message (置顶消息) — VERIFIED

Sources:
<https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/pin/create.md?lang=zh-CN> ·
<https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/pin/delete.md?lang=zh-CN>

这就是本项目 R4「未解决告警置顶」用的官方能力：普通卡片消息（`im/v1/messages`）
没有 `time_sensitive` 之类的字段，但可以作为一个**消息**被置顶到会话顶部。

### Pin — `POST /open-apis/im/v1/pins`

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v1/pins` |
| HTTP Method | `POST` |
| 频率限制 | 1000 次/分钟、50 次/秒；**对同一条消息**的 Pin 操作不超过 5 QPS |
| 支持的应用类型 | 自建应用 / 商店应用 |
| 权限要求 (开启任一即可) | 获取与发送单聊、群组消息(`im:message`); 添加、取消 Pin 消息(`im:message.pins:write_only`); 以应用的身份发消息(`im:message:send_as_bot`) |

前提条件：应用需开启机器人能力；**Pin 时机器人必须在消息所属的会话内**。
限制：当前操作者不可见的消息无法 Pin。
注意：如果消息已经被 Pin，接口返回该 Pin 的操作信息（等同于成功，可重复调用）。

Body: `message_id` (required) — e.g. `om_dc13264520392913993dd051dba21dcf`.

Response `data.pin`: `{message_id, chat_id, operator_id, operator_id_type (open_id|app_id), create_time}`.

错误码（400）：`230001` 参数错误; `230002` 机器人不在群组中; `230006` 未开启机器人能力;
`230011` 消息已被撤回; `230013` 机器人对该用户不可用; `230027` 权限不足;
`230045` 群聊不存在; `230046` **该群仅群主和群管理员可以 Pin 消息**;
`230047` 对同一条消息 Pin 触发限流; `230050` 消息对操作者不可见;
`230054` 该消息类型不支持本操作; `232009` 群组已解散。

### Unpin — `DELETE /open-apis/im/v1/pins/:message_id`

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v1/pins/:message_id` |
| HTTP Method | `DELETE` |
| 权限要求 | 与 Pin 相同（任一即可） |

前提与限制同 Pin。注意：**如果消息未被 Pin 或已被撤回，该接口返回成功**
（`{"code":0,"msg":"success","data":{}}`），因此「取消置顶」天然幂等。

对本项目的含义：

- 不需要为新能力单独申请权限 —— 发送卡片已经在用的 `im:message:send_as_bot` 就够；
- `230046` 是生产里最容易踩的错：群设置只允许群主/管理员置顶，而机器人通常只是普通成员。
  因此 R4 的失败必须是**非阻塞**的（只记日志与指标，告警照常投递）；
- 应答里的 `operator_id_type = app_id` 说明这条 Pin 是机器人自己做的，可用于排查。

---

## 5. Bot / channel discovery

### 5.1 List groups the bot/app is in — VERIFIED

Source: <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/chat/list.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v1/chats` |
| HTTP Method | `GET` |
| 权限要求 (任一) | 获取与更新群组信息(`im:chat`); 获取群组信息(`im:chat:readonly`); 查看群信息(`im:chat:read`); 读取群信息（历史版本）(`im:chat.group_info:readonly`) |
| 字段权限要求 | 获取用户 user ID(`contact:user.employee_id:readonly`) |

Query params (all of them):

| name | type | required | allowed values / notes |
|---|---|---|---|
| `user_id_type` | string | no | `open_id` \| `union_id` \| `user_id`; default `open_id`; used to type `owner_id` |
| `sort_type` | string | no | `ByCreateTimeAsc`（默认，按创建时间升序）\| `ByActiveTimeDesc`（按活跃时间降序；可能漏群） |
| `page_token` | string | no | 分页标记 |
| `page_size` | int | no | 默认 `20`, 最大 `100` |

注意: 该接口返回的群列表**不包含单聊**（`chat_mode` 为 `p2p`）; 如果获取机器人所在群列表，
`user_id_type` 可以不填.

Response `data`:

| name | type | description |
|---|---|---|
| `items[]` | list_chat[] | 群组列表 |
| `items[].chat_id` | string | 群组 ID |
| `items[].avatar` | string | 群头像 URL |
| `items[].name` | string | 群名称 |
| `items[].description` | string | 群描述 |
| `items[].owner_id` | string | 群主用户 ID（群主为机器人时无返回值） |
| `items[].owner_id_type` | string | `user_id` \| `open_id` \| `union_id` |
| `items[].external` | boolean | 是否是外部群 |
| `items[].tenant_key` | string | 租户 Key |
| `items[].chat_status` | string | `normal` \| `dissolved` \| `dissolved_save` |
| `page_token` | string | 当 `has_more` 为 true 时返回 |
| `has_more` | boolean | 是否还有更多项 |

The list response does **not** include `chat_mode`/`chat_type`; those are only on
[GET /open-apis/im/v1/chats/:chat_id](#52-get-one-group--verified). Enumerate `items[]` and match
`name` client-side (see also 5.3).

### 5.2 Get one group — VERIFIED

Source: <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/chat/get.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v1/chats/:chat_id` |
| HTTP Method | `GET` |
| 权限要求 (任一) | `im:chat`; `im:chat:read`; `im:chat:readonly` |
| 字段权限要求 | `contact:user.employee_id:readonly` |

Useful response fields (subset; all VERIFIED verbatim):

| name | type | description / enums |
|---|---|---|
| `avatar` | string | 群头像 URL |
| `name` | string | 群名称（没有设置群名称时不返回） |
| `description` | string | 群描述 |
| `i18n_names.zh_cn` / `.en_us` / `.ja_jp` | string | 群国际化名称 |
| `chat_mode` | string | `group` \| `topic` \| `p2p` |
| `chat_type` | string | `private` \| `public`（单聊不返回） |
| `chat_tag` | string | `inner` \| `tenant` \| `department` \| `edu` \| `meeting` \| `customer_service` |
| `external` | boolean | 是否是外部群 |
| `tenant_key` | string | 租户 Key |
| `owner_id_type` | string | `open_id` \| `user_id` \| `union_id`（群主为机器人或单聊时不返回） |
| `owner_id` | string | 群主用户 ID（同上） |
| `user_count` | string | 群内用户数量 |
| `bot_count` | string | 群内机器人数量 |
| `chat_status` | string | `normal` \| `dissolved` \| `dissolved_save` |
| `group_message_type` | string | `chat` \| `thread`（仅普通对话群） |
| `add_member_permission` | string | `only_owner` \| `all_members` |
| `share_card_permission` | string | `allowed` \| `not_allowed` |
| `at_all_permission` | string | `only_owner` \| `all_members` |
| `edit_permission` | string | `only_owner` \| `all_members` |
| `moderation_permission` | string | `only_owner` \| `all_members` \| `moderator_list` |
| `join_message_visibility` / `leave_message_visibility` | string | `only_owner` \| `all_members` \| `not_anyone` |
| `membership_approval` | string | `no_approval_required` \| `approval_required` |
| `restricted_mode_setting.status` | boolean | 保密模式是否开启 |
| `restricted_mode_setting.screenshot_has_permission_setting` | string | `all_members` \| `not_anyone` |
| `restricted_mode_setting.download_has_permission_setting` | string | `all_members` \| `not_anyone` |
| `restricted_mode_setting.message_has_permission_setting` | string | `all_members` \| `not_anyone` |
| `urgent_setting` / `video_conference_setting` / `hide_member_count_setting` | string | see doc |

使用限制: 调用接口的机器人/用户需要在群组内，否则只返回群名称、群头像、成员数量及群状态;
内部群要求同租户.

### 5.3 Resolve a chat by name — keyword search (no exact-name API) — VERIFIED

There is **no "get chat by name"** endpoint. The documented options are keyword search:

**(legacy, GET)**
Source: <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/chat/search.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v1/chats/search` |
| HTTP Method | `GET` |
| 权限要求 (任一) | `im:chat`; `im:chat:readonly`; `im:chat:read`; `im:chat.group_info:readonly` |
| Query | `user_id_type` (optional), `query` (keyword, ≤64 chars; 空值或>64字符返回空), `page_token`, `page_size` (默认20, 最大100) |

`query` supports 群国际化名称、群成员名称, 多语种, 拼音/前缀模糊搜索. Response items have the same
shape as the chat-list items (`chat_id`, `avatar`, `name`, `description`, `owner_id`,
`owner_id_type`, `external`, `tenant_key`, `chat_status`).

**(recommended newer, POST)**
Source: <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/group/im-v2/chat/search.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v2/chats/search` |
| HTTP Method | `POST` |
| 接口频率限制 | 100 次/分钟 |
| 权限要求 (任一) | `im:chat`; `im:chat:read`; `im:chat:readonly` |

Query params: `page_size`, `page_token`, `user_id_type`.
Body: `query` (string, 0–50 chars), `filter` (object), `sorter` (string).

`filter` fields: `search_types` (`private` \| `external` \| `public_joined` \|
`public_not_joined`), `member_ids` (string[]), `is_manager` (boolean),
`disable_search_by_user` (boolean), `chat_modes` (`default` 普通群 \| `thread` 话题群).
`sorter`: `create_time_desc` \| `update_time_desc` \| `member_count_desc`.

```json
{
    "query": "部门群",
    "filter": {
        "search_types": ["private"],
        "member_ids": ["ou-7890123456abcdef"],
        "is_manager": true,
        "disable_search_by_user": true,
        "chat_modes": ["thread"]
    },
    "sorter": "create_time_desc"
}
```

For a reliable mapping `chat name → chat_id`, the robust approach for this project is:
`GET /open-apis/im/v1/chats` (all groups the bot is in) and match `name` locally, falling back to
search only for groups the bot has not yet joined (search still requires visibility).

### 5.4 Users the bot can DM — VERIFIED (with caveats)

Endpoint: `GET https://open.feishu.cn/open-apis/contact/v3/users`
Source: <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/contact-v3/user/list.md?lang=zh-CN>

| | |
|---|---|
| HTTP Method | `GET` |
| 权限要求 (开启任一权限即可) | 获取部门组织架构信息(`contact:department.organize:readonly`); 以应用身份读取通讯录(`contact:contact:readonly_as_app`); 读取通讯录(`contact:contact:readonly`); 以应用身份访问通讯录(`contact:contact:access_as_app`) |
| 字段权限要求 | `contact:user.base:readonly`; `contact:contact:readonly_as_app`; `contact:user.employee:readonly`; `contact:user.department:readonly`; `contact:user.gender:readonly`; `contact:user.email:readonly`; `contact:user.phone:readonly`; `contact:user.employee_id:readonly`; `contact:contact:readonly`; `contact:contact:access_as_app` |

Query params: `user_id_type` (`open_id` \| `union_id` \| `user_id`, default `open_id`),
`department_id_type` (`department_id` \| `open_department_id`, default `open_department_id`),
`department_id` (optional), `page_token`, `page_size` (max `100`).

Response: `data.has_more`, `data.page_token`, `data.items[]` with `union_id`, `user_id`, `open_id`,
`name`, `en_name`, `email`, `mobile`, `mobile_visible`, `gender` (`0` 保密 / `1` 男 / `2` 女),
`avatar`, `status`, `department_ids`, `leader_user_id`, `city`, `country`, `work_station`,
`join_time`, `is_tenant_manager`, `employee_no`, `employee_type`, `orders`, `custom_attrs`,
`enterprise_email`, `job_title`.

Caveats (verbatim from the doc): **本接口已为历史版本，不再维护更新，不推荐使用。推荐你使用获取
部门直属用户列表（`contact-v3/user/find_by_department`）接口。** Also this endpoint is the
directory, **not** the bot's availability list — being in the directory does not imply the bot is
allowed to DM the user. The send API's real gate is the app's 可用范围
(`im:message` error `230013`). See [Gaps](#gaps--uncertainties).

---

## 6. CardKit

CardKit is the "card entity" model. A card entity is created once, referenced by `card_id` when
sending, and can then be updated (full / partial / streaming / per-element) through card and
element APIs.

Key limits (VERIFIED, from the create doc's 使用限制 and the streaming guide):

- 本接口仅支持**卡片 JSON 2.0 结构**或搭建工具搭建的**新版卡片**（JSON 2.0 是硬性要求; error
  `300303 Only schema 2.0 is supported`).
- 不支持将卡片设置为独享卡片模式，即 `update_multi` 不能为 `false`（error `300302`）。
- **一个卡片实体，仅支持发送一次。** (also: 发送卡片实体的应用必须是该卡片实体的创建应用)
- **卡片实体的有效期为 14 天。** 创建超出 14 天后无法再调用相关接口操作卡片（error `200750`）。
- 单张卡片 ≤ **30 KB**; ≤ **200** 个元素/组件; 单卡片实体的卡片/组件级 OpenAPI 操作频率上限
  **10 次/秒**.
- 所有 CardKit 接口权限: 创建与更新卡片(`cardkit:card:write`); 凭证: `tenant_access_token`.
- 调用更新接口的应用身份需与创建目标卡片实体的应用身份一致.

Method list (VERIFIED from <https://open.feishu.cn/document/cardkit-v1/feishu-card-resource-overview.md?lang=zh-CN>):

| method | path | scope |
|---|---|---|
| `POST` | `/open-apis/cardkit/v1/cards` | `cardkit:card:write` |
| `PUT` | `/open-apis/cardkit/v1/cards/:card_id` | `cardkit:card:write` |
| `PATCH` | `/open-apis/cardkit/v1/cards/:card_id/settings` | `cardkit:card:write` |
| `POST` | `/open-apis/cardkit/v1/cards/:card_id/batch_update` | `cardkit:card:write` |
| `POST` | `/open-apis/cardkit/v1/cards/:card_id/elements` | `cardkit:card:write` |
| `PUT` | `/open-apis/cardkit/v1/cards/:card_id/elements/:element_id` | `cardkit:card:write` |
| `PATCH` | `/open-apis/cardkit/v1/cards/:card_id/elements/:element_id` | `cardkit:card:write` |
| `PUT` | `/open-apis/cardkit/v1/cards/:card_id/elements/:element_id/content` | `cardkit:card:write` |
| `DELETE` | `/open-apis/cardkit/v1/cards/:card_id/elements/:element_id` | `cardkit:card:write` |

### 6.1 Create card entity — VERIFIED

Source: <https://open.feishu.cn/document/cardkit-v1/card/create.md?lang=zh-CN>

`POST https://open.feishu.cn/open-apis/cardkit/v1/cards`

Request body:

| name | type | required | description |
|---|---|---|---|
| `type` | string | **yes** | `card_json`（由卡片 JSON 代码构建）\| `template`（由搭建工具搭建的卡片模板）; 1–50 chars |
| `data` | string | **yes** | 卡片数据（转义后的字符串）; `type=card_json` → 卡片 JSON 代码（必须声明 `schema: "2.0"`）; `type=template` → 模板数据（仅支持新版卡片）; 1–3,000,000 chars |

```json
{  // type 为 card_json 时
  "type": "card_json",
  "data": "{\"schema\":\"2.0\",\"header\":{\"title\":{\"content\":\"项目进度更新提醒\",\"tag\":\"plain_text\"}},\"config\":{\"streaming_mode\":true,\"summary\":{\"content\":\"\"},\"streaming_config\":{\"print_frequency_ms\":{\"default\":70,\"android\":70,\"ios\":70,\"pc\":70},\"print_step\":{\"default\":1,\"android\":1,\"ios\":1,\"pc\":1},\"print_strategy\":\"fast\"}},\"body\":{\"elements\":[{\"tag\":\"markdown\",\"content\":\"截至今日，项目完成度已达80%\",\"element_id\":\"markdown_1\"}]}}"
}
```

```json
{  // type 为 template 时
  "type": "template",
  "data": "{\"template_id\":\"AAqIi1B8abcef\",\"template_version_name\":\"1.0.0\",\"template_variable\":{\"open_id\":\"ou_5c6d1637498e704f541095bba3dabcef\"}}"
}
```

Response: `data.card_id` (string), e.g. `"7355372766134157313"`.

```json
{ "code": 0, "msg": "success", "data": { "card_id": "7355372766134157313" } }
```

Errors: `10002`; `200860` 卡片体积超限 (≤30KB); `300301` duplicate `element_id`; `300302`
`update_multi` is false; `300303` Only schema 2.0 is supported; `200220`; `300305` >200 components;
`300307` DSL empty.

### 6.2 Full update of a card entity (全量更新卡片实体) — VERIFIED

Source: <https://open.feishu.cn/document/cardkit-v1/card/update.md?lang=zh-CN>

`PUT https://open.feishu.cn/open-apis/cardkit/v1/cards/:card_id`

Path: `card_id` (1–20 chars).

Body:

| name | type | required | description |
|---|---|---|---|
| `card` | card | **yes** | 更新后的完整卡片 JSON 内容 |
| `card.type` | string | **yes** | 固定 `card_json` |
| `card.data` | string | **yes** | 卡片 JSON 数据（仅 JSON 2.0；转义字符串；1–1,000,000 chars） |
| `uuid` | string | no | 幂等 ID（1–64 chars），相同批次只执行一次 |
| `sequence` | int | **yes** | 操作卡片的序号，须严格递增；int32 `1`~`2147483647` |

```json
{
    "card": {
        "type": "card_json",
        "data": "{\"schema\":\"2.0\",\"header\":{\"title\":{\"content\":\"项目进度更新提醒\",\"tag\":\"plain_text\"}},\"body\":{\"elements\":[{\"tag\":\"markdown\",\"content\":\"截至今日，项目完成度已达80%\"}]}}"
    },
    "uuid": "a0d69e20-1dd1-458b-k525-dfeca4015204",
    "sequence": 1
}
```

Response: `data` is `{}`. Errors: `200740` not exist; `200750` expired; `200770` UUID conflict;
`200810` card in interaction; `200860`; `300301`; `300302`; `300303`; `300305`; `300307`; `300311`
wrong app; `300317` sequence not increasing.

### 6.3 Partial update (局部更新卡片实体 / batch update) — VERIFIED

Source: <https://open.feishu.cn/document/cardkit-v1/card/batch_update.md?lang=zh-CN>

`POST https://open.feishu.cn/open-apis/cardkit/v1/cards/:card_id/batch_update`

Body:

| name | type | required | description |
|---|---|---|---|
| `uuid` | string | no | 幂等 ID (1–64 chars) |
| `sequence` | int | **yes** | 严格递增的 int32 `1`~`2147483647` |
| `actions` | string | **yes** | 操作列表（JSON 数组转义后的字符串，1–1,000,000 chars） |

`actions` entries are objects `{"action": "<op>", "params": {…}}`. Documented ops:

- `partial_update_setting` — 更新卡片配置，支持更新卡片的 `config` 和 `card_link` 字段（结构同 6.6）.
- `add_elements` — 添加组件，支持 `type`、`target_element_id`、`elements`（结构同 6.7）.
- `delete_elements` — 删除组件，支持 `element_ids`（组件 ID 数组）.
- `partial_update_element` — 更新组件属性，支持 `element_id` 和 `partial_element`（结构同 6.5）.
- `update_element` — 全量更新组件，支持 `element_id` 和 `element`（结构同 6.4）.

```json
{
    "uuid": "a0d69e20-1dd1-458b-k525-dfeca4015204",
    "sequence": 1,
    "actions": "[{\"action\":\"partial_update_setting\",\"params\":{\"settings\":{\"config\":{\"streaming_mode\":true}}}},{\"action\":\"add_elements\",\"params\":{\"type\":\"insert_before\",\"target_element_id\":\"markdown_1\",\"elements\":[{\"tag\":\"markdown\",\"element_id\":\"md_1\",\"content\":\"欢迎使用飞书卡片搭建工具。\"}]}},{\"action\":\"delete_elements\",\"params\":{\"element_ids\":[\"text_1\",\"text_2\"]}},{\"action\":\"partial_update_element\",\"params\":{\"element_id\":\"markdown_2\",\"partial_element\":{\"content\":\"详情参考飞书卡片相关文档。\"}}},{\"action\":\"update_element\",\"params\":{\"element_id\":\"markdown_3\",\"element\":{\"tag\":\"button\",\"text\":{\"tag\":\"plain_text\",\"content\":\"有帮助\"},\"size\":\"medium\",\"icon\":{\"tag\":\"standard_icon\",\"token\":\"emoji_outlined\"}}}}]"
}
```

Response: `data` is `{}`. Errors add `300312` (cannot update element `tag`), `300313`, `300314`,
`300315`, `300121`, `300122`.

**`sequence` / `uuid` semantics (VERIFIED):** `sequence` guarantees update ordering for one card;
它必须相较于上一次操作**严格递增**(error `300317`). `uuid` is an idempotency key: passing the same
UUID guarantees the same batch is executed only once (conflict → `200770`). `uuid` is optional;
`sequence` is required on every card-mutating CardKit call.

### 6.4 Update (replace) an element (更新组件) — VERIFIED

Source: <https://open.feishu.cn/document/cardkit-v1/card-element/update.md?lang=zh-CN>

`PUT https://open.feishu.cn/open-apis/cardkit/v1/cards/:card_id/elements/:element_id`

Body: `element` (string, **yes**, 完整组件 JSON 转义字符串, 仅 JSON 2.0), `uuid` (no), `sequence`
(**yes**).

```json
{
    "uuid": "a0d69e20-1dd1-458b-k525-dfeca4015204",
    "element": "{\"tag\":\"markdown\",\"element_id\":\"md_1\",\"content\":\"这是一段更新后的文本\"}",
    "sequence": 1
}
```

### 6.5 Partial update of element attributes (更新组件属性) — VERIFIED

Source: <https://open.feishu.cn/document/cardkit-v1/card-element/patch.md?lang=zh-CN>

`PATCH https://open.feishu.cn/open-apis/cardkit/v1/cards/:card_id/elements/:element_id`

Body: `partial_element` (string, **yes**, 组件的新配置项字段转义字符串; **不支持修改 `tag`**),
`uuid` (no), `sequence` (**yes**).

```json
{
    "partial_element": "{\"content\":\"更新后的组件文本\"}",
    "uuid": "a0d69e20-1dd1-458b-k525-dfeca4015204",
    "sequence": 1
}
```

### 6.6 Streaming / full text update on one element (流式更新文本) — VERIFIED

Source: <https://open.feishu.cn/document/cardkit-v1/card-element/content.md?lang=zh-CN>

`PUT https://open.feishu.cn/open-apis/cardkit/v1/cards/:card_id/elements/:element_id/content`

Targets a `tag: plain_text` element or a `tag: markdown` component (搭建工具卡片仅支持对富文本
组件的 `element_id`). Requires `streaming_mode: true` in the card (else `300309 Card streaming
closed`).

Body: `content` (string, **yes**, 新的全量文本, 1–100,000 chars), `uuid` (no), `sequence`
(**yes**).

```json
{
    "uuid": "a0d69e20-1dd1-458b-k525-dfeca4015204",
    "content": "这是更新后的文本内容。将以打字机式的效果输出",
    "sequence": 1
}
```

Behaviour: 若旧文本为传入的新文本的前缀子串，新增文本在旧文本末尾以打字机效果输出；若新旧文本
前缀不同，全量文本直接上屏（无打字机效果）。

Errors include `200850` / `200510` streaming timeout, `300309` streaming closed, `300310` Only
update text, `300120`.

### 6.7 Add elements (新增组件) — VERIFIED

Source: <https://open.feishu.cn/document/cardkit-v1/card-element/create.md?lang=zh-CN>

`POST https://open.feishu.cn/open-apis/cardkit/v1/cards/:card_id/elements`

Body:

| name | type | required | description |
|---|---|---|---|
| `type` | string | **yes** | `insert_before` \| `insert_after` \| `append` |
| `target_element_id` | string | no | `insert_before`/`insert_after` 时必填; `append` 时仅支持容器类组件，缺省则在卡片 body 末尾添加 |
| `uuid` | string | no | 幂等 ID |
| `sequence` | int | **yes** | 严格递增 |
| `elements` | string | **yes** | 添加的组件列表（JSON 数组转义字符串，仅 JSON 2.0） |

```json
{
    "type": "insert_after",
    "target_element_id": "markdown_1",
    "uuid": "a0d69e20-1dd1-458b-k525-dfeca4015204",
    "sequence": 1,
    "elements": "[{\"tag\":\"button\",\"element_id\":\"button_1\",\"text\":{\"tag\":\"plain_text\",\"content\":\"查看更多\"},\"type\":\"default\",\"width\":\"default\",\"size\":\"medium\",\"behaviors\":[{\"type\":\"open_url\",\"default_url\":\"https://open.feishu.cn/?lang=zh-CN\",\"pc_url\":\"\",\"ios_url\":\"\",\"android_url\":\"\"}]}]"
}
```

### 6.8 Delete element (删除组件) — VERIFIED

Source: <https://open.feishu.cn/document/cardkit-v1/card-element/delete.md?lang=zh-CN>

`DELETE https://open.feishu.cn/open-apis/cardkit/v1/cards/:card_id/elements/:element_id`

Body: `uuid` (no), `sequence` (**yes**). 删除容器类组件时，容器中内嵌的组件将一并被删除。

```json
{ "uuid": "a0d69e20-1dd1-458b-k525-dfeca4015204", "sequence": 1 }
```

### 6.9 Update card settings (更新卡片配置, incl. `streaming_mode`) — VERIFIED

Source: <https://open.feishu.cn/document/cardkit-v1/card/settings.md?lang=zh-CN>

`PATCH https://open.feishu.cn/open-apis/cardkit/v1/cards/:card_id/settings`

Body:

| name | type | required | description |
|---|---|---|---|
| `settings` | string | **yes** | 卡片配置相关字段转义后的字符串，包括 `config` 和 `card_link` 字段（仅 JSON 2.0 对应字段; 1–100,000 chars） |
| `uuid` | string | no | 幂等 ID |
| `sequence` | int | **yes** | 严格递增 |

```json
{
    "settings": "{\"config\":{\"streaming_mode\":true,\"streaming_config\":{\"print_frequency_ms\":{\"default\":70,\"android\":70,\"ios\":70,\"pc\":70},\"print_step\":{\"default\":1,\"android\":1,\"ios\":1,\"pc\":1},\"print_strategy\":\"fast\"}}}",
    "uuid": "a0d69e20-1dd1-458b-k525-dfeca4015204",
    "sequence": 1
}
```

Turn streaming off with `{"settings":"{\"config\":{\"streaming_mode\":false}}","uuid":"…","sequence":1}`.

Streaming notes (VERIFIED from the streaming guide): `streaming_config.print_frequency_ms`
(default 70 ms) and `print_step` (default 1 char) support per-platform keys `default` (required when
customising), `android`, `ios`, `pc`. `print_strategy` is `fast`（默认，快速上屏）or `delay`（延迟
上屏）. In streaming mode the QPS limit on card/element updates is lifted. **流式更新模式将在距上次
开启 10 分钟后自动关闭，建议自行手动关闭。** While a card is in streaming mode its chat-list
preview shows `[生成中...]`.

### 6.10 Which update API applies to what — VERIFIED

Source: <https://open.feishu.cn/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/update-feishu-card.md?lang=zh-CN>

| 更新方式 | 适用场景 | 相关接口 |
|---|---|---|
| 全量更新 | 内容差异较大 | 更新应用发送的消息卡片（`PATCH /open-apis/im/v1/messages/:message_id`）; 延时更新消息卡片; 全量更新卡片实体（`PUT /open-apis/cardkit/v1/cards/:card_id`） |
| 局部更新 | 部分内容修改 | 卡片级: 更新卡片配置（settings）; 批量更新卡片实体（batch_update）。组件级: 新增组件、更新组件、更新组件属性、删除组件 |
| 文本流式更新 | 打字机效果 | 流式更新文本（`PUT …/elements/:element_id/content`） |

- **已发送的卡片** can be updated either via the message API by `message_id`
  (`PATCH /open-apis/im/v1/messages/:message_id`, within 14 days of sending) **or** via the CardKit
  entity APIs by `card_id` (the sent message references the entity).
- **卡片实体** (`card_id`) is operated with the CardKit APIs; it must be created by the same app
  that updates it and is valid for 14 days.
- A card entity can be **sent only once**; there is no documented way to send the same `card_id`
  twice.
- `卡片的更新方式` limitations: message PATCH requires `update_multi: true` before and after, is ≤30KB,
  ≤14 days, 5 QPS per message.

---

## 7. App feed card (应用消息流卡片, `im/v2`)

> ⛔ **本项目已不再使用这组接口**（2026-10 移除「应用消息流消息」形态，见
> `migrations/0002_remove_app_feed_card.sql`）。本节仅作为 API 调研记录保留。
> 需要恢复该形态时，注意它只能投递给用户（无群维度）且字段结构与卡片 JSON 完全不通用。

All three endpoints share scope 创建、更新、删除应用消息流卡片(`im:app_feed_card:write`) and the
same `failed_cards` response shape. Client support: 飞书客户端 V7.6 及以上.

There is **no separate batch-create** endpoint: `POST /open-apis/im/v2/app_feed_card` already takes a
`user_ids` array (1–20). Update and delete are batch endpoints keyed by `(biz_id, user_id)`.

### 7.1 Create — VERIFIED

Source: <https://open.feishu.cn/document/im-v2/app_feed_card/create.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v2/app_feed_card` |
| HTTP Method | `POST` |
| 频率限制 | 10 次/秒 |
| 权限要求 (任一) | 创建、更新、删除应用消息流卡片(`im:app_feed_card:write`) |
| 字段权限要求 | 获取用户 user ID(`contact:user.employee_id:readonly`) |
| Query | `user_id_type`: `open_id`（默认）\| `union_id` \| `user_id` |

Body:

| name | type | required | description |
|---|---|---|---|
| `app_feed_card` | open_app_feed_card | no | 应用消息卡片 |
| `app_feed_card.biz_id` | string | no | 业务 ID（不传则系统自动分配并返回）0–200 chars |
| `app_feed_card.title` | string | no | 主标题 0–60 chars |
| `app_feed_card.avatar_key` | string | no | 头像 key 0–100 chars |
| `app_feed_card.preview` | string | no | 预览信息 0–120 chars |
| `app_feed_card.status_label` | open_feed_status_label | no | 状态标签（不传则默认展示卡片触达时间） |
| `app_feed_card.status_label.text` | string | **yes** | 标签文字 1–20 chars |
| `app_feed_card.status_label.type` | string | **yes** | `primary`（默认）\| `secondary` \| `success` \| `danger` |
| `app_feed_card.buttons` | open_app_feed_card_buttons | no | 交互按钮（最多 2 个） |
| `app_feed_card.buttons.buttons` | open_app_feed_card_button[] | **yes** | 按钮组合 0–2 |
| `…buttons.buttons[].multi_url` | open_app_feed_card_url | no | 跳转 URL（仅 https） |
| `…multi_url.url` | string | no | 默认 URL ≤700 |
| `…multi_url.android_url` | string | no | Android URL ≤700 |
| `…multi_url.ios_url` | string | no | iOS URL ≤700 |
| `…multi_url.pc_url` | string | no | PC URL ≤700 |
| `…buttons[].action_type` | string | **yes** | `url_page`（URL 页面）\| `webhook`（回调） |
| `…buttons[].text` | open_app_feed_card_text | **yes** | 文字 |
| `…buttons[].text.text` | string | **yes** | 文本 1–30 chars |
| `…buttons[].button_type` | string | no | `default`（默认）\| `primary` \| `success` |
| `…buttons[].action_map` | map&lt;string, string&gt; | no | action 字典 |
| `app_feed_card.link` | open_app_feed_link | no | 卡片整体跳转链接（**创建时该参数为必填参数**，尽管必填列写“否”） |
| `app_feed_card.link.link` | string | no | 链接（仅 HTTPS / 网页应用或小程序 Applink）≤700 |
| `app_feed_card.time_sensitive` | boolean | no | 即时提醒状态; 默认 `false` |
| `app_feed_card.notify` | app_feed_notify | no | 通知设置，为空时默认进行通知 |
| `app_feed_card.notify.close_notify` | boolean | no | 是否关闭通知，默认 `false` |
| `app_feed_card.notify.custom_sound_text` | string | no | 自定义语音播报文本（仅移动端）≤20 |
| `app_feed_card.notify.with_custom_sound` | boolean | no | 是否播报自定义语音（仅移动端），默认 `false` |
| `user_ids` | string[] | no | 用户 ID 列表（类型与 `user_id_type` 一致）; 长度 1–20 |

`time_sensitive` semantics (verbatim): **设置为 `true` 后，卡片在消息列表临时置顶；设置为 `false`，
消息卡片不置顶。**

```json
{
    "app_feed_card": {
        "biz_id": "096e2927-40a6-41a3-9562-314d641d09ae",
        "title": "主标题",
        "avatar_key": "v3_0041_007bca9f-67ba-4199-bf00-4031b12cf226",
        "preview": "预览信息",
        "status_label": { "text": "标签文字", "type": "primary" },
        "buttons": {
            "buttons": [
                {
                    "multi_url": {
                        "url": "https://www.feishu.cn/",
                        "android_url": "https://www.feishu.cn/",
                        "ios_url": "https://www.feishu.cn/",
                        "pc_url": "https://www.feishu.cn/"
                    },
                    "action_type": "url_page",
                    "text": { "text": "文本" },
                    "button_type": "default",
                    "action_map": { "foo": "bar" }
                }
            ]
        },
        "link": { "link": "https://www.feishu.cn/" },
        "time_sensitive": false,
        "notify": {
            "close_notify": true,
            "custom_sound_text": "您有新的订单",
            "with_custom_sound": true
        }
    },
    "user_ids": ["ou_a0553eda9014c201e6969b478895c230"]
}
```

Response:

| name | type | description |
|---|---|---|
| `code` | int | 错误码，非 0 表示失败 |
| `msg` | string | 错误描述 |
| `data.failed_cards[]` | open_failed_user_app_feed_card_item[] | 失败的卡片 |
| `data.failed_cards[].biz_id` | string | 业务 ID |
| `data.failed_cards[].user_id` | string | 用户 ID |
| `data.failed_cards[].reason` | string | `0` 未知 \| `1` 无权限 \| `2` 未创建 \| `3` 频率限制 \| `4` 重复（examples show names `NOT_CREATED`, `NO_PERMISSION`） |
| `data.biz_id` | string | 卡片业务 ID（系统分配或回显） |

```json
{
    "code": 0,
    "msg": "success",
    "data": {
        "failed_cards": [
            { "biz_id": "bdf22389-87ec-4890-9eb6-78a7efaeecbb", "user_id": "ou_88553eda9014c201e6969b478895c223", "reason": "NOT_CREATED" }
        ],
        "biz_id": "b90ce43a-fca8-4f42-92f4-794bff206ee5"
    }
}
```

Error: HTTP 400 / `230001` `param is invalid`.

### 7.2 Update / replace an already-sent card by `biz_id` — VERIFIED

Source: <https://open.feishu.cn/document/im-v2/app_feed_card/update.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v2/app_feed_card/batch` |
| HTTP Method | `PUT` |
| 频率限制 | 10 次/秒 |
| 权限要求 | `im:app_feed_card:write` |
| Query | `user_id_type` |

**字段更新策略**: 具体更新的字段以 `update_fields` 为准，根据 `update_fields` 代表的字段从
`app_feed_card` 中取出来进行更新，不在 `update_fields` 中的字段不会更新.

Body:

| name | type | required | description |
|---|---|---|---|
| `feed_cards` | user_open_app_feed_card_updater[] | no | 应用消息卡片; 1–20 |
| `feed_cards[].app_feed_card` | open_app_feed_card | **yes** | 同 7.1 的字段集合 |
| `feed_cards[].user_id` | string | **yes** | 用户 ID（类型与 `user_id_type` 一致） |
| `feed_cards[].update_fields` | string[] | **yes** | 更新字段列表; 1–20; 见下表 |

`update_fields` enum values (all of them):

| value | meaning |
|---|---|
| `1` | 标题 |
| `2` | 头像 key |
| `3` | 预览 |
| `10` | 状态标签 |
| `11` | 按钮 |
| `12` | 跳转链接 |
| `13` | 即时提醒状态 |
| `101` | 展示时间更新到当前 |
| `102` | 排序时间更新到当前 |
| `103` | 进行通知 |

```json
{
    "feed_cards": [
        {
            "app_feed_card": {
                "biz_id": "096e2927-40a6-41a3-9562-314d641d09ae",
                "title": "主标题",
                "avatar_key": "v3_0041_007bca9f-67ba-4199-bf00-4031b12cf226",
                "preview": "预览信息",
                "status_label": { "text": "标签文字", "type": "primary" },
                "buttons": { "buttons": [ { "action_type": "url_page", "text": { "text": "文本" }, "button_type": "default" } ] },
                "link": { "link": "https://www.feishu.cn/" },
                "time_sensitive": false,
                "notify": { "close_notify": true, "custom_sound_text": "您有新的订单", "with_custom_sound": true }
            },
            "user_id": "ou_a0553eda9014c201e6969b478895c230",
            "update_fields": ["1"]
        }
    ]
}
```

Response: same `data.failed_cards[]` shape as 7.1.

### 7.3 Delete — VERIFIED

Source: <https://open.feishu.cn/document/im-v2/app_feed_card/delete.md?lang=zh-CN>

| | |
|---|---|
| HTTP URL | `https://open.feishu.cn/open-apis/im/v2/app_feed_card/batch` |
| HTTP Method | `DELETE` |
| 频率限制 | 10 次/秒 |
| 权限要求 | `im:app_feed_card:write` |
| Query | `user_id_type` |

Body:

| name | type | required | description |
|---|---|---|---|
| `feed_cards` | user_open_app_feed_card_deleter[] | no | 应用消息卡片; 1–20 |
| `feed_cards[].biz_id` | string | **yes** | 业务 ID |
| `feed_cards[].user_id` | string | **yes** | 用户 ID |

```json
{
    "feed_cards": [
        { "biz_id": "ed381d34-49ac-4876-8d9e-23447acb587e", "user_id": "ou_88553eda9014c201e6969b478895c223" }
    ]
}
```

Response: same `data.failed_cards[]` shape as 7.1.

---

## 8. Card JSON 2.0 structure essentials — VERIFIED

Source: <https://open.feishu.cn/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/card-json-v2-structure.md?lang=zh-CN>

Top level (2.0): `schema`, `config`, `card_link`, `header`, `body`. (`i18n_elements` and top-level
`elements` are the 1.0 shape — see the note at the end of this section.)

```json
{
    "schema": "2.0",
    "config": { "...": "..." },
    "card_link": { "url": "https://www.baidu.com", "android_url": "", "ios_url": "", "pc_url": "" },
    "header": { "title": { "tag": "plain_text", "content": "示例标题" } },
    "body": { "elements": [ { "tag": "markdown", "content": "…", "element_id": "markdown_1" } ] }
}
```

`schema`: `1.0`（默认，不写即 1.0）\| `2.0`（必须显式声明）。

`config` fields (all optional unless stated):

| field | type | default | description / enums |
|---|---|---|---|
| `streaming_mode` | Boolean | `false` | 卡片是否处于流式更新模式 |
| `streaming_config` | object | – | 流式更新配置 |
| `streaming_config.print_frequency_ms` | object | 70 ms | keys `default`(必填 if custom)、`android`、`ios`、`pc` |
| `streaming_config.print_step` | object | 1 | keys `default`(必填 if custom)、`android`、`ios`、`pc` |
| `streaming_config.print_strategy` | enum | `fast` | `fast`（快速上屏）\| `delay`（延迟上屏） |
| `summary` | Object | – | 自定义摘要信息配置（聊天栏消息预览文案） |
| `summary.content` | String | 无 | 摘要文本；`streaming_mode=true` 时默认为“生成中” |
| `summary.i18n_content` | Object | – | 摘要文本的多语言配置 |
| `locales` | string[] | – | JSON 2.0 新增；用于指定生效的语言；配置后仅其中的语言生效 |
| `enable_forward` | Boolean | `true` | 是否允许转发卡片 |
| `update_multi` | Boolean | `true` | 是否为共享卡片；**JSON 2.0 暂时仅支持设为 `true`** |
| `width_mode` | String | `default` | `default`（PC 宽版/iPad 上限 600px）\| `compact`（400px）\| `fill`（自适应）；搭建工具暂不支持 |
| `use_custom_translation` | Boolean | `false` | 是否使用自定义翻译数据 |
| `enable_forward_interaction` | Boolean | `false` | 转发的卡片是否仍支持回传交互 |
| `style` | Object | 空 | 自定义字号/颜色 |
| `style.text_size` | Object | 空 | 形如 `"cus-0": { "default": "medium", "pc": "medium", "mobile": "large" }` |
| `style.color` | Object | 空 | 形如 `"cus-0": { "light_mode": "rgba(...)", "dark_mode": "rgba(...)" }` |

`card_link`: `url` (default), `pc_url`, `ios_url`, `android_url`. Note: **`url` 和各端链接必填其中
一个**; 若只填 `url` 则它生效; 若三个端链接都填则必须三个全填; 禁止某端跳转可用
`lark://msgcard/unsupported_action`.

`header`:

| field | description / enums |
|---|---|
| `title.tag` | `plain_text` \| `lark_md` |
| `title.content` | 标题内容（必填） |
| `subtitle.tag` / `subtitle.content` | 可选副标题 |
| `text_tag_list[]` | 标题后缀标签，最多 3 个；元素含 `tag:"text_tag"`, `element_id`, `text{tag,content}`, `color` |
| `i18n_text_tag_list` | 多语言标题后缀标签，keys `zh_cn`/`en_us`/`ja_jp`/`zh_hk`/`zh_tw` |
| `template` | `blue` \| `wathet` \| `turquoise` \| `green` \| `yellow` \| `orange` \| `red` \| `carmine` \| `violet` \| `purple` \| `indigo` \| `grey` \| `default`（默认） |
| `icon.tag` | `standard_icon`（用 `token`,`color`）\| `custom_icon`（用 `img_key`） |
| `padding` | JSON 2.0 新增，默认 `"12px"`，范围 `[0,99]px` |

`body`:

| field | description / enums |
|---|---|
| `direction` | `vertical`（默认）\| `horizontal` |
| `padding` | `[0,99]px` |
| `horizontal_spacing` | `small`(4px) \| `medium`(8px) \| `large`(12px) \| `extra_large`(16px) 或 `[0,99]px` |
| `horizontal_align` | `left`（默认）\| `center` \| `right` |
| `vertical_spacing` | `small` \| `medium` \| `large` \| `extra_large` 或 `[0,99]px` |
| `vertical_align` | `top`（默认）\| `center` \| `bottom` |
| `elements[]` | 组件数组；每个组件有 `tag`, `margin`（默认 `"0"`, `[-99,99]px`）, `element_id` |

`element_id` rules: 在同一张卡片内全局唯一；仅允许字母、数字和下划线；必须以字母开头；≤20
字符.

Other hard limits: 最多 200 个元素/组件; `update_multi` 仅支持 `true`; 支持飞书客户端 7.20+.

### 8.1 Template variables (`template_id` + `template_variable`)

The **documented** mechanism is: bind variables in the builder, publish the card, get
`template_id` (+ version), then pass values via `template_variable` (an object of
`variable name → value`). See §2.4(b), §6.1 (`type: "template"`) and the variable doc:
<https://open.feishu.cn/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/feishu-card-cardkit/configure-card-variables.md?lang=zh-CN>

```json
{
    "type": "template",
    "data": {
        "template_id": "AAqi6xJ8rabcd",
        "template_version_name": "1.0.0",
        "template_variable": { "summary_ticket": "10", "summary_hours": "10", "object_list_1": [ { "name": "王冰" } ] }
    }
}
```

Variable-value types (新版卡片): 普通文本、整数、图片、日期、时间、日期时间、人员、人员数组、
选项数组、对象数组, etc. 变量名称须以字母开头，仅支持字母、数字或下划线.

> `${var}` / `$var`-style placeholders in the card JSON source: **not found in any official doc I
> fetched.** The builder exposes a 变量 UI and the exported/`template_variable` mechanism above; the
> literal `${…}` syntax is unverified. See [Gaps](#gaps--uncertainties).

### 8.2 `i18n_elements` and version difference

- `i18n_elements` / `i18n_header` / top-level `elements` belong to **card JSON 1.0**
  (<https://open.feishu.cn/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/card-json-structure.md?lang=zh-CN>).
- **JSON 2.0 不再支持通过 `i18n_elements` 字段设置全局多语言。** Use `config.locales` plus
  component-level `i18n_*` fields. Source:
  <https://open.feishu.cn/document/feishu-cards/card-json-v2-breaking-changes-release-notes.md?lang=zh-CN>
  (1.0 shape: `schema`/`config`/`card_link`/`header`/`i18n_header`/`elements`/`i18n_elements`/`fallback`;
  2.0 shape: `schema`/`config`/`card_link`/`header`/`body.elements`/`fallback`).
- `update_multi` default differs: 1.0 default `false`; 2.0 default `true` and 仅支持 `true`.
- Error `200861`: cards of schema V2 no longer support some schema-1 tags (废弃组件).

---

## 9. 卡片搭建工具 (Card Builder / cardkit) overview — VERIFIED

Source: <https://open.feishu.cn/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/feishu-card-cardkit/feishu-cardkit-overview.md?lang=zh-CN>

Tool entry: <https://open.feishu.cn/cardkit>. Two card generations:

- **新版卡片**: 基于 卡片 JSON 2.0 结构; 新增 20+ 能力; 推荐.
- **旧版卡片**: 基于 卡片 JSON 1.0 结构.

What a user exports / imports (Source:
<https://open.feishu.cn/document/feishu-cards/feishu-card-cardkit/import-and-export-cards.md?lang=zh-CN>):

- **导出**: 搭建工具**仅支持导出 `*.card` 文件**, via 卡片编辑页右上角 **… > 导出卡片**. 卡片
  **源代码** (card JSON) 可在编辑页下方「卡片源代码」处查看并复制.
- **导入**: 支持 `.card` 文件和**旧版消息卡片搭建工具**导出的 `.json` 文件. 从旧版工具导出时
  必须点击「导出卡片」获取 JSON 文件（直接复制卡片 JSON 会导致导入失败）. 旧版变量占位符需在旧版
  工具中已声明变量并填充模拟数据，否则变量在新版工具中失效. 导入后卡片 ID 与旧版不同，需重新获取
  卡片模板 ID.
- 卡片有 **版本管理**: 发布时指定版本号; 发送时可指定 `template_version_name` 固定版本.
- 卡片权限/协作: 可为卡片添加协作者、添加应用或自定义机器人以发布/使用该卡片（否则发送报
  `200381 template is not visible to app`）.

**导入路径 for a third-party tool:** the documented path is to produce either a `.card` file or a
legacy `.json` file and use the builder's **导入卡片** action (UI only — there is no public "import
card file" OpenAPI). A third-party tool that wants programmatic cards should emit **card JSON
(schema 2.0)** and either (a) send it inline via `POST /open-apis/im/v1/messages`, or (b) create a
card entity via `POST /open-apis/cardkit/v1/cards`; template IDs can only be produced by the builder.

**Is JSON 2.0 required for CardKit APIs?** Yes. All CardKit card/element APIs documented here accept
only 卡片 JSON 2.0 (`300303 Only schema 2.0 is supported`), and this is stated in every CardKit
endpoint's 使用限制. Inline card JSON sent directly via the message API may still be JSON 1.0, but
card entities and their update APIs require 2.0. (Source: create/update/settings/batch_update/
element docs + the resource overview note 「下列接口仅支持 卡片 JSON 2.0 结构」.)

---

## 10. Bot overview & prerequisites — VERIFIED

Sources:
<https://open.feishu.cn/document/client-docs/bot-v3/bot-overview.md?lang=zh-CN>,
<https://open.feishu.cn/document/ukTMukTMukTM/uATM04CMxQjLwEDN.md?lang=zh-CN>（用户如何在飞书中使用机器人）,
<https://open.feishu.cn/document/home/introduction-to-scope-and-authorization/availability.md?lang=zh-CN>.

- Two types: **应用机器人** (created in 开发者后台 <https://open.feishu.cn/app> by enabling 机器人
  能力; API-capable) and **自定义机器人** (webhook only, in-group, cannot call these APIs).
- 应用机器人 requires 发布应用 / 企业管理员审核 before config changes (能力、权限、事件) take effect.
- 用户使用方式: 把自己可见的机器人添加到群里（仅支持飞书电脑端; `···` > 设置 > 群机器人 >
  添加机器人）; 或搜索机器人名称单聊. 添加后机器人可往群里发消息、群成员可 @ 机器人.
- **机器人必须进群才能给该群发消息** — the send-message 前提条件 states: 给群组发送消息时，机器人
  需要在该群组中，且在群组内拥有发言权限 (error `230002` otherwise).
- **单聊** requires the target user to be inside the app's **可用范围** (error `230013` otherwise).
  可用范围 is defined as 哪些用户群体（组织、用户组、个人）可以使用该应用; configured by the developer
  at publish time and/or by the tenant admin in 管理后台; changes require re-publishing.
- 应用机器人 vs 自定义机器人 capability table: 应用机器人 supports 向用户发送单聊消息、创建群/管理群/
  获取群信息、访问通讯录、卡片交互; 自定义机器人 only 单向群消息 + 打开链接交互.

Scopes needed for this project (all VERIFIED from the endpoint pages above):

| scope | needed for |
|---|---|
| `im:message` | 发送/撤回/更新消息（任一即可） |
| `im:message:send_as_bot` | 以应用身份发消息（推荐做发送的主 scope） |
| `im:chat:readonly` (or `im:chat` / `im:chat:read`) | 列出/获取群、搜索群 |
| `im:message:recall` | 撤回（可选，`im:message` also works） |
| `im:message:update` | PATCH 已发送卡片（可选，`im:message` also works） |
| `im:app_feed_card:write` | 创建/更新/删除应用消息流卡片 |
| `cardkit:card:write` | 创建/更新卡片实体与组件 |
| `contact:user.employee_id:readonly` | 仅在需要 `user_id` 类型字段/参数时（字段权限） |
| `contact:contact:readonly_as_app` (or the other three contact scopes) | `GET /open-apis/contact/v3/users` |
| `im:message:send_sys_msg` | 仅 `msg_type=system` 需要 |

Doc note about 机器人进群/可见范围: 机器人需要被添加到群组，且目标用户需在机器人可用范围内;
管理员可在管理后台按部门/用户组/用户调整可用范围（无需重新审核）；开发者修改可用范围需重新发版.

---

## Gaps / uncertainties

These could **not** be verified from the official documentation I fetched. The implementer must treat
them as unconfirmed:

1. **`content = {"type":"card","data":{"card":{...}}}` (raw card JSON nested under `data.card`) is
   NOT documented.** Only `{"type":"card","data":{"card_id":"…"}}` (entity reference) and
   `{"type":"template","data":{…}}` are documented nested forms; inline JSON is passed directly as
   the `content` object (see §2.4(c)). Do not assume the `data.card` wrapper exists.
2. **`${var}` / `$var` placeholder syntax in card JSON source is unverified.** No fetched official
   page shows that syntax. The documented flow is builder variables + `template_variable`.
3. **App-feed-card "create" upsert semantics by `biz_id` are unverified.** The create response's
   `reason` enum includes `4 重复` (duplicate), which suggests re-creating the same `biz_id` may be
   rejected rather than upserted. The documented way to change an existing card is
   `PUT /open-apis/im/v2/app_feed_card/batch` with `update_fields`. Whether `biz_id` must be unique
   per user, and its retention period, are not stated.
4. **No exact "resolve chat by name" API.** Only keyword search (`GET /open-apis/im/v1/chats/search`
   and `POST /open-apis/im/v2/chats/search`) is documented; exact-name matching, result ranking and
   whether a non-joined but visible group is returned are not guaranteed/described.
5. **No documented API to list "users the bot can DM".** `GET /open-apis/contact/v3/users` is a
   directory listing (and marked 历史版本/不推荐使用), not the bot availability list. Determining the
   app's 可用范围 programmatically is unverified.
6. **Recall / edit time limit is not a fixed number in the API docs.** `230009` says the message is
   too old, and the doc defers to a **tenant-admin-configured** recall time limit; only the
   cross-user group recall has the explicit "不超过 1 年" rule. Do not hard-code a recall window.
7. **App feed card `reason` value type is inconsistent in the docs**: the enum table lists numeric
   strings (`0`–`4`) while the examples show symbolic names (`NOT_CREATED`, `NO_PERMISSION`).
   Handle both.
8. **App feed card `link` requiredness is contradictory** in the doc: the 必填 column says 否 but the
   description says 「创建时该参数为必填参数」. Treat it as required at create time.
9. **`contact/v3/users` is deprecated** in favour of `contact-v3/user/find_by_department`; the full
   request/response of the recommended replacement was not fetched.
10. **`width_mode` default is inconsistent in the doc**: the structure comment says the default is
    600px / "default not filled", while the field table lists default value `default`; all three
    values are confirmed.
11. **International Lark host** (`open.larksuite.com`) paths were assumed identical; only the
    `open.feishu.cn` host was actually fetched.
12. **Card entity + already-sent-card interaction**: docs say card-entity update APIs apply to a sent
    card (the update-card guide lists 全量更新卡片实体 / 批量更新卡片实体 as post-send update
    methods), but I found no explicit statement about what happens if the message is recalled or if
    you update an entity after its 14-day window but before the message's 14-day window. Both windows
    are separately documented as 14 days.

---

## Recommended sequence for this project

A minimal, well-supported AlertManager → Feishu flow:

1. **Auth**: `POST /open-apis/auth/v3/tenant_access_token/internal` with `app_id`/`app_secret`; cache
   `tenant_access_token` until `expire - 300s`; send as `Authorization: Bearer …`. No scope needed.
2. **Discover targets once at startup**: `GET /open-apis/im/v1/chats?page_size=100` (paginate with
   `page_token`/`has_more`) → build a `name → chat_id` map; optionally verify each with
   `GET /open-apis/im/v1/chats/:chat_id`. For names the bot has not joined, fall back to
   `GET /open-apis/im/v1/chats/search?query=…` (or `POST /open-apis/im/v2/chats/search`). Remember:
   the bot must be a member of any group it posts to, and DM targets must be in the app's 可用范围.
3. **Send an alert card**: the simplest robust path is
   `POST /open-apis/im/v1/messages?receive_id_type=chat_id` with `msg_type=interactive` and inline
   card JSON (**schema 2.0**) in `content`; set `config.update_multi=true` and a useful
   `config.summary.content`; use a freshly generated `uuid` per distinct alert to dedupe retries
   (same `uuid` ⇒ at most one send within 1 h). Keep the card ≤30 KB / ≤200 elements.
   - If you need template variable substitution, use the `{"type":"template","data":{…}}` form with
     `template_id` + `template_version_name` + `template_variable`.
4. **Update an alert card in place** (e.g. firing → resolved):
   - If sent as inline JSON: `PATCH /open-apis/im/v1/messages/:message_id` with the new card
     `content` (`update_multi: true` before and after, within 14 days).
   - If you need partial/streaming updates: create a card entity
     (`POST /open-apis/cardkit/v1/cards`, `type=card_json`, schema 2.0, `update_multi=true`), send
     `{"type":"card","data":{"card_id":"…"}}`, then drive
     `PUT /cards/:card_id` (full), `POST /cards/:card_id/batch_update` (batch),
     `PUT /cards/:card_id/elements/:element_id/content` (streaming text) and
     `PATCH /cards/:card_id/settings` (e.g. turn `streaming_mode` off). **The entity can be sent only
     once and lives 14 days** — create a fresh entity per alert. Keep a strictly increasing
     `sequence` per card and reuse `uuid` only for idempotent retries.
5. **Important / high-signal alerts as app feed cards**: `POST /open-apis/im/v2/app_feed_card` with
   your own `biz_id`, `user_ids` (≤20 per call, `open_id`), `status_label`, `link`,
   `time_sensitive=true` for temporary pinning, and `notify`. Track `(biz_id, user_id)`; to update or
   replace use `PUT /open-apis/im/v2/app_feed_card/batch` with `update_fields`
   (`1,2,3,10,11,12,13,101,102,103`); to remove use `DELETE /open-apis/im/v2/app_feed_card/batch`.
6. **Retire old alerts** when resolved: either PATCH the card to a resolved state, or
   `DELETE /open-apis/im/v1/messages/:message_id` (bot's own messages; subject to the tenant recall
   time limit), or delete the app feed card by `(biz_id, user_id)`.
7. **Scopes to request on the app**: `im:message`, `im:message:send_as_bot`, `im:chat:readonly`,
   `im:app_feed_card:write`, `cardkit:card:write` (plus `contact:user.employee_id:readonly` only if
   you use `user_id`; plus a `contact:*` scope only if you call the contact API). Enable 机器人能力
   and publish the app; add the bot to each target group; put DM recipients / alert channels in the
   app's 可用范围.
