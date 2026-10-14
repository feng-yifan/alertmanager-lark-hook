/**
 * 模拟数据面板（P1）。
 *
 * 三层语义：
 *  1. 基线 —— 内置样例（firing / resolved）或一条**历史告警分组**；
 *  2. 覆盖 —— 逐项覆盖基线，支持 `label:` / `annotation:` 动态 key；
 *  3. 额外变量 —— 基线里没有的 key 也能加，用于试写新标签。
 *
 * 基线每次重建（选择变化时重新拉取），覆盖项由父组件持有并透传给预览 / 测试发送。
 */
import { useMemo, useState } from 'react';
import { getCardTemplateMock } from '../api';
import { useAsync } from '../hooks';
import { selectableChannels } from '../channels';
import type { AlertGroup, Channel, Meta, MetaVariable, MockBaseline, MockSpec } from '../types';
import { GROUP_STATUS_LABEL, label } from '../labels';
import { Badge, Banner, Button, Card, Field, Input, Select } from './ui';

export interface MockPanelProps {
  meta: Meta;
  channels: Channel[];
  /** 最近的历史告警分组，供「用真实告警试一下」。 */
  alertGroups: AlertGroup[];
  /** 基线渠道（影响 `channel_name` / `channel_id`）；空串表示用内置样例渠道。 */
  channelId: string;
  onChannelIdChange: (value: string) => void;
  sample: 'firing' | 'resolved';
  onSampleChange: (value: 'firing' | 'resolved') => void;
  mock: MockSpec;
  onMockChange: (mock: MockSpec) => void;
}

/** 变量分类的展示顺序与中文标题。 */
const SECTIONS: Array<{ group: string; title: string }> = [
  { group: 'group', title: '分组级（AlertManager 契约）' },
  { group: 'alert', title: '代表告警' },
  { group: 'block', title: '预渲染块（服务端已拍平，推荐直接用）' },
];

