import { useEffect, useMemo, useState, type FormEvent } from 'react';
import {
  ApiError,
  errorDetails,
  errorMessage,
  getChannelPolicy,
  listCardTemplates,
  putChannelPolicy,
} from '../../api';
import {
  Badge,
  Banner,
  Button,
  Card,
  ErrorState,
  Field,
  Input,
  JsonBlock,
  Loading,
  Options,
  RadioGroup,
  Select,
  Switch,
} from '../../components/ui';
import { useAsync } from '../../hooks';
import {
  DECISION_TABLE,
  FILTER_NOTE,
  MATCH_OP_LABEL,
  MUTE_NOTE,
  PIN_NOTE,
  RESOLVED_PIN_NOTE,
} from '../../labels';
import { enumOptions, useMeta } from '../../meta';
import { useToast } from '../../toast';
import type {
  CardTemplate,
  EnumOption,
  GroupUpdatePolicy,
  LabelMatcher,
  MatchOp,
  NotificationPolicy,
  NotificationPolicyInput,
  RepeatPolicy,
  ResolvedPolicy,
} from '../../types';
import { formatDateTime, isoToLocalInput, localInputToIso, shortId } from '../../utils';

interface Props {
  channelId: string;
  channelName: string;
  /** parent reload (hook counts / header) */
  onSaved: () => void;
}

interface Draft {
  card_template_id: string;
  pin_unresolved: boolean;
  unpin_on_resolve: boolean;
  pin_resolved: boolean;
  repeat_policy: RepeatPolicy;
  resolved_policy: ResolvedPolicy;
  group_update_policy: GroupUpdatePolicy;
  label_matchers: LabelMatcher[];
  aggregation_window_secs: string;
  min_send_interval_secs: string;
  max_alerts_per_message: string;
  mute_until: string;
}

/** Defaults mirror the example policy in `docs/api.md` §3. */
const DEFAULT_DRAFT: Draft = {
  card_template_id: '',
  pin_unresolved: false,
  unpin_on_resolve: true,
  pin_resolved: false,
  repeat_policy: 'suppress',
  resolved_policy: 'send',
  group_update_policy: 'update_previous',
  label_matchers: [],
  aggregation_window_secs: '0',
  min_send_interval_secs: '0',
  max_alerts_per_message: '20',
  mute_until: '',
};

interface DraftErrors {
  aggregation?: string;
  minInterval?: string;
  maxAlerts?: string;
  matchers: Record<number, string>;
}

const NO_ERRORS: DraftErrors = { matchers: {} };

function toRadio<T extends string>(options: EnumOption[]): Array<{ value: T; label: string }> {
  return options.map((option) => ({ value: option.value as T, label: option.label }));
}

