<p align="center">
  <img src="icon.png" alt="alertmanager-lark-hook：AlertManager 告警 → 飞书卡片" width="220" />
</p>

# alertmanager-lark-hook

把 **Prometheus AlertManager** 的告警，通过**飞书机器人**投递到飞书「消息渠道」（群聊 / 单聊）的自建应用。

- **Rust** 后端（Axum + SQLite）· **TypeScript** 管理后台（Vite + React）· **单体容器**部署
- 支持**卡片搭建工具**产出的卡片（JSON 2.0 与模板 ID 两种形态）
- 每个渠道一个**独立的 AlertManager 接收地址**
- 完整的告警生命周期策略：重复告警 / 解决告警 / 分组更新 / 原地更新 / 撤回旧消息 / 即时提醒

---

## 目录

| 文档 | 内容 |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | 分层架构、限界上下文、关键设计决策、部署拓扑 |
| [`docs/domain-model.md`](docs/domain-model.md) | 聚合 / 实体 / 值对象、状态机、数据库 schema |
| [`docs/business-rules.md`](docs/business-rules.md) | 接收流水线、事件分类、**投递决策表**、R1–R18 补充规则 |
| [`docs/api.md`](docs/api.md) | 全部 REST 接口契约（请求/响应/枚举/错误码） |
| [`docs/research/feishu-api.md`](docs/research/feishu-api.md) | 逐条核对的飞书开放平台接口（URL、字段、权限、示例） |

---

## 1. 功能

| # | 能力 | 实现 |
|---|---|---|
| 1 | 导入卡片搭建工具创建的卡片 | 管理后台粘贴卡片 JSON 2.0，或填写 `template_id` + 版本；自动识别 `${var}` 变量、校验、预览、一键发送测试 |
| 2 | 通过飞书机器人发送消息 | `im/v1/messages`（交互式卡片）与 `im/v2/app_feed_card`（应用消息流卡片） |
| 3 | API 读取可接收消息的渠道 + 每渠道独立 Hook | `GET /open-apis/im/v1/chats` 一键同步；每个渠道签发独立 token 的接收地址，可多 Hook、可轮换 |
| 4 | 配置渠道的发送规则 | 消息类型 / 即时提醒 / 重复告警 / 解决告警 / 分组更新 / 标签过滤 / 聚合窗口 / 限流 / 静默期 |
| 5 | Rust + TS 管理后台 + 容器化 | 前端产物内嵌进二进制，单容器交付；见 [`deploy/Dockerfile`](deploy/Dockerfile) 与 [`docker-compose.yml`](docker-compose.yml) |
| 6 | 嵌入式 SQLite | `sqlx` + bundled libsqlite3，WAL 模式，迁移编译期打包 |
| 7 | 仅使用开放平台文档中的 API | 每个端点都在 [`docs/research/feishu-api.md`](docs/research/feishu-api.md) 中记录出处与原文校验结论 |
| 8 | 清晰的架构与完备的业务规则 | DDD 分层（domain / app / infra / server），决策表可测试，补充了幂等、审计、指标、保留策略、重试、降级等规则 |

---

## 2. 架构一览

```
AlertManager ──webhook──▶ /api/v1/hooks/alertmanager/{token}
                                 │
                     ┌───────────▼────────────┐
                     │  alkh-server (Axum)    │  ← 管理后台 SPA（内嵌）
                     ├────────────────────────┤
                     │  alkh-app  用例 / 决策表 │  ← 聚合窗口、限流、重试
                     ├────────────────────────┤
                     │  alkh-domain 聚合/端口   │  ← 零 I/O
                     ├────────────────────────┤
                     │  alkh-infra SQLite/飞书  │
                     └───────────┬────────────┘
                                 ▼
              cardkit/v1/cards · im/v1/messages · im/v2/app_feed_card
```

```
crates/
  alkh-domain/   聚合、值对象、领域服务、端口 trait
  alkh-app/      用例：接收流水线、投递决策、卡片渲染、后台任务
  alkh-infra/    SQLite 仓储、飞书网关、AES-GCM / Argon2
  alkh-server/   Axum 路由、内嵌前端、指标、定时任务、组合根
web/             Vite + React + TS 管理后台
migrations/      嵌入式 SQLite 迁移
deploy/          Dockerfile、AlertManager 示例配置
```

