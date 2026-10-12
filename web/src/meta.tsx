/**
 * Enum metadata for form controls, loaded from `GET /api/v1/meta`.
 *
 * Enum lists are `{value,label}` objects on the wire. The literal below is a fallback
 * used only when the endpoint is unreachable; remote data always wins. Every value is
 * also normalised through `normalizeEnumList`/`normalizeEnumMap` so a shape change on
 * either side degrades into a readable option list instead of rendering
 * `[object Object]` React keys.
 *
 * Source of truth: `crates/alkh-app/src/meta.rs`, pinned by
 * `meta::tests::meta_wire_format_is_enum_option_objects` and by
 * `crates/alkh-server/tests/e2e.rs::meta_endpoint_pins_the_wire_format`.
 */
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { getMeta } from './api';
import type { EnumOption, Meta } from './types';

export const FALLBACK_META: Meta = {
  version: '0.0.0',
  message_types: [
    { value: 'card_message', label: '卡片消息' },
    { value: 'app_feed_card', label: '应用消息流消息' },
  ],
  repeat_policies: [
    { value: 'suppress', label: '不发送' },
    { value: 'send', label: '发送' },
    { value: 'send_and_revoke_previous', label: '发送并移除旧告警' },
  ],
  resolved_policies: [
    { value: 'send', label: '发送' },
    { value: 'send_and_delete_previous', label: '发送并删除旧告警' },
    { value: 'update_previous', label: '更新旧告警' },
  ],
  group_update_policies: [
    { value: 'update_previous', label: '更新旧告警' },
    { value: 'send', label: '发送新消息' },
    { value: 'suppress', label: '不发送' },
  ],
  match_ops: [
    { value: '=', label: '等于' },
    { value: '!=', label: '不等于' },
    { value: '=~', label: '正则匹配' },
    { value: '!~', label: '正则不匹配' },
  ],
  status_label_types: [
    { value: 'primary', label: '主类型' },
    { value: 'secondary', label: '次要类型' },
    { value: 'success', label: '成功类型' },
    { value: 'danger', label: '危险类型' },
  ],
  feed_button_types: [
    { value: 'default', label: '默认' },
    { value: 'primary', label: '主要' },
    { value: 'success', label: '成功' },
  ],
  biz_id_sources: [
    { value: 'alert_group', label: '按告警分组（同分组复用同一张卡片，默认）' },
    { value: 'hook', label: '按 Hook + 告警分组' },
  ],
  card_sources: [
    { value: 'builder_json', label: '卡片搭建工具 JSON（JSON 2.0）' },
    { value: 'builder_template', label: '卡片搭建工具模板（template_id）' },
  ],
  channel_kinds: [
    { value: 'group', label: '群聊' },
    { value: 'p2p_user', label: '单聊用户' },
  ],
  receive_id_types: [
    { value: 'chat_id', label: '群 chat_id' },
    { value: 'open_id', label: '用户 open_id' },
    { value: 'user_id', label: '用户 user_id' },
    { value: 'email', label: '用户邮箱' },
  ],
  pin_supported_message_types: ['app_feed_card'],
  token_prefix: 'alkh_',
  hook_path_template: '/api/v1/hooks/alertmanager/{token}',
  variables: [
    { name: 'alertname', group: 'alert', type: 'string', description: '告警名称' },
    { name: 'severity', group: 'alert', type: 'string' },
    { name: 'instance', group: 'alert', type: 'string' },
    { name: 'job', group: 'alert', type: 'string' },
    { name: 'status', group: 'alert', type: 'string' },
    { name: 'summary', group: 'alert', type: 'string' },
    { name: 'description', group: 'alert', type: 'string' },
    { name: 'fingerprint', group: 'alert', type: 'string' },
    { name: 'starts_at', group: 'alert', type: 'string' },
    { name: 'ends_at', group: 'alert', type: 'string' },
    { name: 'generator_url', group: 'alert', type: 'string' },
    { name: 'status_text', group: 'alert', type: 'string', description: '未解决 / 已恢复' },
    { name: 'severity_text', group: 'alert', type: 'string' },
    { name: 'group_key', group: 'group', type: 'string' },
    { name: 'group_labels', group: 'group', type: 'object' },
    { name: 'common_labels', group: 'group', type: 'object' },
    { name: 'common_annotations', group: 'group', type: 'object' },
    { name: 'receiver', group: 'group', type: 'string' },
    { name: 'external_url', group: 'group', type: 'string' },
    { name: 'channel_name', group: 'group', type: 'string' },
    { name: 'alert_count', group: 'group', type: 'number' },
    { name: 'firing_count', group: 'group', type: 'number' },
    { name: 'resolved_count', group: 'group', type: 'number' },
    { name: 'truncated_alerts', group: 'group', type: 'number' },
    { name: 'first_seen_at', group: 'group', type: 'string' },
    { name: 'last_seen_at', group: 'group', type: 'string' },
    { name: 'now', group: 'group', type: 'string' },
    { name: 'alerts_table', group: 'block', type: 'markdown', description: '预渲染告警表格' },
    { name: 'alerts_list', group: 'block', type: 'markdown', description: '预渲染告警列表' },
    { name: 'alerts_summary', group: 'block', type: 'markdown', description: '🔥 3 firing / ✅ 1 resolved' },
  ],
  severity_colors: { critical: 'red', warning: 'orange', info: 'blue', default: 'grey' },
};

export interface MetaContextValue {
  meta: Meta;
  /** 'remote' once `GET /api/v1/meta` succeeded, 'fallback' while unavailable. */
  source: 'remote' | 'fallback';
  loading: boolean;
  error: unknown;
  reload: () => void;
}

