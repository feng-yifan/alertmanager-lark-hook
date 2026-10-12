/**
 * Enum metadata for form controls, loaded from `GET /api/v1/meta`.
 *
 * The values below are the verbatim fallback taken from `docs/api.md` §0 so the UI
 * still renders usable option lists when the endpoint is unreachable. Remote data
 * always wins when it loads.
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
  status_label_types: ['primary', 'secondary', 'success', 'danger'],
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

/** Fills in missing enum lists from the fallback so selects never render empty. */
function mergeMeta(remote: Meta): Meta {
  return {
    version: remote.version ?? FALLBACK_META.version,
    message_types: nonEmpty(remote.message_types, FALLBACK_META.message_types),
    repeat_policies: nonEmpty(remote.repeat_policies, FALLBACK_META.repeat_policies),
    resolved_policies: nonEmpty(remote.resolved_policies, FALLBACK_META.resolved_policies),
    group_update_policies: nonEmpty(remote.group_update_policies, FALLBACK_META.group_update_policies),
    match_ops: nonEmpty(remote.match_ops, FALLBACK_META.match_ops),
    status_label_types: remote.status_label_types?.length ? remote.status_label_types : FALLBACK_META.status_label_types,
    variables: remote.variables?.length ? remote.variables : FALLBACK_META.variables,
    severity_colors: remote.severity_colors ?? FALLBACK_META.severity_colors,
  };
}

function nonEmpty(value: EnumOption[] | undefined, fallback: EnumOption[]): EnumOption[] {
  return value && value.length > 0 ? value : fallback;
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
