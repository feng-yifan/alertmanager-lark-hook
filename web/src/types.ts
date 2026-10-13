/**
 * DTOs for the REST contract.
 *
 * 数据形状以**后端真实线格式**为准（`crates/alkh-server/tests/e2e.rs` 已 pin 住），
 * 当 `docs/api.md` 与真实响应不一致时（如卡片模板的嵌套 `card_message`、审计的带 tag
 * 对象、钩子的 `hook_token`），以后端实际行为为准。
 *
 * Naming rules:
 *  - JSON field names are `snake_case`.
 *  - Enum values are the string literals the server actually serializes (`snake_case`).
 *  - Timestamps are RFC 3339 UTC strings, IDs are UUID v4 strings.
 */

export type ISODateTime = string;
export type UUID = string;

/* ------------------------------------------------------------------ */
/* Envelopes                                                           */
/* ------------------------------------------------------------------ */

export interface ApiErrorBody {
  code: string;
  message: string;
  details?: unknown;
}

export interface ApiErrorEnvelope {
  error: ApiErrorBody;
}

export interface Paginated<T> {
  items: T[];
  total: number;
  limit: number;
  offset: number;
}

/* ------------------------------------------------------------------ */
/* 0. meta                                                             */
/* ------------------------------------------------------------------ */

export interface EnumOption {
  value: string;
  label: string;
}

export interface MetaVariable {
  name: string;
  group: string;
  type: string;
  description?: string;
}

export interface Meta {
  version: string;
  message_types: EnumOption[];
  repeat_policies: EnumOption[];
  resolved_policies: EnumOption[];
  group_update_policies: EnumOption[];
  match_ops: EnumOption[];
  /** 状态标签类型：{value,label} 对象数组（不是字符串数组）。 */
  status_label_types: EnumOption[];
  /** 应用消息流卡片按钮类型（default/primary/success），与 status_label_types 不是同一个枚举。 */
  feed_button_types: EnumOption[];
  /** 应用消息流卡片 biz_id 生成策略。 */
  biz_id_sources: EnumOption[];
  card_sources: EnumOption[];
  channel_kinds: EnumOption[];
  receive_id_types: EnumOption[];
  variables: MetaVariable[];
  /** 飞书卡片 JSON 2.0 组件目录（本地卡片构建工具的契约来源）。 */
  card_components: CardComponentMeta[];
  /** 告警积木（同源下发，避免前端另抄一份模板）。 */
  card_presets: CardPresetMeta[];
  /** 严重级别 → 卡片 header 颜色。 */
  severity_colors: Record<string, string>;
  /** `pin_unresolved` 生效的消息类型（目前仅 app_feed_card）。 */
  pin_supported_message_types: string[];
  token_prefix: string;
  hook_path_template: string;
}

/* ------------------------------------------------------------------ */
/* 1. Auth                                                             */
/* ------------------------------------------------------------------ */

export interface LoginRequest {
  username: string;
  password: string;
}

export interface LoginResponse {
  username: string;
  must_change_password: boolean;
}

export interface SessionResponse {
  authenticated: boolean;
  username: string;
  /**
   * `GET /auth/session` 也返回真实的 `must_change_password`（不只是登录响应），
   * 因此刷新页面后仍能触发「需修改密码」提示与强制跳转。
   */
  must_change_password: boolean;
}

export interface ChangePasswordRequest {
  current_password: string;
  new_password: string;
}

/* ------------------------------------------------------------------ */
/* 2. Settings                                                         */
/* ------------------------------------------------------------------ */

export interface RetentionPolicy {
  alert_group_days: number;
  delivery_days: number;
  audit_days: number;
}

export interface AppSettings {
  lark_app_id: string | null;
  /** write-only: the API never echoes the secret, only whether one is stored. */
  app_secret_set: boolean;
  lark_base_url: string;
  public_base_url: string | null;
  retention: RetentionPolicy;
}

export interface AppSettingsUpdate {
  lark_app_id?: string | null;
  /** `null` = unchanged, `""` = clear, non-empty = set. */
  app_secret?: string | null;
  lark_base_url?: string;
  public_base_url?: string | null;
  retention?: RetentionPolicy;
}

