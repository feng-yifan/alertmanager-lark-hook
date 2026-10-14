# web — alertmanager-lark-hook 管理后台

Rust 服务（`crates/alkh-server`）内嵌的 TypeScript 管理后台 SPA：把 Prometheus AlertManager
的告警投递到飞书 / Lark。数据类型以**后端真实线格式**为准（`crates/alkh-server/tests/e2e.rs`
已把真实形状 pin 住），当 `docs/api.md` 与实际响应不一致时以前端已实测的后端行为为准；
业务规则参考 `docs/business-rules.md` 与 `docs/domain-model.md`（实体与枚举）。
界面上不出现规则编号（Rxx）与接口字段名（snake_case），一律用中文表述；字段名只在 `docs/api.md` 里查。

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
  （`/auth/login`、`/auth/session` 例外：登录表单的 401 按 `unauthenticated` 显示为用户名/密码错误，
  而不是「会话已失效」）。
- 错误信封 `{"error":{"code","message"}}`（含可选 `details`）统一解析为 `ApiError`。
  后端 409 只有统一的 `conflict` code（`channel_has_deliveries` / `template_in_use` /
  `duplicate_receive_id` 等细分 code 后端从不产生），因此前端统一按通用冲突提示处理，
  不提供依赖细分 code 的「强制删除」分支。

## 页面清单

| 路由 | 文件 | 内容 |
|---|---|---|
| `/login` | `pages/LoginPage.tsx` | 登录表单、会话引导、重定向回原路径 |
| `/` | `pages/OverviewPage.tsx` | `GET /overview` 统计卡（渠道 / Hook / firing·resolved 分组 / 近 24h sent·failed·suppressed）+ 最近事件 |
| `/channels` | `pages/ChannelsPage.tsx` | 列表、启用开关、搜索、从飞书同步、手动创建（含一次性 `hook_token` 展示） |
| `/channels/:id` | `pages/ChannelDetailPage.tsx` + `pages/channels/*` | 三个 Tab：发送规则（完整通知策略表单 + 投递决策表）、接收 Hook（创建 / 轮换 / 重命名 / 启停 / 删除 + 一次性 token）、发送测试 |
| `/card-templates` | `pages/CardTemplatesPage.tsx` | 模板列表（裸数组）、设为默认（回传整份模板，避免后端整份替换时丢内容）、删除（409 通用冲突提示） |
| `/card-templates/new`、`/card-templates/:id` | `pages/CardTemplateEditorPage.tsx` | 可视化拖拽画布 + JSON 编辑器（同一份文档）+ 搭建工具导入 + 变量面板 + 校验 / 预览；已存在的模板可发送测试（`/card-templates/:id/send-test`） |
| `/alert-groups` | `pages/AlertGroupsPage.tsx` | status / channel / severity / 搜索过滤，分页，删除记录 |
| `/alert-groups/:id` | `pages/AlertGroupDetailPage.tsx` | 分组元数据、告警表格（labels / annotations / generator_url）、labels 面板、投递时间线（重试 / 撤回） |
| `/deliveries` | `pages/DeliveriesPage.tsx` | 状态徽标、过滤（alert_group_id / channel / state）、重试 / 撤回 |
| `/settings` | `pages/SettingsPage.tsx` | 飞书凭据（app_secret 只写）、base URL、保留天数、测试凭据、修改密码、审计日志查看器 |

所有数据视图都有加载中 / 空 / 错误（可重试）状态；所有操作都有 toast 反馈，按钮在请求期间禁用。
Hook 地址不再「只显示一次」：列表直接展示服务端按「设置 → 对外访问地址」拼好的完整 `hook_url`
并提供复制按钮（**不使用** `window.location.origin`，因为 hook 可能只在集群内可达）；
创建 / 轮换仍会弹一次确认面板，但关闭后随时能在列表里再看。
旧数据（0003 迁移前签发、无密文可解）显示遮蔽地址并提示轮换一次。