const MetaContext = createContext<MetaContextValue>({
  meta: FALLBACK_META,
  source: 'fallback',
  loading: false,
  error: null,
  reload: () => undefined,
});

export function MetaProvider({ children }: { children: ReactNode }): JSX.Element {
  const [meta, setMeta] = useState<Meta>(FALLBACK_META);
  const [source, setSource] = useState<'remote' | 'fallback'>('fallback');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<unknown>(null);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    let active = true;
    setLoading(true);
    getMeta(controller.signal)
      .then((remote) => {
        if (!active) return;
        setMeta(mergeMeta(remote));
        setSource('remote');
        setError(null);
      })
      .catch((cause: unknown) => {
        if (!active) return;
        if (cause instanceof DOMException && cause.name === 'AbortError') return;
        setMeta(FALLBACK_META);
        setSource('fallback');
        setError(cause);
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
      controller.abort();
    };
  }, [tick]);

  const reload = useCallback(() => setTick((value) => value + 1), []);
  const value = useMemo<MetaContextValue>(
    () => ({ meta, source, loading, error, reload }),
    [meta, source, loading, error, reload],
  );

  return <MetaContext.Provider value={value}>{children}</MetaContext.Provider>;
}

/**
 * Defensive coercion into `EnumOption[]`.
 *
 * Accepts the documented `[{value,label}]`, tolerates a bare `string[]` (older/looser
 * servers), and drops anything unusable. This is what keeps a contract drift from
 * turning every `<option key>` into the duplicate string `[object Object]`.
 */
export function normalizeEnumList(raw: unknown): EnumOption[] {
  if (!Array.isArray(raw)) return [];
  const out: EnumOption[] = [];
  for (const item of raw) {
    if (typeof item === 'string') {
      out.push({ value: item, label: item });
      continue;
    }
    if (item && typeof item === 'object') {
      const { value, label } = item as { value?: unknown; label?: unknown };
      if (typeof value === 'string') {
        out.push({ value, label: typeof label === 'string' ? label : value });
      } else if (typeof label === 'string') {
        // 历史上 severity_colors 曾以 [{value:'critical',label:'red'}] 形式返回
        out.push({ value: label, label });
      }
    }
  }
  return out;
}

/** Accepts both `{critical:'red'}` and `[{value:'critical',label:'red'}]`. */
export function normalizeEnumMap(raw: unknown): Record<string, string> {
  if (Array.isArray(raw)) {
    return Object.fromEntries(normalizeEnumList(raw).map((option) => [option.value, option.label]));
  }
  if (raw && typeof raw === 'object') {
    return Object.fromEntries(
      Object.entries(raw as Record<string, unknown>).filter(
        (entry): entry is [string, string] => typeof entry[1] === 'string',
      ),
    );
  }
  return {};
}

/** Fills in missing enum lists from the fallback so selects never render empty. */
function mergeMeta(remote: Meta): Meta {
  const list = (value: unknown, fallback: EnumOption[]): EnumOption[] =>
    nonEmpty(normalizeEnumList(value), fallback);
  return {
    version: remote.version ?? FALLBACK_META.version,
    message_types: list(remote.message_types, FALLBACK_META.message_types),
    repeat_policies: list(remote.repeat_policies, FALLBACK_META.repeat_policies),
    resolved_policies: list(remote.resolved_policies, FALLBACK_META.resolved_policies),
    group_update_policies: list(
      remote.group_update_policies,
      FALLBACK_META.group_update_policies,
    ),
    match_ops: list(remote.match_ops, FALLBACK_META.match_ops),
    status_label_types: list(remote.status_label_types, FALLBACK_META.status_label_types),
    feed_button_types: list(remote.feed_button_types, FALLBACK_META.feed_button_types),
    biz_id_sources: list(remote.biz_id_sources, FALLBACK_META.biz_id_sources),
    card_sources: list(remote.card_sources, FALLBACK_META.card_sources),
    channel_kinds: list(remote.channel_kinds, FALLBACK_META.channel_kinds),
    receive_id_types: list(remote.receive_id_types, FALLBACK_META.receive_id_types),
    variables: remote.variables?.length ? remote.variables : FALLBACK_META.variables,
    severity_colors: nonEmptyMap(
      normalizeEnumMap(remote.severity_colors),
      FALLBACK_META.severity_colors,
    ),
    pin_supported_message_types:
      remote.pin_supported_message_types ?? FALLBACK_META.pin_supported_message_types,
    token_prefix: remote.token_prefix ?? FALLBACK_META.token_prefix,
    hook_path_template: remote.hook_path_template ?? FALLBACK_META.hook_path_template,
  };
}

function nonEmpty(value: EnumOption[], fallback: EnumOption[]): EnumOption[] {
  return value.length > 0 ? value : fallback;
}

function nonEmptyMap(
  value: Record<string, string>,
  fallback: Record<string, string>,
): Record<string, string> {
  return Object.keys(value).length > 0 ? value : fallback;
}

export function useMeta(): MetaContextValue {
  return useContext(MetaContext);
}

/**
 * Enum options for `<select>`; an unexpected value coming from the API is appended
 * so stored data is never silently rewritten.
 */
export function enumOptions(options: EnumOption[], current?: string | null): EnumOption[] {
  if (!current || options.some((option) => option.value === current)) return options;
  return [...options, { value: current, label: `${current}（未知选项）` }];
}

export function labelOf(options: EnumOption[], value: string | null | undefined): string {
  if (!value) return '—';
  return options.find((option) => option.value === value)?.label ?? value;
}
