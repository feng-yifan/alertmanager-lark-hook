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
  card_sources: [
    { value: 'builder_json', label: '卡片搭建工具 JSON（JSON 2.0）' },
    { value: 'builder_template', label: '引用搭建工具模板（已停用）' },
  ],
  channel_kinds: [
    { value: 'group', label: '群聊' },
    { value: 'p2p_user', label: '单聊用户' },
  ],
  receive_id_types: [
    { value: 'chat_id', label: '群 ID（chat_id）' },
    { value: 'open_id', label: '用户 ID（open_id）' },
    { value: 'user_id', label: '用户 ID（user_id）' },
    { value: 'email', label: '用户邮箱' },
  ],
  token_prefix: 'alkh_',
  hook_path_template: '/api/v1/hooks/alertmanager/{token}',
  variables: [
    // 分组级
    { name: 'group_key', group: 'group', type: 'string', description: 'AlertManager 分组键' },
    { name: 'receiver', group: 'group', type: 'string', description: 'AlertManager receiver 名' },
    { name: 'external_url', group: 'group', type: 'url', description: 'AlertManager 地址' },
    { name: 'channel_name', group: 'group', type: 'string', description: '消息渠道名称' },
    { name: 'channel_id', group: 'group', type: 'string', description: '渠道接收者 ID' },
    { name: 'alert_count', group: 'group', type: 'number', description: '本次告警条数' },
    { name: 'firing_count', group: 'group', type: 'number', description: '未解决告警条数' },
    { name: 'resolved_count', group: 'group', type: 'number', description: '已恢复告警条数' },
    { name: 'truncated_alerts', group: 'group', type: 'number', description: '被 AlertManager 截断的条数' },
    { name: 'first_seen_at', group: 'group', type: 'datetime', description: '分组首次出现时间' },
    { name: 'last_seen_at', group: 'group', type: 'datetime', description: '分组最后更新时间' },
    { name: 'resolved_at', group: 'group', type: 'datetime', description: '恢复时间（未恢复为空）' },
    { name: 'now', group: 'group', type: 'datetime', description: '当前时间' },
    { name: 'status', group: 'group', type: 'string', description: 'firing / resolved' },
    { name: 'status_text', group: 'group', type: 'string', description: '未解决 / 已恢复' },
    { name: 'status_color', group: 'group', type: 'string', description: '状态对应卡片颜色（未恢复按严重级别，已恢复 green）' },
    { name: 'is_firing', group: 'group', type: 'bool', description: '是否仍存在未解决告警' },
    { name: 'severity', group: 'group', type: 'string', description: '严重级别原始值' },
    { name: 'severity_text', group: 'group', type: 'string', description: '严重级别中文' },
    { name: 'severity_color', group: 'group', type: 'string', description: '严重级别对应卡片颜色（不随恢复变化）' },
    { name: 'group_labels', group: 'group', type: 'json', description: 'groupLabels JSON' },
    { name: 'common_labels', group: 'group', type: 'json', description: 'commonLabels JSON' },
    { name: 'common_annotations', group: 'group', type: 'json', description: 'commonAnnotations JSON' },
    // 代表告警
    { name: 'alertname', group: 'alert', type: 'string', description: '告警名称' },
    { name: 'instance', group: 'alert', type: 'string', description: '实例' },
    { name: 'job', group: 'alert', type: 'string', description: 'job' },
    { name: 'fingerprint', group: 'alert', type: 'string', description: '告警指纹' },
    { name: 'summary', group: 'alert', type: 'string', description: '概要 annotation' },
    { name: 'description', group: 'alert', type: 'string', description: '详情 annotation' },
    { name: 'alert_status', group: 'alert', type: 'string', description: '代表告警状态' },
    { name: 'starts_at', group: 'alert', type: 'datetime', description: '代表告警开始时间' },
    { name: 'ends_at', group: 'alert', type: 'datetime', description: '代表告警结束时间' },
    { name: 'generator_url', group: 'alert', type: 'url', description: '告警来源链接' },
    // 预渲染块
    { name: 'alerts_summary', group: 'block', type: 'markdown', description: '一行汇总，如 🔥 3 条未解决 · ✅ 1 条已恢复' },
    { name: 'alerts_list', group: 'block', type: 'markdown', description: '告警列表（推荐）' },
    { name: 'alerts_table', group: 'block', type: 'markdown', description: '告警 Markdown 表格' },
    // 动态
    { name: 'label:<名称>', group: 'dynamic', type: 'string', description: '任意标签，如 ${label:team}' },
    { name: 'annotation:<名称>', group: 'dynamic', type: 'string', description: '任意注解，如 ${annotation:runbook}' },
    { name: '<名称>', group: 'dynamic', type: 'string', description: '标签/注解可直接用原名引用，如 ${team}' },
  ],
  severity_colors: { critical: 'red', warning: 'orange', info: 'blue', default: 'grey' },
  // 组件目录只来自服务端（`GET /meta`）；离线兜底不提供目录，
  // 画布会提示「目录为空，请升级后端后刷新」。
  card_components: [],
  card_presets: [],
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
    repeat_policies: list(remote.repeat_policies, FALLBACK_META.repeat_policies),
    resolved_policies: list(remote.resolved_policies, FALLBACK_META.resolved_policies),
    group_update_policies: list(
      remote.group_update_policies,
      FALLBACK_META.group_update_policies,
    ),
    match_ops: list(remote.match_ops, FALLBACK_META.match_ops),
    card_sources: list(remote.card_sources, FALLBACK_META.card_sources),
    channel_kinds: list(remote.channel_kinds, FALLBACK_META.channel_kinds),
    receive_id_types: list(remote.receive_id_types, FALLBACK_META.receive_id_types),
    variables: remote.variables?.length ? remote.variables : FALLBACK_META.variables,
    severity_colors: nonEmptyMap(
      normalizeEnumMap(remote.severity_colors),
      FALLBACK_META.severity_colors,
    ),
    // 组件目录没有本地兜底：目录缺失时画布会显示明确提示，
    // 而不是用一份会漂移的本地副本悄悄产出过时的 JSON。
    card_components: Array.isArray(remote.card_components) ? remote.card_components : [],
    card_presets: Array.isArray(remote.card_presets) ? remote.card_presets : [],
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