export interface SettingsCheckResponse {
  ok: boolean;
  chat_count?: number;
  error_code?: number | string;
  message?: string;
}

/* ------------------------------------------------------------------ */
/* 3. Channels / NotificationPolicy                                    */
/* ------------------------------------------------------------------ */

export type ReceiveIdType = 'chat_id' | 'open_id' | 'user_id' | 'email';
export type ChannelKind = 'group' | 'p2p_user';
export type ChannelSource = 'feishu_sync' | 'manual';

export type MessageType = 'card_message' | 'app_feed_card';
export type RepeatPolicy = 'suppress' | 'send' | 'send_and_revoke_previous';
export type ResolvedPolicy = 'send' | 'send_and_delete_previous' | 'update_previous';
export type GroupUpdatePolicy = 'update_previous' | 'send' | 'suppress';
export type MatchOp = '=' | '!=' | '=~' | '!~';

export interface LabelMatcher {
  name: string;
  op: MatchOp;
  value: string;
}

export interface NotificationPolicy {
  message_type: MessageType;
  card_template_id: UUID | null;
  pin_unresolved: boolean;
  repeat_policy: RepeatPolicy;
  resolved_policy: ResolvedPolicy;
  group_update_policy: GroupUpdatePolicy;
  label_matchers: LabelMatcher[];
  aggregation_window_secs: number;
  min_send_interval_secs: number;
  max_alerts_per_message: number;
  mute_until: ISODateTime | null;
  updated_at: ISODateTime;
}

/** `PUT /channels/:id/policy` performs a full replacement; audit fields are server-owned. */
export type NotificationPolicyInput = Omit<NotificationPolicy, 'updated_at'>;

export interface Hook {
  id: UUID;
  name: string;
  token_prefix: string;
  /** placeholder form, e.g. `/api/v1/hooks/alertmanager/<token>` */
  hook_path: string;
  /** placeholder URL in list responses; the real one is returned only on create/rotate */
  hook_url: string;
  enabled: boolean;
  last_called_at: ISODateTime | null;
  call_count: number;
  created_at: ISODateTime;
  /** present on `GET /hooks` (cross-channel list) */
  channel_id?: UUID;
  rotated_at?: ISODateTime | null;
}

/** Response of `POST /hooks` — the plaintext token is returned exactly once. */
export interface HookCreated extends Hook {
  /** 真实线格式字段名是 `hook_token`（不是 `token`）；`hook_url` 同样带回明文地址。 */
  hook_token: string;
}

export interface HookTokenResponse {
  token: string;
  hook_url: string;
}

export interface ChannelRef {
  id: UUID;
  name: string;
}

export interface Channel {
  id: UUID;
  name: string;
  receive_id: string;
  receive_id_type: ReceiveIdType;
  kind: ChannelKind;
  avatar_url: string | null;
  external: boolean;
  enabled: boolean;
  source: ChannelSource;
  bot_can_send: boolean;
  hook_count: number;
  policy: NotificationPolicy | null;
  hooks: Hook[];
  created_at: ISODateTime;
  updated_at: ISODateTime;
  /** `POST /channels` only: plaintext token of the auto-created default hook, shown once. */
  hook_token?: string;
  /** `POST /channels` only (not spelled out in `docs/api.md`, mirrors `hook_token`). */
  hook_url?: string;
}

export interface ChannelCreateRequest {
  name: string;
  receive_id: string;
  receive_id_type: ReceiveIdType;
  kind?: ChannelKind;
  enabled?: boolean;
}

export interface ChannelPatchRequest {
  name?: string;
  enabled?: boolean;
}

export interface ChannelListParams {
  enabled?: boolean;
  q?: string;
  limit?: number;
  offset?: number;
}

export interface ChannelSyncRequest {
  create_missing: boolean;
  import_names: boolean;
}

export interface ChannelSyncResult {
  discovered: number;
  created: number;
  updated: number;
  skipped: number;
}

export interface TestMessageRequest {
  message_type?: MessageType;
  card_template_id?: UUID;
  /** 后端请求字段名是 `sample`（不是 `kind`）；省略时后端按 firing 处理。 */
  sample?: 'firing' | 'resolved';
}