## 目录结构

```
web/
  index.html
  vite.config.ts
  tsconfig.json
  pnpm-workspace.yaml     # pnpm 11 的 allowBuilds（批准 esbuild 的 postinstall）
  public/                 # 原样复制到 dist/ 根目录的静态资源
    favicon.ico           # 品牌主图标，原样入库（不经生成）
    favicon-16x16.png     # 由 favicon.ico 派生，与主图标同源
    favicon-32x32.png
    apple-touch-icon.png  # 180×180，由 docs/images/banner.png 派生（`make icons`）
  src/
    main.tsx              # 挂载 + Provider 组合
    App.tsx               # 路由 + 侧边栏布局 + RequireAuth 守卫
    api.ts                # 唯一 fetch 客户端（ApiError / 错误信封 / 401 处理 / 全部端点）
    types.ts              # 契约 DTO（字段名与枚举值逐字对齐 docs/api.md）
    labels.ts             # 枚举中文标签、投递决策表 DECISION_TABLE、静默/过滤说明文案
    meta.tsx              # GET /meta + 契约兜底枚举 + useMeta()
    session.tsx           # 会话引导 / 登录 / 登出 / 改密标记
    toast.tsx             # 轻量 toast
    hooks.ts              # useAsync / useDebounced
    utils.ts              # 时间、JSON、actor/outcome/validation 防御式渲染
    styles.css            # 全部样式（CSS 变量 + 深色模式）
    components/ui.tsx     # Button/Field/Input/Select/Textarea/Switch/RadioGroup/Badge/
                          # StatusBadge/Card/Modal/ConfirmModal/Tabs/Pagination/
                          # CopyButton/JsonBlock/DescriptionList/Banner/状态组件
    card-builder/         # 卡片模板可视化编辑器（文档即 CardKit JSON 2.0）
      Builder.tsx         # 三栏布局 + DndContext + 撤销/重做 + 选中项
      Canvas.tsx          # 画布：卡片标题块、节点树、卡片设置入口、积木徽标
      drag.tsx            # 指针拖拽引擎（@dnd-kit/core + DragOverlay 半透明浮层）
      dropTarget.ts       # 落点几何（纯函数，可单独验证）
      presets.ts          # 积木识别：画布节点显示左侧积木名（结构指纹）
      Inspector.tsx       # 属性面板：标题 / 节点属性 / 卡片设置，变量按位置过滤 + 绑定胶囊
      Outline.tsx         # 结构大纲：可点击的缩进树（深嵌套导航）
      Palette.tsx         # 结构大纲 + 积木与飞书组件拖拽源
      catalog.ts          # 面板分组与积木解析
      model.ts            # 文档模型：路径、增删改移动、拖放校验、展开估算、变量判定与诊断
    pages/                # 见上表；渠道详情的三个 Tab 在 pages/channels/
```

## 卡片编辑器的交互约定

- **文档就是 CardKit JSON 2.0**，不存在「本地模型 → 编译」这一步，所以画布、
  `/card-templates/preview`、真实投递三者一致。
- **拖拽**用 `@dnd-kit/core` 的指针拖拽（不是原生 HTML5 DnD）：
  - 落点由**指针所在区域**决定（子组件上半区 → 插到它前面，下半区 → 插到后面），
    不再要求指针精确停在两个组件之间的细缝里；算法在 `dropTarget.ts`，是纯函数；
  - 拖拽浮层是 `DragOverlay` 渲染的半透明块，不会盖住画布里的蓝色落点条；
  - 嵌套容器（分栏 / 折叠面板 / 循环体）按「包含指针且面积最小」选最深的列表。
- **节点名称**（`builder_meta`）：属性面板里的「节点名称」写进 `card_message.builder_meta.labels`
  （key 是节点的 `element_id`），随模板落库——节点被移动 / 复制 / 改内容后名字依然跟着它；
  拖入积木时会自动按积木名记一次。节点没有 `element_id` 时改名会先补一个（循环体内不分配，
  因为一份模板会复制多份）。名称只影响画布显示，不参与渲染、不会发给飞书。