export function MockPanel({
  meta,
  channels,
  alertGroups,
  channelId,
  onChannelIdChange,
  sample,
  onSampleChange,
  mock,
  onMockChange,
}: MockPanelProps): JSX.Element {
  const alertGroupId = mock.alert_group_id ?? '';
  const overrides = mock.variables ?? {};

  const baseline = useAsync<MockBaseline>(
    (signal) =>
      getCardTemplateMock(
        {
          ...(channelId ? { channel_id: channelId } : {}),
          sample,
          ...(alertGroupId ? { alert_group_id: alertGroupId } : {}),
        },
        signal,
      ),
    [channelId, sample, alertGroupId],
  );

  const [customKey, setCustomKey] = useState('');
  const [customValue, setCustomValue] = useState('');

  const baselineVars = baseline.data?.variables ?? {};
  const metaByName = useMemo(() => {
    const map = new Map<string, MetaVariable>();
    for (const variable of meta.variables) map.set(variable.name, variable);
    return map;
  }, [meta.variables]);

  const patch = (next: Partial<MockSpec>) => onMockChange({ ...mock, ...next });

  const setOverride = (key: string, value: string) => {
    patch({ variables: { ...overrides, [key]: value } });
  };

  const clearOverride = (key: string) => {
    const next = { ...overrides };
    delete next[key];
    patch({ variables: next });
  };

  const addCustom = () => {
    const key = customKey.trim();
    if (!key) return;
    setOverride(key, customValue);
    setCustomKey('');
    setCustomValue('');
  };

  const overrideCount = Object.keys(overrides).length;
  const knownNames = useMemo(
    () => meta.variables.map((variable) => variable.name).filter((name) => !name.includes('<')),
    [meta.variables],
  );
  const knownSet = useMemo(() => new Set(knownNames), [knownNames]);
  /** 基线里出现、但不在内置清单里的变量（如 `label:team`、`team`）。 */
  const extraBaselineKeys = useMemo(
    () => Object.keys(baselineVars).filter((key) => !knownSet.has(key)).sort(),
    [baselineVars, knownSet],
  );
  /** 用户自己加进去的 key。 */
  const customKeys = useMemo(
    () =>
      Object.keys(overrides)
        .filter((key) => !knownSet.has(key) && !(key in baselineVars))
        .sort(),
    [overrides, knownSet, baselineVars],
  );

  const renderRow = (name: string) => (
    <tr key={name}>
      <td className="mono" style={{ whiteSpace: 'nowrap' }} title={metaByName.get(name)?.description}>
        {name}
        {name in overrides ? (
          <>
            {' '}
            <Badge tone="accent">已覆盖</Badge>
          </>
        ) : null}
      </td>
      <td>
        <Input
          value={overrides[name] ?? ''}
          placeholder={
            baselineVars[name] === undefined
              ? '（基线中不存在）'
              : baselineVars[name] === ''
                ? '（空）'
                : baselineVars[name]
          }
          onChange={(event) => setOverride(name, event.target.value)}
          aria-label={`${name} 的模拟值`}
        />
      </td>
      <td className="actions">
        {name in overrides ? (
          <Button size="sm" variant="ghost" onClick={() => clearOverride(name)}>
            清除
          </Button>
        ) : null}
      </td>
    </tr>
  );

  const sectionVariables = (group: string) =>
    meta.variables.filter((variable) => variable.group === group && !variable.name.includes('<'));

  const mockFromHistory = Boolean(mock.alert_group_id);

  return (
    <Card
      className="mt-16"
      title="模拟数据"
      subtitle={
        <>
          基线数据来自服务端内置样例；覆盖项会写进渲染上下文，未知变量名也会生效。
          历史分组只作为数据源，不会被改写。
        </>
      }
      actions={
        <>
          <Badge tone={overrideCount > 0 ? 'accent' : 'muted'}>
            {overrideCount > 0 ? `${overrideCount} 项覆盖` : '未覆盖'}
          </Badge>
          <Button size="sm" onClick={() => baseline.reload()} loading={baseline.refreshing}>
            刷新基线
          </Button>
          <Button
            size="sm"
            variant="ghost"
            disabled={overrideCount === 0}
            onClick={() => patch({ variables: {} })}
          >
            重置覆盖
          </Button>
        </>
      }
    >

      <div className="grid-3 mt-16">
        <Field label="基线样例" hint="未选择历史分组时生效">
          <Select
            value={sample}
            onChange={(event) => onSampleChange(event.target.value as 'firing' | 'resolved')}
          >
            <option value="firing">未解决</option>
            <option value="resolved">已恢复</option>
          </Select>
        </Field>
        <Field label="历史告警分组" hint="用一条真实告警作为数据源（优先于样例）">
          <Select
            value={alertGroupId}
            onChange={(event) =>
              patch({ alert_group_id: event.target.value || null, variables: {} })
            }
          >
            <option value="">（不使用 · 用内置样例）</option>
            {alertGroups.map((group) => (
              <option key={group.id} value={group.id}>
                {group.alertname ?? group.group_key} · {label(GROUP_STATUS_LABEL, group.status)}
              </option>
            ))}
          </Select>
        </Field>
        <Field label="基线渠道" hint="影响消息里的渠道名与渠道 ID；只列可用渠道，留空用内置样例渠道">
          <Select value={channelId} onChange={(event) => onChannelIdChange(event.target.value)}>
            <option value="">（内置样例渠道）</option>
            {selectableChannels(channels).map((channel) => (
              <option key={channel.id} value={channel.id}>
                {channel.name}
              </option>
            ))}
          </Select>
        </Field>
      </div>

      {baseline.error ? (
        <Banner tone="error">
          基线加载失败：{baseline.error instanceof Error ? baseline.error.message : String(baseline.error)}
        </Banner>
      ) : null}

      {mockFromHistory && baseline.data?.alert_group_label ? (
        <Banner tone="info">
          数据源：历史分组 <span className="mono">{baseline.data.alert_group_label}</span>
          {baseline.data.sample === 'resolved' ? '（已恢复）' : '（未解决）'}
        </Banner>
      ) : null}

      {baseline.loading && !baseline.data ? (
        <p className="faint small">正在加载模拟数据…</p>
      ) : null}

      {baseline.data ? (
        <>
          {SECTIONS.map((section) => {
            const variables = sectionVariables(section.group);
            if (variables.length === 0) return null;
            return (
              <details key={section.group} open={section.group !== 'block'} className="mt-16">
                <summary className="small muted">{section.title}（{variables.length}）</summary>
                <div className="table-wrap">
                  <table className="data">
                    <tbody>{variables.map((variable) => renderRow(variable.name))}</tbody>
                  </table>
                </div>
              </details>
            );
          })}

          <details className="mt-16">
            <summary className="small muted">
              其他变量（动态 label / annotation 等，{extraBaselineKeys.length + customKeys.length}）
            </summary>
            <div className="table-wrap">
              <table className="data">
                <tbody>
                  {[...customKeys, ...extraBaselineKeys].map((key) => renderRow(key))}
                </tbody>
              </table>
            </div>
            <div className="row tight mt-8">
              <Input
                placeholder="变量名，如 label:team"
                className="mono"
                value={customKey}
                onChange={(event) => setCustomKey(event.target.value)}
                style={{ width: 220 }}
                aria-label="自定义变量名"
              />
              <Input
                placeholder="值"
                value={customValue}
                onChange={(event) => setCustomValue(event.target.value)}
                style={{ width: 220 }}
                aria-label="自定义变量值"
              />
              <Button size="sm" onClick={addCustom} disabled={!customKey.trim()}>
                添加变量
              </Button>
            </div>
          </details>
        </>
      ) : null}
    </Card>
  );
}