/* ------------------------------------------------------------------ */
/* 5. Card templates                                                   */
/* ------------------------------------------------------------------ */

export type CardTemplateKind = 'card_message' | 'app_feed_card';
export type CardSource = 'builder_json' | 'builder_template';
export type StatusLabelType = 'primary' | 'secondary' | 'success' | 'danger';
/** 飞书应用消息流卡片按钮类型（`open_app_feed_card_button.button_type`）。 */
export type FeedButtonType = 'default' | 'primary' | 'success';
/**
 * `docs/api.md` §5 shows the literal `"alert_group"`; `docs/domain-model.md` names the
 * variants `AlertGroupId | HookId | Custom`. The short forms are used here, and the UI
 * renders whatever unknown value the API returns instead of rewriting it.
 */
export type BizIdSource = 'alert_group' | 'hook' | 'custom';

/** CardKit JSON 2.0 is an arbitrary (but valid) JSON document. */
export type CardJson = Record<string, unknown>;

/* --- 组件目录（服务端 alkh-domain::card_catalog 下发，前端不另立规则） --- */

export type CardPropType = 'text' | 'markdown' | 'number' | 'bool' | 'enum' | 'px' | 'url';
export type CardPropAccessor = 'field' | 'button_url';

export interface CardPropMeta {
  name: string;
  label: string;
  /** 真实线格式字段名是 `type`（Rust 侧用 `ty` 避开关键字）。 */
  type: CardPropType;
  /** JSON 点号路径；`button_url` 走专用读写逻辑。 */
  path: string;
  accessor: CardPropAccessor;
  values: string[];
  max_chars: number | null;
  placeholder: string;
  hint: string;
}

export interface CardComponentMeta {
  tag: string;
  label: string;
  group: 'container' | 'display' | 'interactive' | 'element' | 'deprecated';
  description: string;
  /** 子节点所在字段（`elements` / `columns`）；叶组件为 null。 */
  children_key: string | null;
  /** 允许的子组件 tag；空数组表示任意。 */
  children: string[];
  /** 允许的父组件 tag；空数组表示任意位置。 */
  child_of: string[];
  /** 是否出现在组件面板；`column` / `plain_text` 由父组件自动创建。 */
  palette: boolean;
  /** 2.0 已废弃时的原因。 */
  deprecated?: string;
  /** 新建节点的默认 JSON（字符串，服务端保证结构合法）。 */
  default_json: string;
  props: CardPropMeta[];
}

export interface TemplateVariable {
  name: string;
  /** 真实线格式字段名是 `ty`（`string` / `number` / `bool`），不是 `type`。 */
  ty: string;
  default: string | null;
  builtin: boolean;
}

/** 告警积木：服务端下发的告警专用组件预设（保证只用真实变量）。 */
export interface CardPresetMeta {
  id: string;
  label: string;
  description: string;
  /** 插入画布的节点数组（JSON 文本）。 */
  nodes_json: string;
}

export interface StatusLabelSpec {
  text: string;
  type_firing: StatusLabelType;
  type_resolved: StatusLabelType;
}

export interface LinkSpec {
  enabled: boolean;
  url: string;
}

export interface FeedButtonSpec {
  text: string;
  url: string;
  button_type: FeedButtonType;
}

export interface AppFeedCardSpec {
  title: string;
  preview: string;
  status_label: StatusLabelSpec;
  link: LinkSpec;
  /** `docs/domain-model.md`: at most 2 buttons. */
  buttons: FeedButtonSpec[];
  close_notify: boolean;
  biz_id_source: BizIdSource;
}

/** 卡片消息内容：真实线格式是嵌在 `CardTemplate.card_message` 下的对象。 */
export interface CardMessageSpec {
  source: CardSource;
  card_json: CardJson | null;
  template_id: string | null;
  template_version_name: string | null;
  variables: TemplateVariable[];
}

export interface CardMessageTemplate {
  id: UUID;
  kind: 'card_message';
  name: string;
  /** 卡片内容嵌套在此字段下（顶层没有扁平的 source/card_json/variables）。 */
  card_message: CardMessageSpec | null;
  app_feed_spec: AppFeedCardSpec | null;
  detected_variables: string[];
  unknown_variables: string[];
  is_default: boolean;
  created_at: ISODateTime;
  updated_at: ISODateTime;
}

