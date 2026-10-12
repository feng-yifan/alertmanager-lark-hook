# web — alertmanager-lark-hook 管理后台

Rust 服务（`crates/alkh-server`）内嵌的 TypeScript 管理后台 SPA：把 Prometheus AlertManager
的告警投递到飞书 / Lark。实现严格对齐 `docs/api.md`（REST 契约）、`docs/business-rules.md`
（投递决策表 R1–R14）与 `docs/domain-model.md`（实体与枚举）。

## 技术栈

- Vite + React 18 + TypeScript（`strict`，并开启 `noUnusedLocals` / `noUnusedParameters`）
- `react-router-dom` v6 路由
- 无组件库、无状态管理库：`web/src/components/ui.tsx` 手写组件 + `web/src/styles.css`
  手写 CSS（CSS 变量、浅色专业后台风格、响应式、`prefers-color-scheme` 深色模式）
- 会话：`fetch` + `credentials: 'same-origin'`，仅有 `SessionProvider` / `ToastProvider` /
  `MetaProvider` 三个 React context

## 开发 / 构建命令

```bash
pnpm install          # npm install 亦可（仓库提交的是 pnpm-lock.yaml）
pnpm dev              # Vite 开发服务器（默认 http://localhost:5173）
pnpm exec tsc --noEmit   # 仅类型检查
pnpm build            # tsc --noEmit && vite build → web/dist
pnpm preview          # 预览构建产物
```

- Vite dev server 代理：`/api`、`/healthz`、`/readyz` → `http://localhost:8080`（见 `vite.config.ts`）。
- `build.outDir = 'dist'`、`base = '/'`；构建产物由 Rust 侧 `rust-embed` 打进二进制
  （`ALKH_STATIC_DIR` 可覆盖为 `web/dist` 以便前端联调，见 `docs/architecture.md` D6）。

## 假定的 API 基址

- 所有请求走同源相对路径 `/api/v1`（`web/src/api.ts` 的 `request()` 统一加前缀），
  生产环境由同一个 8080 端口的 Rust 服务提供，因此不需要 CORS、也不需要配置后备地址。
- 鉴权使用 HttpOnly Cookie `alkh_session`（`credentials: 'same-origin'`）。契约允许
  `Authorization: Bearer`，但浏览器端不注入 token。
- 任何业务接口返回 `401` 时，`api.ts` 会通知会话层清空登录态，路由守卫随即跳转 `/login`
  （`/auth/login`、`/auth/session` 例外，以便登录表单显示 `invalid_credentials`）。
- 错误信封 `{"error":{"code","message"}}`（含可选 `details`）统一解析为 `ApiError`；
  `ApiError.code` 用于 409（`channel_has_deliveries` / `template_in_use` / `duplicate_receive_id`）
  等分支处理。

## 页面清单

| 路由 | 文件 | 内容 |
|---|---|---|
| `/login` | `pages/LoginPage.tsx` | 登录表单、会话引导、重定向回原路径 |
| `/` | `pages/OverviewPage.tsx` | `GET /overview` 统计卡（渠道 / Hook / firing·resolved 分组 / 近 24h sent·failed·suppressed）+ 最近事件 |
| `/channels` | `pages/ChannelsPage.tsx` | 列表、启用开关、搜索、从飞书同步、手动创建（含一次性 `hook_token` 展示） |
| `/channels/:id` | `pages/ChannelDetailPage.tsx` + `pages/channels/*` | 三个 Tab：发送规则（完整 NotificationPolicy 表单 + 决策表 + R4/R5/R6/R7/R8/R11 说明）、接收 Hook（创建 / 轮换 / 重命名 / 启停 / 删除 + 一次性 token）、发送测试 |
| `/card-templates` | `pages/CardTemplatesPage.tsx` | 模板列表、按 kind 过滤、设为默认、删除（409 `template_in_use` 提示） |
| `/card-templates/new`、`/card-templates/:id` | `pages/CardTemplateEditorPage.tsx` | card_message：JSON 编辑器（失焦校验）+ 搭建工具导入 + source 切换 + 变量面板 + 校验 / 预览；app_feed_card：强类型表单 + 长度实时提示 + 变量插入按钮；已存在的模板可发送测试（`/card-templates/:id/send-test`） |
| `/alert-groups` | `pages/AlertGroupsPage.tsx` | status / channel / severity / 搜索过滤，分页，删除记录 |
| `/alert-groups/:id` | `pages/AlertGroupDetailPage.tsx` | 分组元数据、告警表格（labels / annotations / generator_url）、labels 面板、投递时间线（重试 / 撤回） |
| `/deliveries` | `pages/DeliveriesPage.tsx` | 状态徽标、过滤（alert_group_id / channel / state）、重试 / 撤回 |
| `/settings` | `pages/SettingsPage.tsx` | 飞书凭据（app_secret 只写）、base URL、保留天数、测试凭据、修改密码、审计日志查看器 |

所有数据视图都有加载中 / 空 / 错误（可重试）状态；所有操作都有 toast 反馈，按钮在请求期间禁用。
「明文 token / hook_url 只显示一次」在渠道创建、Hook 创建、Hook 轮换三处都用独立的一次性面板
（`⚠ 只显示一次` 警告 + 复制按钮 + 「我已保存」关闭）呈现，列表只显示 `token_prefix` 与占位 `hook_path`。

## 目录结构

