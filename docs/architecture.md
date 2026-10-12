# 架构设计 — alertmanager-lark-hook

> 将 Prometheus AlertManager 的告警，通过飞书机器人投递到飞书「消息渠道」（群聊 / 单聊）的
> 自建应用。Rust 后端 + TypeScript 管理后台 + 嵌入式 SQLite + 容器化部署。

## 1. 系统上下文

```
                ┌────────────────────┐
  Prometheus ──▶│    AlertManager    │
                └─────────┬──────────┘
                          │ webhook_configs (POST JSON, AlertManager v4 payload)
                          │ http://alkh:8080/api/v1/hooks/alertmanager/<hook_token>
                          ▼
        ┌────────────────────────────────────────────────────────┐
        │                 alertmanager-lark-hook                 │
        │  ┌────────────────┐  ┌──────────────────────────┐      │
        │  │ Hook 接收侧    │  │ 管理后台 (REST API)      │      │
        │  │ 去重/聚合/路由 │  │ 内嵌 TS 前端 SPA         │      │
        │  └───────┬────────┘  └──────────────────────────┘      │
        │          │ 告警投递流水线                              │
        │          │                                             │
        │          │ 通知策略引擎 (NotificationPolicy)           │
        │          │ 卡片渲染器 (CardKit JSON 2.0 / 应用消息流)  │
        └──────────┼─────────────────────────────────────────────┘
                   │ 飞书开放平台 OpenAPI (tenant_access_token)
                   ▼
     ┌──────────────────────────────────────────────────────┐
     │ cardkit/v1/cards    创建卡片实体，用于「更新旧告警」 │
     │ im/v1/messages      发送卡片消息 / 撤回消息          │
     │ im/v2/app_feed_card 发送·更新·删除 应用消息流卡片    │
     │ im/v1/chats         读取机器人可触达渠道             │
     └──────────────────────────────────────────────────────┘
```

## 2. 分层与依赖（六边形 / DDD）

模块化单体（modular monolith），一个可执行文件、一个容器，四个 crate：

| crate | 职责 | 依赖方向 |
|---|---|---|
| `alkh-domain` | 纯领域层：聚合、实体、值对象、领域服务、领域事件、**端口 trait（ports）**。零 I/O、零框架 | 无 |
| `alkh-app` | 应用层：用例编排、事务边界、策略执行器、DTO 转换 | → domain |
| `alkh-infra` | 适配器：SQLite 仓储（sqlx）、飞书 OpenAPI 网关（reqwest）、AES-GCM 加密、系统时钟 | → domain（实现 ports） |
| `alkh-server` | 交付层（composition root）：Axum 路由、AlertManager Hook、管理 REST、内嵌前端、后台任务、配置、可观测性 | → app, infra, domain |

依赖倒置：`alkh-app` 只依赖 `alkh-domain` 中定义的端口 trait，`alkh-server` 在启动时把
`alkh-infra` 的实现注入进去。领域层不出现 `sqlx` / `reqwest` / `axum` 任何符号。

### 端口（`alkh-domain::ports`）

- `ChannelRepository`、`HookRepository`、`CardTemplateRepository`、`NotificationPolicyRepository`
- `AlertGroupRepository`、`DeliveryRepository`、`AuditLogRepository`
- `SettingsRepository`、`AdminUserRepository`、`SessionRepository`
- `LarkGateway`（发送/更新/撤回消息、创建卡片实体、同步渠道）
- `SecretCipher`（应用密钥加解密）、`Clock`、`EventDedup`（投递幂等窗口）、`LarkCredentials`（运行时凭据提供）

## 3. 限界上下文

1. **渠道管理 (Channel Management)** — 从飞书同步可触达渠道，维护启停与元数据。
2. **接入端管理 (Hook Management)** — 每个渠道签发一个独立 hook token / URL。
3. **卡片模板 (Card Template)** — 导入卡片搭建工具产物；卡片消息（JSON 2.0 / template_id）与
   应用消息流模板两种形态；变量校验与预览。
4. **通知策略 (Notification Policy)** — 消息类型、置顶、重复告警、恢复告警、分组更新、
   标签过滤、聚合窗口、限流、静默期。
5. **告警接入与投递 (Alert Intake & Delivery)** — AlertGroup 聚合根 + Delivery 子实体，
   承载事件分类、投递决策、消息引用与生命周期。
6. **应用设置与鉴权 (Settings & Auth)** — 飞书应用凭据（加密存储）、管理员账号、会话。
7. **运维 (Operations)** — 指标、审计日志、数据保留与清理、投递重试。

## 4. 关键设计决策