export interface AppFeedCardTemplate {
  id: UUID;
  kind: 'app_feed_card';
  name: string;
  card_message: CardMessageSpec | null;
  app_feed_spec: AppFeedCardSpec | null;
  detected_variables: string[];
  unknown_variables: string[];
  is_default: boolean;
  created_at: ISODateTime;
  updated_at: ISODateTime;
}

export type CardTemplate = CardMessageTemplate | AppFeedCardTemplate;

export interface CardMessageInput {
  source: CardSource;
  card_json?: CardJson | null;
  template_id?: string | null;
  template_version_name?: string | null;
  variables?: TemplateVariable[];
}

/** `POST`/`PATCH /card-templates` 的真实请求体；`name` 与 `kind` 是必填。 */
export interface CardTemplateInput {
  name: string;
  kind: CardTemplateKind;
  card_message?: CardMessageInput | null;
  app_feed_spec?: AppFeedCardSpec | null;
  is_default?: boolean;
}

export type CardTemplateCreate = CardTemplateInput;
/** PATCH 与创建共用同一请求体（`name`/`kind` 同样必填）。 */
export type CardTemplatePatch = CardTemplateInput;

/** `GET /card-templates/default`：内置默认模板内容。 */
export interface CardTemplateDefaults {
  card_message: CardMessageSpec;
  app_feed_spec: AppFeedCardSpec;
}

/**
 * `docs/api.md` does not fix the shape of validation entries, so both the plain
 * string form and a structured form are accepted and rendered defensively.
 */
export type ValidationItem =
  | string
  | {
      code?: string;
      message?: string;
      path?: string;
      field?: string;
    };

export interface ValidateRequest {
  name: string;
  kind: CardTemplateKind;
  card_message?: CardMessageInput | null;
  app_feed_spec?: AppFeedCardSpec | null;
  is_default?: boolean;
}

export interface ValidateResponse {
  valid: boolean;
  errors: ValidationItem[];
  warnings: ValidationItem[];
  detected_variables: string[];
  unknown_variables: string[];
}

export interface PreviewRequest extends ValidateRequest {
  sample?: 'firing' | 'resolved';
  channel_id?: UUID;
  /** 预览/测试发送的 mock 数据（P1）。 */
  mock?: MockSpec;
}

/**
 * 预览 / 测试发送的 mock 数据。
 *
 * - `alert_group_id`：用一条历史告警分组作为基线数据源（设置后 `sample` 只作展示）；
 * - `variables`：逐项覆盖，支持 `label:` / `annotation:` 动态 key；未知 key 会被直接写入上下文。
 */
export interface MockSpec {
  alert_group_id?: UUID | null;
  variables?: Record<string, string>;
}

export type MockSource = 'builtin' | 'alert_group';

/** `GET /card-templates/mock` 的响应。 */
export interface MockBaseline {
  source: MockSource;
  sample: 'firing' | 'resolved';
  alert_group_id: UUID | null;
  /** 历史分组摘要，供面板回显；`source = builtin` 时为 null。 */
  alert_group_label: string | null;
  /** 变量名 → 基线值（覆盖前的原始值）。 */
  variables: Record<string, string>;
}

export interface PreviewResponse {
  rendered_card_json: CardJson | null;
  rendered_app_feed_spec: AppFeedCardSpec | null;
  variables: Record<string, unknown>;
}

export interface CardTemplateSendTestRequest {
  channel_id: UUID;
  sample?: 'firing' | 'resolved';
  mock?: MockSpec;
}

/** `POST /card-templates/send-test`：用**未保存的草稿**模板发测试消息。 */
export interface DraftSendTestRequest extends ValidateRequest {
  channel_id: UUID;
  sample?: 'firing' | 'resolved';
  mock?: MockSpec;
}

/* ------------------------------------------------------------------ */
/* 6. Alert groups / deliveries                                        */
/* ------------------------------------------------------------------ */

