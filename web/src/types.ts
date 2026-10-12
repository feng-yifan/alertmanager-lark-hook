/**
 * DTOs mirroring `docs/api.md` verbatim.
 *
 * Naming rules derived from the contract:
 *  - JSON field names are `snake_case` (as in every example in `docs/api.md`).
 *  - Enum values are the *string literals that appear in `docs/api.md`*; where the
 *    contract only shows one member of an enum (e.g. `receive_id_type: "chat_id"`)
 *    the remaining members are the `snake_case` rendering of `docs/domain-model.md`
 *    (e.g. `OpenId` -> `open_id`). Every such guess is listed in `web/README.md`.
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
  bot_name?: string;
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
export interface HookCreated extends Partial<Hook> {
  id: UUID;
  channel_id: UUID;
  name: string;
  token: string;
  hook_url: string;
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
  kind?: 'firing' | 'resolved';
}

export interface TestMessageResponse {
  ok: boolean;
  delivery_id?: UUID;
  message_id?: string;
  [key: string]: unknown;
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

export interface TemplateVariable {
  name: string;
  type: string;
  default: string;
  builtin: boolean;
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

export interface CardMessageTemplate {
  id: UUID;
  kind: 'card_message';
  name: string;
  source: CardSource;
  card_json: CardJson | null;
  template_id: string | null;
  template_version_name: string | null;
  variables: TemplateVariable[];
  detected_variables: string[];
  is_default: boolean;
  created_at: ISODateTime;
  updated_at: ISODateTime;
}

export interface AppFeedCardTemplate {
  id: UUID;
  kind: 'app_feed_card';
  name: string;
  app_feed_spec: AppFeedCardSpec;
  detected_variables: string[];
  is_default: boolean;
  created_at: ISODateTime;
  updated_at: ISODateTime;
}

export type CardTemplate = CardMessageTemplate | AppFeedCardTemplate;

export interface CardMessageTemplateCreate {
  kind: 'card_message';
  name: string;
  source: CardSource;
  card_json?: CardJson | null;
  template_id?: string | null;
  template_version_name?: string | null;
  variables?: TemplateVariable[];
  is_default?: boolean;
}

export interface AppFeedCardTemplateCreate {
  kind: 'app_feed_card';
  name: string;
  app_feed_spec: AppFeedCardSpec;
  is_default?: boolean;
}

export type CardTemplateCreate = CardMessageTemplateCreate | AppFeedCardTemplateCreate;
export type CardTemplatePatch = Partial<CardTemplateCreate> & { name?: string };

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
  kind: CardTemplateKind;
  card_json?: CardJson | null;
  app_feed_spec?: AppFeedCardSpec | null;
}

export interface ValidateResponse {
  valid: boolean;
  errors: ValidationItem[];
  warnings: ValidationItem[];
  detected_variables: string[];
  unknown_variables: string[];
}

export interface PreviewRequest extends ValidateRequest {
  sample: 'firing' | 'resolved';
  channel_id?: UUID;
}

export interface PreviewResponse {
  rendered_card_json: CardJson | null;
  rendered_app_feed_spec: AppFeedCardSpec | null;
  variables: Record<string, unknown>;
}

export interface CardTemplateSendTestRequest {
  channel_id: UUID;
  sample?: 'firing' | 'resolved';
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
  channel: ChannelRef | null;
  hook: ChannelRef | null;
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
  /** `docs/domain-model.md` §10: Ok | Skipped{reason} | Failed{error}. */
  outcome: string;
}

export interface Overview {
  channels: { total: number; enabled: number };
  hooks: { total: number; enabled: number };
  alert_groups: { firing: number; resolved: number };
  deliveries_24h: { sent: number; failed: number; suppressed: number };
  recent_events: RecentEvent[];
}

/** Serialization of `AuditActor` (Admin{user}|AlertManager{hook}|System{job}) is unspecified. */
export type AuditActor = string | Record<string, unknown>;

export interface AuditLog {
  id: number;
  at: ISODateTime;
  actor: AuditActor;
  action: string;
  target: string | null;
  /** serialization of `Outcome` is unspecified beyond the `outcome=` filter being scalar. */
  outcome: string;
  detail: unknown;
}

export interface AuditLogListParams {
  action?: string;
  outcome?: string;
  limit?: number;
  offset?: number;
}