```
web/
  index.html
  vite.config.ts
  tsconfig.json
  pnpm-workspace.yaml     # pnpm 11 的 allowBuilds（批准 esbuild 的 postinstall）
  src/
    main.tsx              # 挂载 + Provider 组合
    App.tsx               # 路由 + 侧边栏布局 + RequireAuth 守卫
    api.ts                # 唯一 fetch 客户端（ApiError / 错误信封 / 401 处理 / 全部端点）
    types.ts              # 契约 DTO（字段名与枚举值逐字对齐 docs/api.md）
    labels.ts             # 枚举中文标签、决策表 DECISION_TABLE、R4/R5/R11 文案
    meta.tsx              # GET /meta + 契约兜底枚举 + useMeta()
    session.tsx           # 会话引导 / 登录 / 登出 / 改密标记
    toast.tsx             # 轻量 toast
    hooks.ts              # useAsync / useDebounced
    utils.ts              # 时间、JSON、actor/outcome/validation 防御式渲染
    styles.css            # 全部样式（CSS 变量 + 深色模式）
    components/ui.tsx     # Button/Field/Input/Select/Textarea/Switch/RadioGroup/Badge/
                          # StatusBadge/Card/Modal/ConfirmModal/Tabs/Pagination/
                          # CopyButton/JsonBlock/DescriptionList/Banner/状态组件
    pages/                # 见上表；渠道详情的三个 Tab 在 pages/channels/
```

## 契约中未明确、由前端自行决定的部分

1. **`biz_id_source` 取值**：`docs/api.md` §5 只出现 `"alert_group"`，`docs/domain-model.md`
   写作 `AlertGroupId | HookId | Custom`。采用 `alert_group` / `hook` / `custom` 三个短名；
   若接口返回其它值，`enumOptions()` 会把它作为「未知选项」保留在 select 中，不会静默改写数据。
2. **契约未列全的枚举字符串**：按 `docs/domain-model.md` 的变体名 snake_case 推断并逐字使用，
   例如 `receive_id_type` 的 `open_id` / `user_id` / `email`、`kind` 的 `p2p_user`、
   `source` 的 `builder_json` / `builder_template`、`DeliveryKind` 的 `firing_created` /
   `repeated` / `updated` / `resolved` / `test`、`DeliveryState` 的 `pending` / `active` /
   `superseded` / `recalled` / `resolved` / `failed`。
3. **`POST /channels` 的一次性 `hook_token`**：契约只说「响应附带 `hook_token` 明文一次」。
   类型里把它设为可选，并额外容忍可选的 `hook_url`；若后端只回 `hook_token`，页面用当前
   origin 拼接占位 URL 提示。创建/轮换的展示逻辑不依赖该字段的存在。
4. **`validate` 的 `errors`/`warnings` 元素结构未定义**：`types.ts` 用
   `ValidationItem = string | { code?, message?, path?, field? }`，渲染统一走
   `utils.validationText()`，两种形态都能显示。
5. **`AuditLog.actor` / `outcome` 的序列化未定义**：`docs/domain-model.md` §10 是 Rust 枚举
   （`Admin{user}` / `Skipped{reason}` 等）。前端用 `actorText()` / `outcomeLabel()` 两种形态
   （字符串或对象）都能渲染；`outcome=` 过滤按契约是标量，只提供 ok / skipped / failed。
6. **`must_change_password` 只来自登录响应**：`GET /auth/session` 仅返回
   `{authenticated, username}`，因此该标记只保存在当前标签页内存中，并在「设置」页显著提示。
7. **`validate` / `preview` 只接受 `card_json`**：契约请求体没有 `template_id`，所以
   `source = builder_template` 时，页面明确提示：如需校验/预览，请把同一份导出 JSON 粘到导入区，
   保存时仍按 `builder_template` 提交。
8. **`GET /channels/:id/policy` 可能 404**：契约说创建渠道会自动生成默认策略，但未规定 404 语义。
   页面把 404 当作「尚无策略」，用契约示例中的默认值填充表单并提示保存后会创建策略。
9. **`GET /card-templates` 无分页参数**：契约只列 `?kind=`，所以模板列表不做 offset 分页
   （仍兼容分页信封的字段）。
10. **`Delivery` 的 `channel_id` / `rendered_digest`**：只出现在 `docs/domain-model.md` §7，
    `docs/api.md` 的示例里没有，故在类型中是可选的，界面不依赖它们。
11. **跨渠道 `GET /hooks` 未使用**：契约存在该端点，但 Hook 列表直接取渠道详情里的 `hooks`
    （避免为跨渠道列表臆造 channel 名称映射）。
12. **`AppSettings.default_app_feed_link`**：`docs/domain-model.md` §8 有该字段，但
    `docs/api.md` 的 `GET /settings` 示例与本次 UI 需求都没有它，故未在界面暴露。
13. **`RecallFailed`**：`docs/business-rules.md` R2 提到该状态，但 §7 的 `DeliveryState`
    枚举里没有；前端把它加入联合类型与徽标映射，遇到时正常显示而不会退化成未知值。
14. **`link.url` 的 https 校验**：约束针对**渲染后**的值，因此模板化写法（如 `${external_url}`）
    在本地校验时放行；只有既不含 `${` 又不是 `https://` 开头的值才判为错误。
15. **pnpm 11 的 `pnpm-workspace.yaml`**：本环境预置了 `allowBuilds: {esbuild: ...}` 占位，
    已改为 `true` 以允许 esbuild 的 postinstall（否则 `vite build` 无法运行）。