---

## 3. 快速开始（容器）

```bash
cp .env.example .env
# 至少设置 ALKH_MASTER_KEY 与 ALKH_ADMIN_PASSWORD
openssl rand -hex 32        # 填入 ALKH_MASTER_KEY

docker compose up -d --build
# 打开 http://localhost:8080 ，用 ALKH_ADMIN_USERNAME / ALKH_ADMIN_PASSWORD 登录
```

带上演示用 AlertManager：

```bash
docker compose --profile demo up -d --build
# AlertManager: http://localhost:9093
```

## 4. 本地开发

```bash
# 前端依赖
cd web && pnpm install

# 终端 A：后端（使用 web/dist 作为静态目录，改前端无需重编译）
make run

# 终端 B：前端热更新（Vite 代理 /api → localhost:8080）
cd web && pnpm dev
```

常用命令见 `make help`。

---

## 5. 飞书应用准备

1. 在[飞书开放平台](https://open.feishu.cn/app)创建**企业自建应用**，添加**机器人**能力并发布。
2. 申请以下权限（任一满足即可，括号内为备选）：

   | 用途 | 权限 |
   |---|---|
   | 发送 / 更新消息 | `im:message` 或 `im:message:send_as_bot` |
   | 撤回消息 | `im:message:recall`（或 `im:message`） |
   | 读取机器人所在群 | `im:chat:readonly`（或 `im:chat`） |
   | 应用消息流卡片 | `im:app_feed_card:write` |
   | 卡片实体创建 / 更新 | `cardkit:card:write` |
   | 可选：返回 user_id 字段 | `contact:user.employee_id:readonly` |

3. 把机器人**拉进目标群**；给用户发消息时，该用户需在应用的**可用范围**内。
4. 在管理后台「设置」中填写 `app_id` / `app_secret`，点「测试飞书凭据」确认连通。
   `app_secret` 使用 AES-256-GCM 加密后落库，接口永不回显明文。

> 使用卡片搭建工具的卡片时，若用 `template_id` 方式，需要在卡片编辑页把本应用加为可用应用，
> 并**保存 + 发布**卡片；建议填写固定 `template_version_name` 避免线上被改版影响。

---

## 6. 使用流程

1. **配置凭据**：设置页填写飞书应用凭据并测试。
2. **同步渠道**：渠道页点「从飞书同步」，或手工创建（支持 `chat_id` / `open_id` / `user_id` / `email`）。
3. **导入卡片**：卡片模板页粘贴卡片搭建工具导出的 JSON，或填写模板 ID；校验 + 预览确认变量渲染正确。
4. **配置发送规则**：渠道详情 → 发送规则，选择消息类型、模板、重复/解决/分组更新策略等。
5. **创建 Hook**：渠道详情 → 接收 Hook → 新建；**复制一次性显示的完整 URL**。
6. **配置 AlertManager**：把 URL 写入 `webhook_configs`（见 [`deploy/alertmanager.example.yml`](deploy/alertmanager.example.yml)），
   `send_resolved: true` 必须开启。
7. **验证**：渠道详情 → 发送测试，或 `amtool alert add ...` 触发真实告警。

---

## 7. 核心业务规则（摘要）

完整规则见 [`docs/business-rules.md`](docs/business-rules.md)。

| 事件 | 策略字段 | 可选值 → 动作 |
|---|---|---|
| 新告警 | — | 始终发送新消息 |
| **重复告警** | `repeat_policy` | `suppress`（默认，只更新状态）· `send` · `send_and_revoke_previous` |
| 分组内容变化 | `group_update_policy` | `update_previous`（默认，原地更新卡片）· `send` · `suppress` |
| **解决告警** | `resolved_policy` | `send`（默认）· `send_and_delete_previous` · `update_previous` |

补充规则（节选）：

- **R1** 卡片实体过期 / 消息已删除时，「原地更新」自动降级为发送新消息，不报错。
- **R3** 渲染内容未变化则跳过更新，避免无意义调用与限频。
- **R4** 「即时提醒」仅对**应用消息流卡片**生效（飞书 `time_sensitive`）；卡片消息无对应能力，界面会明确提示。
- **R9** 幂等：本地 `event_hash` 去重窗口 + 飞书 `uuid` 双重保障。
- **R11** 标签过滤采用 Prometheus matcher 语义（`=` `!=` `=~` `!~`，`!=`/`!~` 对不存在的标签也匹配）。
- **R14** 失败投递按 1s / 5s / 30s / 2m / 10m 退避重试 5 次，之后可手动重试。
- **R15** 应用消息流卡片**只能发给用户**（飞书该接口没有群维度），渠道策略会做交叉校验。
- **R17** 卡片 JSON 必须 `schema = "2.0"` 且 `config.update_multi` 不为 `false`，否则拒绝保存。

---

## 8. 配置项

| 变量 | 默认 | 说明 |
|---|---|---|
| `ALKH_HTTP_ADDR` | `0.0.0.0:8080` | 监听地址 |
| `ALKH_DATA_DIR` | `/data` | 数据目录（SQLite + 主密钥） |
| `ALKH_DATABASE_URL` | `sqlite://{data_dir}/alkh.db` | 覆盖数据库 URL |
| `ALKH_MASTER_KEY` | 自动生成到 `{data_dir}/master.key` | 32 字节 hex / base64，加密飞书密钥 |
| `ALKH_ADMIN_USERNAME` / `ALKH_ADMIN_PASSWORD` | `admin` / 随机 | 仅首次启动创建 |
| `ALKH_PUBLIC_BASE_URL` | 空 | 生成 Hook 绝对地址 |
| `ALKH_LARK_BASE_URL` | `https://open.feishu.cn` | Lark 国际版为 `https://open.larksuite.com` |
| `ALKH_COOKIE_SECURE` | `false` | HTTPS 部署时置 `true` |
| `ALKH_LOG_LEVEL` / `ALKH_LOG_JSON` | `info` / `false` | 日志 |
| `ALKH_STATIC_DIR` | 空 | 覆盖内嵌前端目录（前端开发用） |
| `ALKH_WORKER_TICK_MS` | `200` | 聚合冲刷 tick |
| `ALKH_RETRY_INTERVAL_SECS` | `30` | 失败重试扫描间隔 |
| `ALKH_RETENTION_INTERVAL_SECS` | `3600` | 保留清理间隔 |

保留天数在管理后台「设置」中调整（默认告警分组 30 天 / 投递 30 天 / 审计 14 天）。

---

## 9. 接口与可观测性

- `POST /api/v1/hooks/alertmanager/{token}` — AlertManager 接收端点（token 即凭证）
- 管理 REST 全部接口见 [`docs/api.md`](docs/api.md)
- `GET /healthz` 存活 · `GET /readyz` 就绪 · `GET /metrics` Prometheus 指标
- 审计日志记录每一次接收、发送、更新、撤回、失败与配置变更

---

## 10. 开发与测试

```bash
make test     # 全部 Rust 测试
make lint     # Clippy（-D warnings）
make web      # 构建前端并同步到内嵌目录
make release  # 产出单文件二进制 target/release/alkh
```

## 11. 已知限制与待确认项

飞书文档中若干细节无法从官方文档 100% 确认，已在
[`docs/research/feishu-api.md`](docs/research/feishu-api.md) 的 *Gaps / uncertainties* 中逐条列出，
包括：

- 卡片搭建工具产出的 JSON 使用 `${var}` 占位符属于**本项目的模板约定**（官方文档只描述 `template_variable` 键值映射）；
- 应用消息流卡片创建接口**没有文档化的 upsert 语义**，重复 `biz_id` 可能被拒绝；
- 消息撤回时限由**企业管理员策略**决定，并非固定值；
- 没有按名称精确查找群的官方接口（仅有搜索接口）。

生产使用前，建议在测试租户用「发送测试」与真实告警各跑通一遍。
