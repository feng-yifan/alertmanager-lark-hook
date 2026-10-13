/**
 * Chinese labels for enum values.
 *
 * Display-only fallbacks for values that are **not** served by `GET /api/v1/meta`
 * (e.g. delivery states in the audit/delivery lists). Anything that `/meta` does
 * serve — policies, match ops, channel kinds — must be read from `meta` instead of
 * being duplicated here, otherwise the two copies drift and the UI renders stale
 * (or malformed) option lists.
 */
import type {
  ChannelKind,
  ChannelSource,
  DeliveryKind,
  DeliveryState,
  ReceiveIdType,
} from './types';

export const DELIVERY_STATE_LABEL: Record<string, string> = {
  pending: '待发送',
  active: '已发送',
  superseded: '已被取代',
  recalled: '已撤回',
  recall_failed: '撤回失败',
  resolved: '已更新为已恢复',
  failed: '发送失败',
};

export const DELIVERY_KIND_LABEL: Record<string, string> = {
  firing_created: '新告警分组',
  repeated: '重复通知',
  updated: '分组内容变化',
  resolved: '恢复通知',
  test: '测试消息',
};


export const GROUP_STATUS_LABEL: Record<string, string> = {
  firing: '未解决',
  resolved: '已恢复',
};

export const ALERT_STATUS_LABEL: Record<string, string> = {
  firing: '未解决',
  resolved: '已恢复',
};

export const CHANNEL_KIND_LABEL: Record<string, string> = {
  group: '群聊',
  p2p_user: '单聊用户',
};

export const RECEIVE_ID_TYPE_LABEL: Record<string, string> = {
  chat_id: 'chat_id（群）',
  open_id: 'open_id（用户）',
  user_id: 'user_id',
  email: 'email',
};

export const CHANNEL_SOURCE_LABEL: Record<string, string> = {
  feishu_sync: '飞书同步',
  manual: '手动创建',
};

export const MATCH_OP_LABEL: Record<string, string> = {
  '=': '等于',
  '!=': '不等于',
  '=~': '正则匹配',
  '!~': '正则不匹配',
};

export const CARD_SOURCE_LABEL: Record<string, string> = {
  builder_json: '导入卡片 JSON（builder_json）',
  builder_template: '引用搭建工具模板（builder_template）',
};

export function label(map: Record<string, string>, value: string | null | undefined, fallback = '—'): string {
  if (!value) return fallback;
  return map[value] ?? value;
}

export const RECEIVE_ID_TYPE_VALUES: ReceiveIdType[] = ['chat_id', 'open_id', 'user_id', 'email'];
export const CHANNEL_KIND_VALUES: ChannelKind[] = ['group', 'p2p_user'];
export const CHANNEL_SOURCE_VALUES: ChannelSource[] = ['feishu_sync', 'manual'];
export const DELIVERY_STATE_VALUES: DeliveryState[] = [
  'pending',
  'active',
  'superseded',
  'recalled',
  'recall_failed',
  'resolved',
  'failed',
];
export const DELIVERY_KIND_VALUES: DeliveryKind[] = [
  'firing_created',
  'repeated',
  'updated',
  'resolved',
  'test',
];

/**
 * `docs/business-rules.md` §4 decision table, rendered read-only in the policy editor
 * so the operator can see exactly what each combination does.
 */
export interface DecisionRow {
  event: string;
  field: string;
  value: string;
  valueLabel: string;
  action: string;
}

export const DECISION_TABLE: DecisionRow[] = [
  { event: 'NewFiring', field: '—', value: '—', valueLabel: '始终投递', action: 'SendNew：创建卡片实体 → 发送 → Delivery=Active' },
  { event: 'Repeated', field: 'repeat_policy', value: 'send', valueLabel: '发送', action: 'SendNew，旧 Delivery → Superseded' },
  { event: 'Repeated', field: 'repeat_policy', value: 'suppress', valueLabel: '不发送（默认）', action: 'Suppress：不发送，仅更新 last_seen_at' },
  {
    event: 'Repeated',
    field: 'repeat_policy',
    value: 'send_and_revoke_previous',
    valueLabel: '发送并移除旧告警',
    action: 'SendNewThenRevoke：发送新消息后撤回旧消息，旧 → Recalled',
  },
  { event: 'Updated', field: 'group_update_policy', value: 'update_previous', valueLabel: '更新旧告警（默认）', action: 'UpdateInPlace：原地更新当前 Active 卡片内容' },
  { event: 'Updated', field: 'group_update_policy', value: 'send', valueLabel: '发送新消息', action: 'SendNew，旧 → Superseded' },
  { event: 'Updated', field: 'group_update_policy', value: 'suppress', valueLabel: '不发送', action: 'Suppress' },
  { event: 'Resolved', field: 'resolved_policy', value: 'send', valueLabel: '发送（默认）', action: 'SendNew（恢复卡片），旧 → Superseded' },
  { event: 'Resolved', field: 'resolved_policy', value: 'send_and_delete_previous', valueLabel: '发送并删除旧告警', action: 'SendNewThenRevoke，旧 → Recalled' },
  { event: 'Resolved', field: 'resolved_policy', value: 'update_previous', valueLabel: '更新旧告警', action: 'UpdateInPlace：把旧卡片原地更新为已恢复，旧 → Resolved' },
];

/** R4：卡片消息（`im/v1/messages`）没有官方的置顶/即时提醒字段。 */
export const R4_CARD_MESSAGE_CAVEAT =
  'R4：卡片消息（im/v1/messages）没有官方的置顶/即时提醒字段，因此本项目不提供「即时提醒」开关。';

export const R5_MUTE_NOTE =
  'R5：mute_until 未过期时抑制所有发送类动作，但事件分类与分组状态照常推进——解除静默后不会补发历史消息，只反映最新状态。';

export const R11_FILTER_NOTE =
  'R11：多个 matcher 为 AND 关系；匹配对象是每条告警的 labels ∪ commonLabels（告警自身 label 优先）。' +
  '分组内至少一条告警命中即投递，未命中的告警会从消息中剔除；过滤后为空则 skipped:filtered。matcher 为空表示全部通过。';
