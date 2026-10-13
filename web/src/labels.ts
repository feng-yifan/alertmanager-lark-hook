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
  chat_id: '群 ID（chat_id）',
  open_id: '用户 ID（open_id）',
  user_id: '用户 ID（user_id）',
  email: '用户邮箱',
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
  builder_json: '卡片搭建工具 JSON（JSON 2.0）',
  builder_template: '引用搭建工具模板（已停用）',
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
 * 投递决策表（只读）：把策略组合与它到底会发生什么列出来，
 * 避免运维只能靠试。
 */
export interface DecisionRow {
  event: string;
  field: string;
  value: string;
  valueLabel: string;
  action: string;
}

export const DECISION_TABLE: DecisionRow[] = [
  { event: '首次告警', field: '—', value: '—', valueLabel: '始终投递', action: '发送新消息：创建卡片实体 → 发送 → 投递变为「已发送」' },
  { event: '重复通知', field: '重复告警策略', value: 'send', valueLabel: '发送', action: '发送新消息，旧投递变为「已被取代」' },
  { event: '重复通知', field: '重复告警策略', value: 'suppress', valueLabel: '不发送（默认）', action: '不发送，只更新「最近出现时间」' },
  { event: '重复通知', field: '重复告警策略', value: 'send_and_revoke_previous', valueLabel: '发送并移除旧告警', action: '发送新消息后撤回旧消息，旧投递变为「已撤回」' },
  { event: '内容变化', field: '内容变化策略', value: 'update_previous', valueLabel: '更新旧告警（默认）', action: '原地更新当前「已发送」的那张卡片' },
  { event: '内容变化', field: '内容变化策略', value: 'send', valueLabel: '发送新消息', action: '发送新消息，旧投递变为「已被取代」' },
  { event: '内容变化', field: '内容变化策略', value: 'suppress', valueLabel: '不发送', action: '不发送' },
  { event: '全部恢复', field: '恢复策略', value: 'send', valueLabel: '发送（默认）', action: '发送恢复卡片，旧投递变为「已被取代」' },
  { event: '全部恢复', field: '恢复策略', value: 'send_and_delete_previous', valueLabel: '发送并删除旧告警', action: '发送新消息后撤回旧消息，旧投递变为「已撤回」' },
  { event: '全部恢复', field: '恢复策略', value: 'update_previous', valueLabel: '更新旧告警', action: '把旧卡片原地更新为已恢复，旧投递变为「已更新为已恢复」' },
];

export const MUTE_NOTE =
  '静默未结束前会抑制所有发送类动作，但事件分类与分组状态照常推进——解除静默后不会补发历史消息，只反映最新状态。';

export const PIN_NOTE =
  '开启后：分组内还有未解决告警时，把当前卡片 Pin 到会话的 Pin 列表；全部恢复后自动取消 Pin。' +
  '卡片被新消息取代时会改用新消息。这是飞书的 Pin 功能（Pin 标签页里可 Pin 多条），' +
  '不是固定在会话顶部的「置顶消息」（后者只能置顶 1 条，且是另一套接口）。' +
  '飞书侧要求机器人在群内、且该群允许机器人 Pin 消息（部分群只允许群主/管理员），' +
  '失败只记录日志与指标，不影响告警投递。';

export const FILTER_NOTE =
  '多个匹配条件之间是「与」关系；匹配对象是每条告警的标签与分组公共标签的并集（告警自身标签优先）。' +
  '分组内至少一条告警命中即投递，未命中的告警会从消息中剔除；过滤后为空则不发送。未配置匹配条件表示全部通过。';