export type GroupStatus = 'firing' | 'resolved';
export type AlertStatus = 'firing' | 'resolved';

export interface AlertInstance {
  fingerprint: string;
  status: AlertStatus;
  labels: Record<string, string>;
  annotations: Record<string, string>;
  starts_at: ISODateTime;
  ends_at: ISODateTime | null;
  generator_url: string | null;
}

export interface AlertGroup {
  id: UUID;
  group_key: string;
  receiver: string;
  /** 只有详情接口（`GET /alert-groups/:id`）返回，列表接口没有。 */
  channel?: ChannelRef | null;
  /** 只有详情接口返回，列表接口没有。 */
  hook?: ChannelRef | null;
  status: GroupStatus;
  alertname: string | null;
  severity: string | null;
  group_labels: Record<string, string>;
  common_labels: Record<string, string>;
  common_annotations: Record<string, string>;
  alerts: AlertInstance[];
  firing_count: number;
  resolved_count: number;
  truncated_alerts: number;
  event_count: number;
  first_seen_at: ISODateTime;
  last_seen_at: ISODateTime;
  last_notified_at: ISODateTime | null;
  resolved_at: ISODateTime | null;
  active_delivery_id: UUID | null;
  /** `GET /alert-groups/:id` embeds the group's deliveries. */
  deliveries?: Delivery[];
}

export type DeliveryKind = 'firing_created' | 'repeated' | 'updated' | 'resolved' | 'test';

/**
 * `docs/domain-model.md` §7 lists Pending|Active|Superseded|Recalled|Resolved|Failed.
 * R2 additionally mentions a `RecallFailed` bookkeeping state, kept here so the UI
 * renders it instead of falling through.
 */
export type DeliveryState =
  | 'pending'
  | 'active'
  | 'superseded'
  | 'recalled'
  | 'recall_failed'
  | 'resolved'
  | 'failed';

export interface Delivery {
  id: UUID;
  alert_group_id: UUID;
  kind: DeliveryKind;
  message_type: MessageType;
  state: DeliveryState;
  card_template_id: UUID | null;
  lark_message_id: string | null;
  card_entity_id: string | null;
  app_feed_biz_id: string | null;
  attempt: number;
  last_error: string | null;
  created_at: ISODateTime;
  updated_at: ISODateTime;
  /** present in some responses (`docs/domain-model.md` §7 lists them on the entity). */
  channel_id?: UUID;
  rendered_digest?: string;
}

export interface AlertGroupListParams {
  status?: string;
  channel_id?: string;
  severity?: string;
  q?: string;
  limit?: number;
  offset?: number;
}

export interface DeliveryListParams {
  alert_group_id?: string;
  channel_id?: string;
  state?: string;
  limit?: number;
  offset?: number;
}

/* ------------------------------------------------------------------ */
/* 7. Overview / audit logs                                            */
/* ------------------------------------------------------------------ */

export interface RecentEvent {
  at: ISODateTime;
  action: string;
  target: string | null;
  /** 带 tag 的对象：`{"type":"ok"}` / `{"type":"skipped","reason":...}` / `{"type":"failed","error":...}`。 */
  outcome: AuditOutcome;
}

export interface Overview {
  channels: { total: number; enabled: number };
  hooks: { total: number; enabled: number };
  alert_groups: { firing: number; resolved: number };
  deliveries_24h: { sent: number; failed: number; suppressed: number };
  recent_events: RecentEvent[];
}

/** `AuditActor` 是带 tag 的对象：`{"type":"admin","username":...}` / `{"type":"alert_manager","hook_id":...}` / `{"type":"system","job":...}`。 */
export type AuditActor = string | Record<string, unknown>;

/** `Outcome` 是带 tag 的对象：`{"type":"ok"}` / `{"type":"skipped","reason":...}` / `{"type":"failed","error":...}`。 */
export type AuditOutcome = string | Record<string, unknown>;

export interface AuditLog {
  id: number;
  at: ISODateTime;
  actor: AuditActor;
  action: string;
  target: string | null;
  outcome: AuditOutcome;
  detail: unknown;
}

export interface AuditLogListParams {
  action?: string;
  outcome?: string;
  limit?: number;
  offset?: number;
}