| # | 决策 | 理由 |
|---|---|---|
| D1 | 模块化单体而非微服务 | 单实例小工具，SQLite 单写者模型，拆分只会带来分布式事务成本 |
| D2 | 嵌入式 SQLite（`sqlx` + bundled libsqlite3）+ WAL | 零外部依赖、容器内单文件、`journal_mode=WAL` + `busy_timeout` 支撑并发读 |
| D3 | 卡片实体（CardKit `card_id`）作为「更新旧告警」的载体 | CardKit 支持在同一 `card_id` 上全量/局部更新，是实现 in-place 更新的唯一官方路径 |
| D4 | 领域层定义端口，infra 实现 | 可测试：用例层用内存假实现即可跑单测 |
| D5 | hook 采用不可猜测 token 路径 `/{token}` | AlertManager 的 webhook receiver 无内置鉴权，token 即凭证；支持轮换 |
| D6 | 前端构建产物 `rust-embed` 进二进制 | 单文件交付，容器无需 nginx；同时支持 `ALKH_STATIC_DIR` 覆盖以便前端开发 |
| D7 | 事件分类 + 决策表驱动投递 | 把「重复/恢复/分组更新」收敛到一张显式决策表，可测试、可解释 |
| D8 | 幂等：Feishu `uuid` 去重 + 本地 `event_hash` 去重 | AlertManager 会重投；`uuid` 在飞书侧 1 小时内至多成功一条 |
| D9 | 飞书凭据 AES-256-GCM 加密落库 | 数据库文件可能被备份/挂载，明文 app_secret 不可接受 |
| D10 | 后台任务与请求处理同进程（tokio task） | 单实例部署，无需外部队列；投递重试/聚合冲刷/保留清理 |
| D11 | 不做跨聚合事务，以"幂等 + 重试"保证一致 | SQLite 单文件下引入 UnitOfWork 收益低；接收流水线对同一事件可安全重放（R9），失败返回 500 交给 AlertManager 重试 |
| D12 | 卡片原地更新按模板来源分流 | `card_json` 走 CardKit 卡片实体（`PUT /cardkit/v1/cards/:card_id`，需严格递增 `sequence`）；`template` 走 `PATCH /im/v1/messages/:message_id`（支持 `{"type":"template",...}`）。两条路径都是各自被官方文档覆盖的唯一方式 |

## 5. 运行时拓扑与部署

```
deploy/
  Dockerfile            # 多阶段：node 构建前端 → rust 构建后端 → distroless/debian-slim 运行
  docker-compose.yml    # alkh + 可选 alertmanager 演示 + 数据卷
```

- 单容器暴露 `8080`；`/` 为管理后台，`/api/v1/**` 为 REST，`/api/v1/hooks/alertmanager/**` 为接收端点。
- 持久化：`/data/alkh.db`（SQLite + WAL），通过 volume 挂载。
- 配置全部来自环境变量（12-factor），前缀 `ALKH_`：

| 变量 | 说明 | 默认 |
|---|---|---|
| `ALKH_HTTP_ADDR` | 监听地址 | `0.0.0.0:8080` |
| `ALKH_DATA_DIR` | 数据目录 | `/data` |
| `ALKH_DATABASE_URL` | 覆盖 SQLite URL | `sqlite://{data_dir}/alkh.db` |
| `ALKH_MASTER_KEY` | 32 字节密钥（hex/base64），用于加密飞书密钥 | 自动生成到 `{data_dir}/master.key` |
| `ALKH_ADMIN_USERNAME` | 初始管理员用户名 | `admin` |
| `ALKH_ADMIN_PASSWORD` | 初始管理员密码（仅首次启动生效） | 随机生成并打印到日志 |
| `ALKH_PUBLIC_BASE_URL` | 生成 hook 绝对地址用 | 空 |
| `ALKH_LARK_BASE_URL` | 飞书 OpenAPI 基址 | `https://open.feishu.cn` |
| `ALKH_LOG_LEVEL` | 日志级别 | `info` |
| `ALKH_STATIC_DIR` | 覆盖内嵌前端目录（开发用） | 空（用内嵌） |

## 6. 横切关注点

- **可观测性**：`/healthz`（存活）、`/readyz`（DB 可写 + 迁移完成）、`/metrics`（Prometheus 文本，
  手写轻量注册表，避免重依赖）。结构化日志 `tracing` + JSON 输出，含 `log_id`（飞书响应头
  `X-Tt-Logid`）便于排查。
- **错误处理**：领域错误 `DomainError` → 应用错误 `AppError` → HTTP 统一错误信封
  `{"error":{"code","message"}}`。
- **并发**：同一 `(hook_id, group_key)` 的事件通过 keyed mutex 串行化，避免重复投递竞态。
- **保留策略**：后台任务按天清理超期 `alert_group` / `delivery` / `audit_log`。
- **安全**：管理 API 走会话 Cookie（HttpOnly, SameSite=Lax）或 `Authorization: Bearer`；
  密码 Argon2id 哈希；hook token 仅比较哈希常量时间；所有 SQL 参数化。
