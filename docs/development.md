# 开发与测试

> 面向本仓库的开发者。使用 / 部署说明见 [README](../README.md)。

## 目录

- [环境要求](#环境要求)
- [目录结构](#目录结构)
- [本地开发](#本地开发)
- [构建](#构建)
- [测试与代码规范](#测试与代码规范)
- [前端](#前端)
- [图标生成](#图标生成)
- [容器构建](#容器构建)
- [契约维护（重要）](#契约维护重要)
- [架构约束](#架构约束)

---

## 环境要求

| 工具 | 版本 | 用途 |
|---|---|---|
| Rust | **1.89+**（`Cargo.toml` 声明的下限；本仓库在 1.96 上开发） | 后端 |
| Node.js | 22+ | 前端构建 |
| pnpm | 11+ | 前端包管理（`npm` 也可用） |
| Python + Pillow | 3.9+ | 仅在重新生成图标时需要 |

SQLite 会被编译进二进制（`sqlx` 的 bundled libsqlite3），**无需**单独安装；构建需要 C 编译器与 `pkg-config`。

## 目录结构

```
crates/
  alkh-domain/    纯领域层：聚合、值对象、领域服务、端口 trait。零 I/O、零框架依赖
  alkh-app/       应用层：接收流水线、投递决策表、卡片渲染、后台任务
  alkh-infra/     适配器：SQLite 仓储、飞书网关、AES-GCM / Argon2、系统时钟
  alkh-server/    交付层（组合根）：Axum 路由、内嵌前端、指标、定时任务
web/              Vite + React + TS 管理后台
docs/images/      文档用图片（README 顶部品牌图）
migrations/       嵌入式 SQLite 迁移（编译期打包进二进制）
deploy/           Dockerfile、AlertManager / Prometheus 示例配置
docs/             架构与设计文档（见 README 索引）
scripts/          构建辅助脚本（图标生成等）
```

依赖方向：`alkh-server → alkh-app → alkh-domain`；`alkh-infra` 实现 `alkh-domain` 中定义的端口，由 `alkh-server` 在组合根注入。

## 本地开发

```bash
# 前端依赖
cd web && pnpm install && cd ..

# 终端 A：后端（用 web/dist 作为静态目录，改前端无需重编译）
make run

# 终端 B：前端热更新（Vite 把 /api 代理到 localhost:8080）
cd web && pnpm dev
```

`make run` 使用 `ALKH_DATA_DIR=./data`，首次启动会在终端打印随机管理员密码（或用 `ALKH_ADMIN_PASSWORD` 指定）。

常用目标见 `make help`：

| 目标 | 作用 |
|---|---|
| `make run` | 本地起后端 + 磁盘上的前端产物 |
| `make dev` | 只起后端（前端交给 `pnpm dev`） |
| `make icons` | 从品牌资产派生 favicon PNG 尺寸与 apple-touch-icon |
| `make web` | 构建前端并同步到 `crates/alkh-server/web-dist` |
| `make build` | debug 构建（含前端） |
| `make release` | release 构建，产出 `target/release/alkh` |
| `make test` | 全部 Rust 测试 |
| `make lint` | Clippy（`-D warnings`） |
| `make fmt` | rustfmt |
| `make docker` | 构建容器镜像 |
| `make up` / `make down` | docker compose 启停 |

## 构建

前端产物由 `rust-embed` 打进二进制，`#[folder = "web-dist"]` 指向 `crates/alkh-server/web-dist`。因此：

- **改前端必须重新跑 `make web` 再 `cargo build`**，否则二进制里还是旧页面；
- 只调后端时可以用 `ALKH_STATIC_DIR=/绝对路径/web/dist` 直接读磁盘，免去重编译；
- 仓库里 `crates/alkh-server/web-dist/index.html` 是一个**占位页**（提交进版本库，保证目录存在），真实产物由 `make web` 生成且不入库。

```bash
make release          # = make web + cargo build --release --bin alkh
./target/release/alkh --healthcheck   # 容器 HEALTHCHECK 用的自检
```

## 测试与代码规范

```bash
cargo test --workspace                      # 全部测试
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cd web && pnpm exec tsc --noEmit && pnpm build
```

测试分布（当前 149 个）：

| 位置 | 数量 | 覆盖内容 |
|---|---|---|
| `alkh-domain` | 40 | 领域不变量、决策表、matcher 语义、事件哈希、模板校验与渲染 |
| `alkh-app` | 35 | 聚合窗口、限流、渲染上下文、元数据线格式 |
| `alkh-infra` | 50 | 各仓储的 CRUD / 分页 / 过滤器、加密与口令哈希、飞书请求体构造 |
| `alkh-server` | 9 | 配置、Cookie / Bearer 解析、指标渲染 |
| `alkh-server/tests/e2e.rs` | 15 | **真实 HTTP 栈 + 内存 SQLite + Fake 飞书网关**的端到端流程 |

端到端测试直接 `axum::serve` 一个真实监听端口，用 `reqwest` 走 HTTP，并用 `wiring::build_state(..., Some(fake_gateway))` 注入假的飞书网关 —— 因此能覆盖「创建渠道 → 签发 Hook → AlertManager 投递 → 去重 → 原地更新 → 恢复 → 查询」的完整链路，而不需要真实飞书凭据。

## 前端

- `web/src/api.ts` 是**唯一**的 fetch 客户端：统一 `/api/v1` 前缀、`credentials: 'same-origin'`、错误信封解析、401 跳登录。
- `web/src/types.ts` 里的 DTO 字段名必须与 [`docs/api.md`](api.md) 逐字一致。
- `web/src/meta.tsx` 从 `GET /api/v1/meta` 加载枚举与变量清单，远程数据优先，本地只有兜底。**枚举一律以 `/meta` 为准，不要在 `labels.ts` 里复制一份**（历史上这两份副本漂移过，导致下拉框渲染出重复的 `[object Object]` key）。
- 新增后端字段时的顺序：改 `docs/api.md` → 改后端 → 改 `types.ts` → 改页面。

## 图标生成

品牌资产（均入库，**不放在仓库根目录**）：

| 路径 | 说明 |
|---|---|
| `web/public/favicon.ico` | 品牌主图标，浏览器直接取用，**不经过生成** |
| `docs/images/banner.png` | 高清品牌图（1536×1024，深色底）：README 顶部展示 + apple-touch-icon 的来源 |

派生小尺寸图标：

```bash
make icons      # = python3 scripts/make-icons.py
```

产物：`web/public/favicon-16x16.png`、`favicon-32x32.png`（与 `favicon.ico` 同源，保证各尺寸视觉一致）、
`apple-touch-icon.png`（180×180，来自高清图）。脚本头部记录了处理约定
（为何 180px 不用 `favicon.ico` 放大、方形化为何用边缘延展而不是填纯色），改动前请先读。

## 容器构建

```bash
make docker                                   # 等价于 docker build -f deploy/Dockerfile -t alertmanager-lark-hook .
docker compose up -d --build
```

`deploy/Dockerfile` 是三阶段构建：

1. `node:22-alpine` 构建前端；
2. `rust:1.85-bookworm` 用桩代码预热依赖缓存，再复制真实源码构建（依赖层可命中缓存）；
3. `debian:bookworm-slim` 运行，非 root 用户、`/data` 卷、内置 `--healthcheck`。

## 发布与 CI

### 持续集成

`.github/workflows/ci.yml` 在所有 push 到 main 与 PR 上运行两个 job：

| job | 内容 |
|---|---|
| `rust` | `cargo fmt --check` → `cargo clippy -D warnings` → `cargo test --workspace` |
| `web` | `pnpm install --frozen-lockfile` → `tsc --noEmit` → `pnpm build` |

端到端测试自带 Fake 飞书网关，**不需要任何真实凭据或外部服务**，因此在 CI 里可以直接跑通。

### 发布镜像

`.github/workflows/publish.yml` 把镜像推到 GitHub Container Registry：

```bash
git tag v0.1.0
git push origin v0.1.0
```

产出 `ghcr.io/fengyifan/alertmanager-lark-hook` 的以下标签：

| 标签 | 触发 |
|---|---|
| `0.1.0` / `0.1` / `0` | 推送 `v0.1.0` tag |
| `main` / `latest` | 推送 main 分支 |
| `sha-xxxxxxx` | 每次构建 |

用内置的 `GITHUB_TOKEN` 推送（`permissions: packages: write`），不需要额外配置 secret。

默认只构建 `linux/amd64`。需要 `linux/arm64`（树莓派 / Apple Silicon 节点）时，
在 workflow 里加上 `docker/setup-qemu-action` 并把 `platforms` 改成 `linux/amd64,linux/arm64`
—— 注意 Dockerfile 内的 Rust 编译在 QEMU 模拟下会慢约一个数量级。

### 本地构建镜像

```bash
make docker        # 等价于 docker build -f deploy/Dockerfile -t ghcr.io/fengyifan/alertmanager-lark-hook:latest .
```

`.dockerignore` 排除了 `target/`、`web/node_modules`、`web/dist`、`docs/` 等与构建无关的内容
（否则 `target/` 可能有好几个 GB 会被一起发给 daemon）。**改动它时请确认
Dockerfile 里所有 `COPY` 的源路径仍然存在于构建上下文中。**

## 契约维护（重要）

手写的 API 契约与实现很容易悄悄脱节（本项目就踩过一次：`/meta` 的枚举列表后端返回对象数组、文档写成字符串数组，前端照着文档写，结果下拉框渲染出重复的 `[object Object]` key）。目前有三道防线：

1. **后端单测锁定线格式** —— `alkh_app::meta::tests::meta_wire_format_is_enum_option_objects`
   断言 `/meta` 每个枚举字段都是非空的 `{value,label}` 对象数组、变量序列化为 `type`（不是 Rust 字段名 `ty`）、`severity_colors` 是对象映射。
2. **端到端测试锁定 HTTP 响应** —— `e2e::meta_endpoint_pins_the_wire_format`
   对真实响应做同样的断言，确保序列化层没有额外包装或改名。
3. **README 变量表与代码同步** —— `alkh_app::meta::tests::readme_documents_every_variable`
   读取 README 中 `<!-- variable-table:start -->` / `<!-- variable-table:end -->` 之间的内容，
   校验：代码里声明的每个内置变量都在 README 里出现，且 README 没有写代码里不存在的变量。

改动变量或枚举时，请同步更新：

- [ ] `crates/alkh-app/src/meta.rs`（代码，唯一事实来源）
- [ ] `README.md` §3.3 的变量表（有测试把关）
- [ ] `docs/api.md` 的 `/meta` 示例
- [ ] `web/src/types.ts` 的 `Meta` 类型
- [ ] `web/src/meta.tsx` 的兜底字面量（同样对齐真实形状）

## 架构约束

改代码时请守住这几条：

- **`alkh-domain` 不允许出现 I/O 或框架符号**：没有 `sqlx` / `reqwest` / `axum`，端口只以 trait 形式声明。领域逻辑必须能在不启动任何外部依赖的情况下跑单测。
- **`alkh-app` 只依赖 `alkh-domain`**，通过端口 trait 访问外部世界，不认识具体适配器。
- **`alkh-infra` 不依赖 `alkh-app`**，只实现 `alkh-domain::ports`。
- **组合根只有 `alkh-server`**：连接池、加密器、网关的装配都在 `wiring.rs`，业务代码不直接拿 `SqlitePool`（就绪探针与组合根除外）。
- **迁移只增不改**：`migrations/` 下的文件一旦发布就不要修改，新增用新的编号文件。
- 新增领域规则时，同步补上[业务规则文档](business-rules.md)里的编号条目（R1、R2…）与对应单测。