- **积木识别**（`presets.ts`）：从左侧拖入的积木在画布上**仍然显示积木名**
  （结构指纹比对，忽略 `element_id`），左右面板一一对应；改动过内容的节点会退回显示
  组件类型名。旧的「变量版告警列表」节点带一个「转为循环」按钮。
- **卡片标题是独立组件**：画布顶部单独一块，点击编辑标题 / 副标题 / 配色；
  卡片设置（消息摘要 / 内容方向 / 内边距）有画布底部与工具栏两个入口。
- **变量按位置过滤**（`Inspector.tsx` 的 `variablesFor`）：`${item.*}` 只出现在
  「循环」组件的子组件里；循环体外的变量下拉不会列出它们。
- **变量绑定可视化**（`model.ts` 的 `classifyVariable` + `variableTokensInText`）：
  文本字段下方把已写入的 `${...}` 渲染成胶囊——**蓝=可用、黄=越权（循环外用了 `item.*`）、
  红=不在变量表里**，悬停给出说明。判定用完整变量表，下拉才按位置过滤。
- **诊断面板**（`model.ts` 的 `analyzeCard`）：集中列出「循环外用了 `item.*`」「未声明的变量」
  「空的循环体」「单个循环展开超过 200 个组件」「整卡超 200 / 30KB」，每条都能**点击定位**
  到出问题的节点。
- **展开代价可见**：循环节点头显示「展开约 N 个组件」，超过 200 时提示块变红。
- **结构大纲**（`Outline.tsx`）：左栏顶部是可点击的缩进树（卡片标题 / 各组件 / 卡片设置），
  深嵌套（循环 → 分栏 → 条件）用它导航比在画布上找更稳。
- **本地指令**：`repeat` / `if` 来自服务端组件目录（`group = "directive"`），
  保存的是指令本身，预览 / 发送时由服务端展开（见根目录 `docs/business-rules.md` §5.3）。
  工具栏的「展开约 N」是展开后的组件数估算（指令本身不计数）。
- **测试发送分两种**：卡片编辑器的「发送测试」只发送、**什么都不落库**（验证样式用，
  见 `docs/business-rules.md` §5.1）；渠道页的「发送测试消息」走完整链路并写投递记录。
- **`element_id` 与循环**：编辑器不给循环体内的节点分配 `element_id`（拖进循环体即剥掉，
  复制循环时也会清掉副本内的 id）；一份模板会变多份、id 必然重复，飞书会以 `300301` 拒收。
  服务端展开时还会再剥一次副本上的 `element_id`；循环体外的 id 正常保留。

## 契约中未明确、由前端自行决定的部分

1. **`biz_id_source` 取值**：`docs/api.md` §5 只出现 `"alert_group"`，`docs/domain-model.md`
   写作 `AlertGroupId | HookId | Custom`。采用 `alert_group` / `hook` / `custom` 三个短名；
   若接口返回其它值，`enumOptions()` 会把它作为「未知选项」保留在 select 中，不会静默改写数据。
2. **契约未列全的枚举字符串**：按 `docs/domain-model.md` 的变体名 snake_case 推断并逐字使用，
   例如 `receive_id_type` 的 `open_id` / `user_id` / `email`、`kind` 的 `p2p_user`、
   `source` 的 `builder_json` / `builder_template`、`DeliveryKind` 的 `firing_created` /
   `repeated` / `updated` / `resolved` / `test`、`DeliveryState` 的 `pending` / `active` /
   `superseded` / `recalled` / `resolved` / `failed`。
3. **`hook_token` 字段名**：`POST /channels`、`POST /hooks` 的字段名都是 `hook_token`
   （不是 `token`），并同时返回完整 `hook_url`；`POST /hooks/:id/rotate` 的响应字段仍是 `token`。
   它只用于创建 / 轮换的即时反馈，列表里的 `hook_url` 才是长期可复制的地址。