export default function PolicyTab({ channelId, channelName, onSaved }: Props): JSX.Element {
  const toast = useToast();
  const { meta } = useMeta();

  const { data, loading, error, reload } = useAsync(async (signal) => {
    const [policy, templates] = await Promise.all([
      getChannelPolicy(channelId, signal).catch((cause: unknown) => {
        if (cause instanceof ApiError && cause.status === 404) return null;
        throw cause;
      }),
      listCardTemplates(signal).catch(() => null),
    ]);
    return { policy, templates };
  }, [channelId]);

  const [draft, setDraft] = useState<Draft>(DEFAULT_DRAFT);
  const [errors, setErrors] = useState<DraftErrors>(NO_ERRORS);
  const [saveError, setSaveError] = useState<unknown>(null);
  const [saving, setSaving] = useState(false);

  const policy = data?.policy ?? null;

  useEffect(() => {
    if (!data) return;
    setDraft(data.policy ? draftFromPolicy(data.policy) : DEFAULT_DRAFT);
    setErrors(NO_ERRORS);
    setSaveError(null);
  }, [data]);

  const templates: CardTemplate[] = useMemo(() => data?.templates ?? [], [data]);

  const selectedTemplate = templates.find((template) => template.id === draft.card_template_id);
  const selectedTemplateMissing = draft.card_template_id !== '' && !selectedTemplate;

  const templateOptions = useMemo(() => {
    const options = templates.map((template) => ({
      value: template.id,
      label: `${template.name}${template.is_default ? ' · 默认' : ''}`,
    }));
    if (draft.card_template_id && !options.some((option) => option.value === draft.card_template_id)) {
      const referenced = templates.find((template) => template.id === draft.card_template_id);
      options.push({
        value: draft.card_template_id,
        label: referenced
          ? `${referenced.name}（不在列表中）`
          : `未知模板 ${shortId(draft.card_template_id)}（不在列表中）`,
      });
    }
    return options;
  }, [templates, draft.card_template_id]);

  const validate = (value: Draft): DraftErrors => {
    const result: DraftErrors = { matchers: {} };

    const aggregation = Number(value.aggregation_window_secs);
    if (!Number.isInteger(aggregation) || aggregation < 0 || aggregation > 3600) {
      result.aggregation = '聚合窗口需为 0–3600 的整数秒';
    }
    const minInterval = Number(value.min_send_interval_secs);
    if (!Number.isInteger(minInterval) || minInterval < 0 || minInterval > 3600) {
      result.minInterval = '最小发送间隔需为 0–3600 的整数秒';
    }
    const maxAlerts = Number(value.max_alerts_per_message);
    if (!Number.isInteger(maxAlerts) || maxAlerts < 1 || maxAlerts > 200) {
      result.maxAlerts = '告警上限需为 1–200 的整数';
    }
    value.label_matchers.forEach((matcher, index) => {
      if (!matcher.name.trim()) {
        result.matchers[index] = 'label 名称不能为空';
        return;
      }
      if (!matcher.value.trim() && matcher.value !== '') {
        result.matchers[index] = '匹配值不能为空';
        return;
      }
      if ((matcher.op === '=~' || matcher.op === '!~') && matcher.value !== '') {
        try {
          // eslint-disable-next-line no-new
          new RegExp(matcher.value);
        } catch (cause) {
          result.matchers[index] = `正则无法编译：${cause instanceof Error ? cause.message : String(cause)}`;
        }
      }
    });
    return result;
  };

  const hasErrors = (value: DraftErrors): boolean =>
    Boolean(value.aggregation || value.minInterval || value.maxAlerts) || Object.keys(value.matchers).length > 0;

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    const found = validate(draft);
    setErrors(found);
    setSaveError(null);
    if (hasErrors(found)) {
      toast.error('请先修正表单中的校验错误');
      return;
    }
    const payload: NotificationPolicyInput = {
      card_template_id: draft.card_template_id || null,
      pin_unresolved: draft.pin_unresolved,
      unpin_on_resolve: draft.unpin_on_resolve,
      pin_resolved: draft.pin_resolved,
      repeat_policy: draft.repeat_policy,
      resolved_policy: draft.resolved_policy,
      group_update_policy: draft.group_update_policy,
      label_matchers: draft.label_matchers.map((matcher) => ({
        name: matcher.name.trim(),
        op: matcher.op,
        value: matcher.value,
      })),
      aggregation_window_secs: Number(draft.aggregation_window_secs),
      min_send_interval_secs: Number(draft.min_send_interval_secs),
      max_alerts_per_message: Number(draft.max_alerts_per_message),
      mute_until: draft.mute_until ? localInputToIso(draft.mute_until) : null,
    };
    setSaving(true);
    try {
      await putChannelPolicy(channelId, payload);
      toast.success('通知策略已保存');
      reload();
      onSaved();
    } catch (cause) {
      setSaveError(cause);
      toast.error(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  };

  const updateMatcher = (index: number, patch: Partial<LabelMatcher>) => {
    setDraft((current) => ({
      ...current,
      label_matchers: current.label_matchers.map((matcher, position) =>
        position === index ? { ...matcher, ...patch } : matcher,
      ),
    }));
  };

  if (loading && !data) return <Loading label="正在加载通知策略…" />;
  if (error && !data) return <ErrorState error={error} onRetry={reload} />;

  return (
    <form onSubmit={(event) => void submit(event)}>
      {!policy ? (
        <Banner tone="info">
          渠道「{channelName}」当前没有存储的策略（GET /channels/:id/policy 返回 404）。下面的表单使用契约默认值，
          点击「保存策略」后会创建策略。
        </Banner>
      ) : null}

      {saveError ? (
        <Banner tone="error">
          <div>{errorMessage(saveError)}</div>
          {errorDetails(saveError) ? (
            <div className="mt-8">
              <JsonBlock value={errorDetails(saveError)} maxHeight={220} />
            </div>
          ) : null}
        </Banner>
      ) : null}

      <Card
        title="卡片模板"
        actions={policy ? <span className="faint small">策略更新于 {formatDateTime(policy.updated_at)}</span> : null}
      >
        <Field
          label="卡片模板"
          hint="留空时使用内置默认卡片模板；保存时会校验引用的模板存在"
        >
          <Select
            value={draft.card_template_id}
            onChange={(event) => setDraft((current) => ({ ...current, card_template_id: event.target.value }))}
          >
            <option value="">（无 · 使用内置默认卡片模板）</option>
            {templateOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </Select>
        </Field>

        {selectedTemplateMissing ? (
          <Banner tone="warn">
            策略引用的模板 {shortId(draft.card_template_id)} 不在当前模板列表中（可能已被删除）。
            保存前请重新选择；运行时若模板失效会回退到内置默认卡片。
          </Banner>
        ) : null}
      </Card>

      <Card title="投递策略" subtitle="这三项决定什么情况下发新消息、原地更新旧卡片或撤回旧消息">
        <div className="grid-3">
          <Field label="重复告警" hint="同一分组再次收到重复通知时">
            <RadioGroup<RepeatPolicy>
              value={draft.repeat_policy}
              options={toRadio<RepeatPolicy>(meta.repeat_policies)}
              onChange={(next) => setDraft((current) => ({ ...current, repeat_policy: next }))}
            />
          </Field>
          <Field label="恢复通知" hint="分组内全部告警恢复时">
            <RadioGroup<ResolvedPolicy>
              value={draft.resolved_policy}
              options={toRadio<ResolvedPolicy>(meta.resolved_policies)}
              onChange={(next) => setDraft((current) => ({ ...current, resolved_policy: next }))}
            />
          </Field>
          <Field label="内容变化" hint="分组内新增告警或部分恢复时">
            <RadioGroup<GroupUpdatePolicy>
              value={draft.group_update_policy}
              options={toRadio<GroupUpdatePolicy>(meta.group_update_policies)}
              onChange={(next) => setDraft((current) => ({ ...current, group_update_policy: next }))}
            />
          </Field>
        </div>

        <div className="table-wrap mt-8">
          <table className="data">
            <thead>
              <tr>
                <th>事件</th>
                <th>策略字段</th>
                <th>取值</th>
                <th>动作</th>
              </tr>
            </thead>
            <tbody>
              {DECISION_TABLE.map((row, index) => (
                <tr key={`${row.event}-${row.value}-${index}`}>
                  <td className="nowrap">{row.event}</td>
                  <td className="nowrap">{row.field}</td>
                  <td className="nowrap">{row.valueLabel}</td>
                  <td>{row.action}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Card>

      <Card
        title="未解决告警 Pin"
        subtitle="有未解决告警时把当前卡片 Pin 到会话的 Pin 列表（默认关闭；与飞书的「置顶消息」不是同一个功能）"
      >
        <div className="callout info mb-16">{PIN_NOTE}</div>
        <Switch
          checked={draft.pin_unresolved}
          onChange={(next) => setDraft((current) => ({ ...current, pin_unresolved: next }))}
          label="有未解决告警时 Pin 当前卡片"
          hint="重复通知每次新发一张卡片时都会各自累积一条 Pin。关闭开关后，该分组的下一个事件就会取消所有未解决 Pin。"
        />
        {draft.pin_unresolved ? (
          <Switch
            checked={draft.unpin_on_resolve}
            onChange={(next) => setDraft((current) => ({ ...current, unpin_on_resolve: next }))}
            label="全部恢复后自动取消 Pin"
            hint="关闭后，告警全部恢复时仍保留未解决卡片的 Pin，需要手动取消（便于回看已恢复的告警）。"
          />
        ) : null}
      </Card>

      <Card
        title="已解决告警 Pin"
        subtitle="全部恢复时把恢复卡片也 Pin 上（默认关闭；与「未解决告警 Pin」相互独立，可同时存在多条 Pin）"
      >
        <div className="callout info mb-16">{RESOLVED_PIN_NOTE}</div>
        <Switch
          checked={draft.pin_resolved}
          onChange={(next) => setDraft((current) => ({ ...current, pin_resolved: next }))}
          label="全部恢复时 Pin 恢复卡片"
          hint="无论恢复通知是原地更新、单独发送还是删旧发新，都会 Pin 当前活跃的恢复卡片。"
        />
      </Card>

      <Card title="标签过滤">
        <div className="callout info mb-16">{FILTER_NOTE}</div>
        {draft.label_matchers.length === 0 ? (
          <p className="faint small">未配置匹配条件 —— 全部告警通过。</p>
        ) : (
          draft.label_matchers.map((matcher, index) => (
            <div key={index}>
              <div className="matcher-row">
                <Input
                  value={matcher.name}
                  placeholder="label 名称，如 severity"
                  aria-label="label 名称"
                  onChange={(event) => updateMatcher(index, { name: event.target.value })}
                />
                <Select
                  value={matcher.op}
                  aria-label="匹配操作"
                  onChange={(event) => updateMatcher(index, { op: event.target.value as MatchOp })}
                >
                  <Options options={enumOptions(meta.match_ops, matcher.op)} />
                </Select>
                <Input
                  className="mono"
                  value={matcher.value}
                  placeholder={matcher.op === '=~' || matcher.op === '!~' ? '正则，如 critical|warning' : '匹配值'}
                  aria-label="匹配值"
                  onChange={(event) => updateMatcher(index, { value: event.target.value })}
                />
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    setDraft((current) => ({
                      ...current,
                      label_matchers: current.label_matchers.filter((_, position) => position !== index),
                    }))
                  }
                >
                  删除
                </Button>
              </div>
              {errors.matchers[index] ? <div className="field-error mb-8">{errors.matchers[index]}</div> : null}
            </div>
          ))
        )}
        <div className="row">
          <Button
            size="sm"
            onClick={() =>
              setDraft((current) => ({
                ...current,
                label_matchers: [
                  ...current.label_matchers,
                  { name: 'severity', op: '=~', value: 'critical|warning' } satisfies LabelMatcher,
                ],
              }))
            }
          >
            添加匹配条件
          </Button>
          <span className="faint small">
            操作符：
            {Object.entries(MATCH_OP_LABEL)
              .map(([value, text]) => `${text} ${value}`)
              .join(' · ')}
          </span>
        </div>
      </Card>

      <Card title="聚合、限流与静默">
        <div className="form-grid">
          <Field
            label="聚合窗口（秒）"
            hint="大于 0 时，同一 Hook 与分组的多次事件在窗口内合并为一次决策，取最后一个事件的状态与告警并集；窗口内无新事件时发送。0 = 关闭"
            error={errors.aggregation}
          >
            <Input
              type="number"
              min={0}
              max={3600}
              value={draft.aggregation_window_secs}
              onChange={(event) => setDraft((current) => ({ ...current, aggregation_window_secs: event.target.value }))}
            />
          </Field>
          <Field
            label="最小发送间隔（秒）"
            hint="同一渠道两次发送的最小间隔；未满足时动作延迟到窗口末端合并执行（不丢弃）。0 = 关闭"
            error={errors.minInterval}
          >
            <Input
              type="number"
              min={0}
              max={3600}
              value={draft.min_send_interval_secs}
              onChange={(event) => setDraft((current) => ({ ...current, min_send_interval_secs: event.target.value }))}
            />
          </Field>
          <Field label="单条消息告警上限" hint="超出的告警折叠为「+N more」；AlertManager 侧已截断时提示总数为下界" error={errors.maxAlerts}>
            <Input
              type="number"
              min={1}
              max={200}
              value={draft.max_alerts_per_message}
              onChange={(event) => setDraft((current) => ({ ...current, max_alerts_per_message: event.target.value }))}
            />
          </Field>
          <Field label="静默至" hint={MUTE_NOTE}>
            <div className="row tight">
              <Input
                type="datetime-local"
                value={draft.mute_until}
                onChange={(event) => setDraft((current) => ({ ...current, mute_until: event.target.value }))}
                style={{ width: 220 }}
              />
              <Button size="sm" onClick={() => setDraft((current) => ({ ...current, mute_until: '' }))}>
                清除静默
              </Button>
            </div>
            <div className="hint">
              {draft.mute_until
                ? `静默至本地时间 ${draft.mute_until.replace('T', ' ')}（按 UTC 提交）`
                : '当前无静默'}
            </div>
          </Field>
        </div>
      </Card>

      <div className="row" style={{ justifyContent: 'flex-end' }}>
        {draft.card_template_id && selectedTemplate ? (
          <Badge tone="muted">
            已选模板：{selectedTemplate.name}
          </Badge>
        ) : null}
        <Button onClick={() => setDraft(policy ? draftFromPolicy(policy) : DEFAULT_DRAFT)} disabled={saving}>
          重置
        </Button>
        <Button variant="primary" type="submit" loading={saving}>
          保存策略
        </Button>
      </div>
    </form>
  );
}

function draftFromPolicy(policy: NotificationPolicy): Draft {
  return {
    card_template_id: policy.card_template_id ?? '',
    pin_unresolved: policy.pin_unresolved,
    unpin_on_resolve: policy.unpin_on_resolve,
    pin_resolved: policy.pin_resolved,
    repeat_policy: policy.repeat_policy,
    resolved_policy: policy.resolved_policy,
    group_update_policy: policy.group_update_policy,
    label_matchers: policy.label_matchers.map((matcher) => ({ ...matcher })),
    aggregation_window_secs: String(policy.aggregation_window_secs),
    min_send_interval_secs: String(policy.min_send_interval_secs),
    max_alerts_per_message: String(policy.max_alerts_per_message),
    mute_until: isoToLocalInput(policy.mute_until),
  };
}