4. **`validate` 的 `errors`/`warnings` 元素结构**：后端实际返回字符串数组；`types.ts` 保留
   `ValidationItem = string | { code?, message?, path?, field? }`，渲染统一走
   `utils.validationText()`，两种形态都能显示。
5. **`AuditLog.actor` / `outcome` 是 serde 的 internally-tagged 对象**：
   `actor = {"type":"admin","username":...} | {"type":"alert_manager","hook_id":...} | {"type":"system","job":...}`，
   `outcome = {"type":"ok"} | {"type":"skipped","reason":...} | {"type":"failed","error":...}`。
   `utils.actorText()` / `outcomeText()` 按 tag 解析并显示 username / hook id，同时兼容旧的字符串形态；
   `outcome=` 过滤只提供 ok / skipped / failed 三个标量。
6. **`must_change_password` 同时来自会话与登录响应**：`GET /auth/session` 返回
   `{authenticated, username, must_change_password}`，因此刷新页面后仍会显示「需修改密码」并强制跳转 `/settings`。
7. **卡片模板请求体是嵌套的 `card_message`**：`POST`/`PATCH /card-templates`、
   `/card-templates/validate`、`/card-templates/preview` 都要求 `{name, kind, card_message?}`，
   其中 `card_message = {source, card_json, template_id, template_version_name, variables}`，
   变量类型字段名是 `ty`（不是 `type`）。把 `card_json` 放在顶层会被后端静默忽略并回退成内置默认卡片
   （HTTP 201 但内容丢失）；PATCH 是整份替换，`setDefault` 也必须回传完整模板内容。
8. **`GET /channels/:id/policy` 可能 404**：契约说创建渠道会自动生成默认策略，但未规定 404 语义。
   页面把 404 当作「尚无策略」，用契约示例中的默认值填充表单并提示保存后会创建策略。
9. **`GET /card-templates` 与 `GET /hooks` 返回裸数组**：模板列表不做 offset 分页；
   `api.normalizeList()` 同时容忍 `T[]` 与旧的 `{items:T[]}` 形态。
10. **`Delivery` 的 `channel_id` / `rendered_digest`**：只出现在 `docs/domain-model.md` §7，
    `docs/api.md` 的示例里没有，故在类型中是可选的，界面不依赖它们。
11. **跨渠道 `GET /hooks` 未使用**：契约存在该端点（返回裸数组），但 Hook 列表直接取渠道详情里的
    `hooks`（避免为跨渠道列表臆造 channel 名称映射）。
13. **`RecallFailed`**：`docs/business-rules.md` 提到该状态，但 §7 的 `DeliveryState`
    枚举里没有；前端把它加入联合类型与徽标映射，遇到时正常显示而不会退化成未知值。
14. **`link.url` 的 https 校验**：约束针对**渲染后**的值，因此模板化写法（如 `${external_url}`）
    在本地校验时放行；只有既不含 `${` 又不是 `https://` 开头的值才判为错误。
15. **pnpm 11 的 `pnpm-workspace.yaml`**：本环境预置了 `allowBuilds: {esbuild: ...}` 占位，
    已改为 `true` 以允许 esbuild 的 postinstall（否则 `vite build` 无法运行）。
16. **发送测试的请求/响应**：`POST /channels/:id/test-message` 与
    `POST /card-templates/:id/send-test` 的请求字段是 `sample`（firing / resolved，不是 `kind`），
    响应是完整的 `Delivery`（没有 `{ok, delivery_id, message_id}` 信封），页面按 `state` 判断结果。
17. **`POST /settings/check` 不返回 `bot_name`**：响应只有 `{ok, chat_count, error_code, message}`，
    页面不再显示「机器人：未知」。
18. **`AlertGroup` 列表不返回 `channel` / `hook`**：只有详情接口有；列表页在缺字段时显示「—」。
