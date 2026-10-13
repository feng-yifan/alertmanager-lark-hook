import { useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import {
  createCardTemplate,
  errorDetails,
  errorMessage,
  getCardTemplate,
  getCardTemplateDefaults,
  listAlertGroups,
  listChannels,
  patchCardTemplate,
  previewCardTemplate,
  sendTestCardTemplate,
  sendTestCardTemplateDraft,
  validateCardTemplate,
} from '../api';
import {
  Badge,
  Banner,
  Button,
  Card,
  CopyButton,
  DescriptionList,
  Field,
  Input,
  JsonBlock,
  Loading,
  Options,
  RadioGroup,
  Select,
  StatusBadge,
  Switch,
  Textarea,
} from '../components/ui';
import ErrorBoundary from '../components/ErrorBoundary';
import { MockPanel } from '../components/MockPanel';
import { CardBuilder } from '../card-builder/Builder';
import { useAsync } from '../hooks';
import { enumOptions, useMeta } from '../meta';
import { useToast } from '../toast';
import type {
  AlertGroup,
  AppFeedCardSpec,
  BizIdSource,
  CardJson,
  Delivery,
  FeedButtonType,
  CardSource,
  CardTemplate,
  CardTemplateCreate,
  CardMessageInput,
  CardTemplateKind,
  EnumOption,
  MockSpec,
  PreviewResponse,
  StatusLabelType,
  ValidateResponse,
} from '../types';
import { cx, formatBytes, isPlainObject, prettyJson, validationText } from '../utils';

/* ---------------------------------------------------------------- helpers */

const DEFAULT_SPEC: AppFeedCardSpec = {
  title: '${alertname} ${status_text}',
  preview: '${summary}',
  status_label: { text: '${severity_text}', type_firing: 'danger', type_resolved: 'success' },
  link: { enabled: true, url: '${external_url}' },
  buttons: [{ text: '查看详情', url: '${external_url}', button_type: 'primary' }],
  close_notify: false,
  biz_id_source: 'alert_group',
};

function normalizeSpec(input: AppFeedCardSpec | null | undefined): AppFeedCardSpec {
  if (!input) return { ...DEFAULT_SPEC, buttons: DEFAULT_SPEC.buttons.map((button) => ({ ...button })) };
  return {
    title: input.title ?? '',
    preview: input.preview ?? '',
    status_label: {
      text: input.status_label?.text ?? '',
      type_firing: input.status_label?.type_firing ?? 'danger',
      type_resolved: input.status_label?.type_resolved ?? 'success',
    },
    link: { enabled: input.link?.enabled ?? true, url: input.link?.url ?? '' },
    buttons: (input.buttons ?? []).slice(0, 2).map((button) => ({
      text: button.text ?? '',
      url: button.url ?? '',
      button_type: button.button_type ?? 'primary',
    })),
    close_notify: input.close_notify ?? false,
    biz_id_source: input.biz_id_source ?? 'alert_group',
  };
}

/** Parses the paste area, tolerating ```json fences and wrapper objects. */
function extractCardJson(text: string): CardJson {
  let trimmed = text.trim();
  const fence = /^```(?:json)?\s*([\s\S]*?)\s*```$/i.exec(trimmed);
  if (fence && fence[1]) trimmed = fence[1].trim();
  const parsed: unknown = JSON.parse(trimmed);
  if (isPlainObject(parsed)) {
    const inner = parsed.card_json ?? parsed.cardJson ?? parsed.card;
    if (inner !== undefined) return inner as CardJson;
  }
  if (!isPlainObject(parsed)) throw new Error('卡片 JSON 必须是对象（CardKit JSON 2.0）');
  return parsed;
}

function charLength(value: string): number {
  return Array.from(value).length;
}

/** 校验响应即使缺少某个数组字段也不能让页面抛异常。 */
function normalizeValidation(report: ValidateResponse): ValidateResponse {
  return {
    valid: report.valid ?? false,
    errors: Array.isArray(report.errors) ? report.errors : [],
    warnings: Array.isArray(report.warnings) ? report.warnings : [],
    detected_variables: Array.isArray(report.detected_variables) ? report.detected_variables : [],
    unknown_variables: Array.isArray(report.unknown_variables) ? report.unknown_variables : [],
  };
}

/**
 * `GET /api/v1/meta` 的枚举列表已经是 `{value,label}` 对象数组（后端自带中文标签），
 * 这里只做「当前值不在列表中时补一个未知选项」的处理。
 *
 * 注意：早先的版本把 `status_label_types` 当成字符串数组，导致 `value` 变成对象、
 * `key` 被渲染成重复的 `[object Object]`。回归测试见
 * `alkh_app::meta::tests::meta_wire_format_is_enum_option_objects`。
 */
function enumOptionsOf(
  types: EnumOption[],
  current: string | null | undefined,
): EnumOption[] {
  return enumOptions(types, current ?? undefined);
}

function LengthHint({ value, max, label }: { value: string; max: number; label?: string }): JSX.Element {
  const length = charLength(value);
  const over = length > max;
  return (
    <>
      <div className={cx('length-hint', over && 'over')}>
        {label ? `${label} ` : ''}
        {length}/{max}
        {over ? ` · 超出 ${length - max} 字符` : ''}
      </div>
      <div className={cx('meter', over && 'over')}>
        <span style={{ width: `${Math.min(100, (length / max) * 100)}%` }} />
      </div>
    </>
  );
}

type FocusKey = 'title' | 'preview' | 'status_text' | `button:${number}:text`;

interface SpecIssue {
  field: string;
  message: string;
}

/** Client-side mirror of business-rules §6 for app feed cards. */
function validateSpec(spec: AppFeedCardSpec): SpecIssue[] {
  const issues: SpecIssue[] = [];
  if (!spec.title.trim()) issues.push({ field: 'title', message: 'title 必填' });
  if (charLength(spec.title) > 60) issues.push({ field: 'title', message: 'title 不能超过 60 字符' });
  if (charLength(spec.preview) > 120) issues.push({ field: 'preview', message: 'preview 不能超过 120 字符' });
  if (charLength(spec.status_label.text) > 20) {
    issues.push({ field: 'status_label.text', message: 'status_label.text 不能超过 20 字符' });
  }
  if (!spec.link.enabled) {
    issues.push({ field: 'link.enabled', message: '应用消息流卡片创建时 link 必填（domain-model §4），请启用 link' });
  }
  const url = spec.link.url.trim();
  if (!url) {
    issues.push({ field: 'link.url', message: 'link.url 渲染后必须非空' });
  } else if (!/^https:\/\//i.test(url) && !url.includes('${')) {
    issues.push({ field: 'link.url', message: 'link.url 必须为 https（business-rules §6）' });
  }
  if (spec.buttons.length > 2) issues.push({ field: 'buttons', message: '按钮最多 2 个' });
  spec.buttons.forEach((button, index) => {
    if (!button.text.trim()) issues.push({ field: `buttons[${index}].text`, message: '按钮文案必填' });
    if (charLength(button.text) > 30) issues.push({ field: `buttons[${index}].text`, message: '按钮文案不能超过 30 字符' });
    const buttonUrl = button.url.trim();
    if (!buttonUrl) issues.push({ field: `buttons[${index}].url`, message: '按钮 url 必填' });
    else if (!/^https:\/\//i.test(buttonUrl) && !buttonUrl.includes('${')) {
      issues.push({ field: `buttons[${index}].url`, message: '按钮 url 必须为 https' });
    }
  });
  return issues;
}

/* ------------------------------------------------------------------- page */

/**
 * 页面外壳：任何渲染期异常都由 ErrorBoundary 兜底，避免整页白屏。
 */
export default function CardTemplateEditorPage(): JSX.Element {
  return (
    <ErrorBoundary>
      <CardTemplateForm />
    </ErrorBoundary>
  );
}

function CardTemplateForm(): JSX.Element {
  const { id } = useParams<{ id: string }>();
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const toast = useToast();
  const { meta } = useMeta();

  const isEdit = Boolean(id);
  const kindFromQuery = searchParams.get('kind');

  const { data: loaded, loading: loadingTemplate, error: loadError, reload } = useAsync<CardTemplate | null>(
    (signal) => (id ? getCardTemplate(id, signal) : Promise.resolve(null)),
    [id],
  );

  const { data: channelPage } = useAsync((signal) => listChannels({ limit: 200 }, signal), []);
  const channels = channelPage?.items ?? [];
  // 内置默认卡片：新建模板的起步内容 + 旧来源（builder_template）的转换落点。
  // 单一事实来源在后端 defaults.rs，前端不再抄一份。
  const { data: defaults } = useAsync((signal) => getCardTemplateDefaults(signal), []);
  // mock 面板的「历史告警分组」下拉：取最近 50 条，firing 优先。
  const { data: alertGroupPage } = useAsync(
    (signal) => listAlertGroups({ limit: 50 }, signal),
    [],
  );
  const alertGroups: AlertGroup[] = alertGroupPage?.items ?? [];

  const [kind, setKind] = useState<CardTemplateKind>(
    kindFromQuery === 'app_feed_card' ? 'app_feed_card' : 'card_message',
  );
  const [name, setName] = useState('');
  const [isDefault, setIsDefault] = useState(false);
  const [source, setSource] = useState<CardSource>('builder_json');
  /** 卡片内容的编辑方式：可视化画布 / 原始 JSON（编辑同一份文档）。 */
  const [view, setView] = useState<'visual' | 'json'>('visual');
  const [cardText, setCardText] = useState('');
  const [cardJson, setCardJson] = useState<CardJson | null>(null);
  const [cardTextError, setCardTextError] = useState<string | null>(null);
  const [importText, setImportText] = useState('');
  const [importError, setImportError] = useState<string | null>(null);
  const [templateId, setTemplateId] = useState('');
  const [templateVersion, setTemplateVersion] = useState('');
  const [spec, setSpec] = useState<AppFeedCardSpec>(() => normalizeSpec(null));
  const [specIssues, setSpecIssues] = useState<SpecIssue[]>([]);
  const [focusKey, setFocusKey] = useState<FocusKey>('title');
  const [declaredVariables, setDeclaredVariables] = useState<Array<{ name: string; ty: string }>>([]);

  const [sample, setSample] = useState<'firing' | 'resolved'>('firing');
  const [previewChannelId, setPreviewChannelId] = useState('');
  /** mock 覆盖项：预览与测试发送共用（P1）。 */
  const [mock, setMock] = useState<MockSpec>({});
  const [validation, setValidation] = useState<ValidateResponse | null>(null);
  const [preview, setPreview] = useState<PreviewResponse | null>(null);
  const [validating, setValidating] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<unknown>(null);
  const [testChannelId, setTestChannelId] = useState('');
  const [sendingTest, setSendingTest] = useState(false);
  const [testResult, setTestResult] = useState<Delivery | null>(null);

  // hydrate the form from the loaded template
  useEffect(() => {
    if (!loaded) return;
    setKind(loaded.kind);
    setName(loaded.name);
    setIsDefault(loaded.is_default);
    setValidation(null);
    setPreview(null);
    if (loaded.kind === 'card_message') {
      // 真实线格式把卡片内容嵌在 `card_message` 下；字段缺失时全部按空值处理。
      const cardSpec = loaded.card_message ?? null;
      setSource(cardSpec?.source ?? 'builder_json');
      setCardJson(cardSpec?.card_json ?? null);
      setCardText(cardSpec?.card_json ? prettyJson(cardSpec.card_json) : '');
      setCardTextError(null);
      setTemplateId(cardSpec?.template_id ?? '');
      setTemplateVersion(cardSpec?.template_version_name ?? '');
      const variables = Array.isArray(cardSpec?.variables) ? cardSpec.variables : [];
      setDeclaredVariables(variables.map((variable) => ({ name: variable.name, ty: variable.ty })));
    } else {
      setSpec(normalizeSpec(loaded.app_feed_spec));
      setDeclaredVariables([]);
    }
  }, [loaded]);

  const cardTemplateVariables = useMemo(
    () =>
      loaded && loaded.kind === 'card_message' && Array.isArray(loaded.card_message?.variables)
        ? loaded.card_message.variables
        : [],
    [loaded],
  );

  const parseCardText = (text: string): CardJson | null => {
    try {
      const parsed: unknown = JSON.parse(text);
      if (!isPlainObject(parsed)) throw new Error('卡片 JSON 必须是对象（CardKit JSON 2.0）');
      setCardTextError(null);
      setCardJson(parsed);
      return parsed;
    } catch (cause) {
      setCardTextError(cause instanceof Error ? cause.message : String(cause));
      setCardJson(null);
      return null;
    }
  };

  const importCard = () => {
    setImportError(null);
    try {
      const parsed = extractCardJson(importText);
      setCardJson(parsed);
      setCardText(prettyJson(parsed));
      setCardTextError(null);
      toast.success(`已导入卡片 JSON（${formatBytes(JSON.stringify(parsed).length)}）`);
    } catch (cause) {
      setImportError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  /** 画布的回写：可视化与 JSON 两个视图共享同一份文档状态。 */
  const applyCardJson = (next: CardJson) => {
    setCardJson(next);
    setCardText(prettyJson(next));
    setCardTextError(null);
  };

  /** 载入内置的起步卡片（新模板与转换旧来源共用）。 */
  const loadStarterCard = () => {
    const starter = defaults?.card_message.card_json;
    if (!starter) {
      toast.error('默认卡片尚未加载完成');
      return;
    }
    applyCardJson(starter);
    setView('visual');
  };

  /** P2：把已停用的 builder_template 来源转换为本地卡片 JSON。 */
  const convertLegacyCard = () => {
    setSource('builder_json');
    setTemplateId('');
    setTemplateVersion('');
    loadStarterCard();
    toast.success('已切换为本地卡片 JSON，请检查内容后保存');
  };

  // 新建模板：直接用内置默认卡片起步，不再让用户面对空白 JSON
  useEffect(() => {
    if (isEdit || kind !== 'card_message' || !defaults || cardJson) return;
    const starter = defaults.card_message.card_json;
    if (!starter) return;
    setCardJson(starter);
    setCardText(prettyJson(starter));
  }, [isEdit, kind, defaults, cardJson]);

  const buildCardMessageInput = (): CardMessageInput => ({
    source,
    card_json: source === 'builder_json' ? cardJson : null,
    template_id: source === 'builder_template' ? templateId.trim() || null : null,
    template_version_name: source === 'builder_template' ? templateVersion.trim() || null : null,
    ...(cardTemplateVariables.length > 0 ? { variables: cardTemplateVariables } : {}),
  });

  /**
   * validate / preview / create / patch 共用同一请求体形状：
   * `name` 必填，卡片内容必须嵌在 `card_message` 里（顶层扁平字段会被后端忽略并回退默认卡片）。
   */
  const buildTemplateInput = (): CardTemplateCreate =>
    kind === 'card_message'
      ? { name: name.trim(), kind: 'card_message', card_message: buildCardMessageInput(), is_default: isDefault }
      : { name: name.trim(), kind: 'app_feed_card', app_feed_spec: spec, is_default: isDefault };

  const ensureRunnable = (): boolean => {
    if (!name.trim()) {
      toast.error('模板名称（name）为必填');
      return false;
    }
    if (kind === 'card_message') {
      if (source === 'builder_json' && !cardJson) {
        toast.error('builder_json 来源必须提供合法的卡片 JSON');
        return false;
      }
      if (source === 'builder_template' && !templateId.trim()) {
        toast.error('builder_template 来源必须填写 template_id');
        return false;
      }
    }
    return true;
  };

  const runValidate = async () => {
    if (!ensureRunnable()) return;
    setValidating(true);
    setPreview(null);
    try {
      const result = normalizeValidation(await validateCardTemplate(buildTemplateInput()));
      setValidation(result);
      toast.success(result.valid ? '校验通过' : `校验未通过（${result.errors.length} 个错误）`);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setValidating(false);
    }
  };

  /** 去掉空覆盖，避免把无意义的 `{variables:{}}` 传给后端。 */
  const effectiveMock = (): MockSpec | undefined => {
    const variables = Object.fromEntries(
      Object.entries(mock.variables ?? {}).filter(([key]) => key.trim().length > 0),
    );
    const hasVariables = Object.keys(variables).length > 0;
    const alertGroupId = mock.alert_group_id ?? null;
    if (!hasVariables && !alertGroupId) return undefined;
    return {
      ...(alertGroupId ? { alert_group_id: alertGroupId } : {}),
      ...(hasVariables ? { variables } : {}),
    };
  };

  const runPreview = async () => {
    if (!ensureRunnable()) return;
    setPreviewing(true);
    try {
      const result = await previewCardTemplate({
        ...buildTemplateInput(),
        sample,
        ...(previewChannelId ? { channel_id: previewChannelId } : {}),
        ...(effectiveMock() ? { mock: effectiveMock() } : {}),
      });
      setPreview(result);
      toast.success('预览已生成');
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setPreviewing(false);
    }
  };

  const sendDraftTest = async () => {
    if (!testChannelId) {
      toast.error('请选择测试渠道');
      return;
    }
    if (!ensureRunnable()) return;
    setSendingTest(true);
    setTestResult(null);
    try {
      const result = await sendTestCardTemplateDraft({
        ...buildTemplateInput(),
        channel_id: testChannelId,
        sample,
        ...(effectiveMock() ? { mock: effectiveMock() } : {}),
      });
      setTestResult(result);
      toast.success('草稿测试消息已提交发送');
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setSendingTest(false);
    }
  };

  const insertVariable = (variable: string) => {
    const token = `\${${variable}}`;
    if (kind !== 'app_feed_card') {
      setCardText((current) => `${current}${token}`);
      return;
    }
    if (focusKey === 'title') setSpec((current) => ({ ...current, title: `${current.title}${token}` }));
    else if (focusKey === 'preview') setSpec((current) => ({ ...current, preview: `${current.preview}${token}` }));
    else if (focusKey === 'status_text') {
      setSpec((current) => ({ ...current, status_label: { ...current.status_label, text: `${current.status_label.text}${token}` } }));
    } else {
      const index = Number(focusKey.split(':')[1]);
      setSpec((current) => ({
        ...current,
        buttons: current.buttons.map((button, position) =>
          position === index ? { ...button, text: `${button.text}${token}` } : button,
        ),
      }));
    }
  };

  const save = async () => {
    setSaveError(null);
    if (!name.trim()) {
      toast.error('模板名称不能为空');
      return;
    }
    if (kind === 'card_message') {
      if (source === 'builder_json' && !cardJson) {
        toast.error('builder_json 来源必须提供合法的卡片 JSON');
        return;
      }
      if (source === 'builder_template' && !templateId.trim()) {
        toast.error('builder_template 来源必须填写 template_id');
        return;
      }
      if (cardTextError) {
        toast.error('卡片 JSON 存在语法错误，请先修正');
        return;
      }
    } else {
      const issues = validateSpec(spec);
      setSpecIssues(issues);
      if (issues.length > 0) {
        toast.error('应用消息流卡片存在校验问题，请先修正');
        return;
      }
    }

    const body = buildTemplateInput();

    setSaving(true);
    try {
      if (isEdit && id) {
        const saved = await patchCardTemplate(id, body);
        toast.success('模板已保存');
        setValidation(null);
        if (saved.id === id) reload();
      } else {
        const saved = await createCardTemplate(body);
        toast.success('模板已创建');
        navigate(`/card-templates/${saved.id}`);
      }
    } catch (cause) {
      setSaveError(cause);
      toast.error(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  };

  if (isEdit && loadingTemplate && !loaded) return <Loading label="正在加载模板…" />;
  if (isEdit && loadError && !loaded) return <div className="content"><Banner tone="error">{errorMessage(loadError)}</Banner></div>;

  const patchSpec = (patch: Partial<AppFeedCardSpec>) => setSpec((current) => ({ ...current, ...patch }));

  return (
    <>
      <div className="page-header">
        <div>
          <h1>
            <span className="row tight">
              <Link to="/card-templates" className="faint small">
                ← 卡片模板
              </Link>
              <span>{isEdit ? `编辑：${loaded?.name ?? ''}` : '新建卡片模板'}</span>
            </span>
          </h1>
          <div className="subtitle">
            {isEdit ? 'kind 不可修改；如需切换请新建模板。' : '选择消息类型后填写模板内容。'}
          </div>
        </div>
        <div className="page-actions">
          <Button onClick={() => void runValidate()} loading={validating}>
            校验
          </Button>
          <Button onClick={() => void runPreview()} loading={previewing}>
            预览
          </Button>
          <Button variant="primary" onClick={() => void save()} loading={saving}>
            {isEdit ? '保存模板' : '创建模板'}
          </Button>
        </div>
      </div>

      {saveError ? (
        <Banner tone="error">
          <div>{errorMessage(saveError)}</div>
          {errorDetails(saveError) ? (
            <div className="mt-8">
              <JsonBlock value={errorDetails(saveError)} maxHeight={240} />
            </div>
          ) : null}
        </Banner>
      ) : null}

      <Card title="基本信息">
        <div className="form-grid">
          <Field label="模板名称" required>
            <Input value={name} onChange={(event) => setName(event.target.value)} />
          </Field>
          <Field label="kind（消息类型）" required>
            {isEdit ? (
              <div className="row tight">
                <StatusBadge kind="message_type" value={kind} />
                <span className="faint small">已创建，不可更改</span>
              </div>
            ) : (
              <RadioGroup<CardTemplateKind>
                value={kind}
                options={[
                  { value: 'card_message', label: 'card_message · 卡片消息' },
                  { value: 'app_feed_card', label: 'app_feed_card · 应用消息流消息' },
                ]}
                onChange={setKind}
              />
            )}
          </Field>
        </div>
        <Switch
          checked={isDefault}
          onChange={setIsDefault}
          label="is_default · 设为该类型的默认模板"
          hint="每种 kind 至多一个 is_default = true（domain-model §4）"
        />
      </Card>

      {kind === 'card_message' ? (
        <>
          <Card
            title="卡片消息内容"
            subtitle="本地卡片 JSON 2.0：可视化拖拽或直接改 JSON，两者编辑的是同一份文档"
            actions={
              source === 'builder_json' ? (
                <>
                  <Button
                    size="sm"
                    variant={view === 'visual' ? 'primary' : 'default'}
                    onClick={() => setView('visual')}
                  >
                    可视化
                  </Button>
                  <Button
                    size="sm"
                    variant={view === 'json' ? 'primary' : 'default'}
                    onClick={() => setView('json')}
                  >
                    JSON
                  </Button>
                </>
              ) : null
            }
          >
            {source === 'builder_template' ? (
              <Banner tone="warn">
                该模板使用的是<strong>已停用</strong>的来源 <code>builder_template</code>
                （引用飞书搭建工具模板）。本地没有卡片 JSON 就无法可视化编辑、也无法预览；
                请点「转换为本地卡片」——本地 JSON 卡片同样支持变量、原地更新与撤回。
                <div className="row mt-8">
                  <Button variant="primary" onClick={convertLegacyCard} disabled={!defaults}>
                    转换为本地卡片
                  </Button>
                </div>
              </Banner>
            ) : null}

            {source === 'builder_json' && view === 'visual' ? (
              cardJson ? (
                <CardBuilder
                  card={cardJson}
                  meta={meta}
                  starter={defaults?.card_message.card_json ?? null}
                  onChange={applyCardJson}
                />
              ) : (
                <Banner tone="warn">
                  当前没有可用的卡片 JSON。请点「载入起步卡片」，或在「JSON」页签粘贴一份。
                  <div className="row mt-8">
                    <Button variant="primary" onClick={loadStarterCard} disabled={!defaults}>
                      载入起步卡片
                    </Button>
                  </div>
                </Banner>
              )
            ) : null}

            {source === 'builder_json' && view === 'json' ? (
              <>
                <Field
                  label="card_json（CardKit JSON 2.0）"
                  hint={'失焦时校验 JSON 语法；schema 必须为 "2.0"，组件数 ≤ 200，体积 ≤ 30KB（business-rules §6）'}
                  error={cardTextError}
                >
                  <Textarea
                    className="mono"
                    rows={16}
                    value={cardText}
                    placeholder={'{\n  "schema": "2.0",\n  "config": {},\n  "header": {},\n  "body": { "elements": [] }\n}'}
                    onChange={(event) => setCardText(event.target.value)}
                    onBlur={(event) => void parseCardText(event.target.value)}
                  />
                </Field>
                <div className="row small muted">
                  <span>已解析：{cardJson ? '是' : '否'}</span>
                  <span>体积：{cardJson ? formatBytes(JSON.stringify(cardJson).length) : '—'}（上限 30KB）</span>
                  {cardJson && typeof cardJson.schema === 'string' ? (
                    <span className={cardJson.schema === '2.0' ? '' : 'field-error'}>schema = {String(cardJson.schema)}</span>
                  ) : null}
                </div>

                <Field
                  label="粘贴卡片 JSON"
                  hint="支持原始卡片 JSON，或包含 card_json / cardJson / card 字段的包装对象，也容忍 ```json 代码块（兼容搭建工具导出的卡）"
                  error={importError}
                >
                  <Textarea
                    className="mono"
                    rows={6}
                    value={importText}
                    placeholder='{"schema":"2.0", ...}'
                    onChange={(event) => setImportText(event.target.value)}
                  />
                </Field>
                <div className="row">
                  <Button onClick={importCard} disabled={!importText.trim()}>
                    导入到编辑器
                  </Button>
                  <Button
                    variant="ghost"
                    onClick={() => {
                      setImportText('');
                      setImportError(null);
                    }}
                  >
                    清空
                  </Button>
                </div>
              </>
            ) : null}
          </Card>
        </>
      ) : (
        <>
          <Card title="应用消息流卡片（app_feed_spec）" subtitle="字段受飞书 open_app_feed_card 约束（domain-model §4）">
            <Field label="title" required>
              <Input
                value={spec.title}
                onFocus={() => setFocusKey('title')}
                onChange={(event) => patchSpec({ title: event.target.value })}
              />
              <LengthHint value={spec.title} max={60} />
            </Field>

            <Field label="preview">
              <Textarea
                rows={3}
                value={spec.preview}
                onFocus={() => setFocusKey('preview')}
                onChange={(event) => patchSpec({ preview: event.target.value })}
              />
              <LengthHint value={spec.preview} max={120} />
            </Field>

            <div className="grid-3">
              <Field label="status_label.text">
                <Input
                  value={spec.status_label.text}
                  onFocus={() => setFocusKey('status_text')}
                  onChange={(event) =>
                    patchSpec({ status_label: { ...spec.status_label, text: event.target.value } })
                  }
                />
                <LengthHint value={spec.status_label.text} max={20} />
              </Field>
              <Field label="status_label.type_firing" hint="firing 时标签颜色">
                <Select
                  value={spec.status_label.type_firing}
                  onChange={(event) =>
                    patchSpec({
                      status_label: { ...spec.status_label, type_firing: event.target.value as StatusLabelType },
                    })
                  }
                >
                  <Options options={enumOptionsOf(meta.status_label_types, spec.status_label.type_firing)} />
                </Select>
              </Field>
              <Field label="status_label.type_resolved" hint="resolved 时标签颜色">
                <Select
                  value={spec.status_label.type_resolved}
                  onChange={(event) =>
                    patchSpec({
                      status_label: { ...spec.status_label, type_resolved: event.target.value as StatusLabelType },
                    })
                  }
                >
                  <Options options={enumOptionsOf(meta.status_label_types, spec.status_label.type_resolved)} />
                </Select>
              </Field>
            </div>

            <div className="grid-2">
              <Field label="link.enabled" hint="创建应用消息流卡片时 link 必填（domain-model §4）">
                <Switch
                  checked={spec.link.enabled}
                  onChange={(next) => patchSpec({ link: { ...spec.link, enabled: next } })}
                  label={spec.link.enabled ? '启用 link' : '未启用（飞书会拒绝创建）'}
                />
              </Field>
              <Field label="link.url" error={specIssues.find((issue) => issue.field === 'link.url')?.message}>
                <Input
                  className="mono"
                  value={spec.link.url}
                  onFocus={() => setFocusKey('title')}
                  onChange={(event) => patchSpec({ link: { ...spec.link, url: event.target.value } })}
                />
              </Field>
            </div>

            <Field label="buttons" hint="最多 2 个；文案 ≤ 30 字符">
              {spec.buttons.map((button, index) => (
                <div key={index} className="mb-8">
                  <div className="builder-row">
                    <Input
                      value={button.text}
                      placeholder="按钮文案"
                      onFocus={() => setFocusKey(`button:${index}:text`)}
                      onChange={(event) =>
                        patchSpec({
                          buttons: spec.buttons.map((item, position) =>
                            position === index ? { ...item, text: event.target.value } : item,
                          ),
                        })
                      }
                    />
                    <Input
                      className="mono"
                      value={button.url}
                      placeholder="https://..."
                      onChange={(event) =>
                        patchSpec({
                          buttons: spec.buttons.map((item, position) =>
                            position === index ? { ...item, url: event.target.value } : item,
                          ),
                        })
                      }
                    />
                    <div className="row tight">
                      <Select
                        value={button.button_type}
                        onChange={(event) =>
                          patchSpec({
                            buttons: spec.buttons.map((item, position) =>
                              position === index
                                ? { ...item, button_type: event.target.value as FeedButtonType }
                                : item,
                            ),
                          })
                        }
                        style={{ width: 130 }}
                      >
                        <Options options={enumOptionsOf(meta.feed_button_types, button.button_type)} />
                      </Select>
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => patchSpec({ buttons: spec.buttons.filter((_, position) => position !== index) })}
                      >
                        删除
                      </Button>
                    </div>
                  </div>
                  <LengthHint value={button.text} max={30} label="按钮文案" />
                </div>
              ))}
              <Button
                size="sm"
                disabled={spec.buttons.length >= 2}
                onClick={() => patchSpec({ buttons: [...spec.buttons, { text: '', url: '', button_type: 'primary' }] })}
                style={{ marginTop: 8 }}
              >
                添加按钮
              </Button>
              {spec.buttons.length >= 2 ? <span className="hint"> 最多 2 个按钮（domain-model §4）</span> : null}
            </Field>

            <div className="grid-2">
              <Field label="close_notify" hint="飞书字段 close_notify">
                <Switch
                  checked={spec.close_notify}
                  onChange={(next) => patchSpec({ close_notify: next })}
                  label={spec.close_notify ? 'true' : 'false'}
                />
              </Field>
              <Field label="biz_id_source" hint="AlertGroupId（默认）| HookId | Custom">
                <Select
                  value={spec.biz_id_source}
                  onChange={(event) => patchSpec({ biz_id_source: event.target.value as BizIdSource })}
                >
                  <Options options={enumOptionsOf(meta.biz_id_sources, spec.biz_id_source)} />
                </Select>
              </Field>
            </div>

            {specIssues.length > 0 ? (
              <Banner tone="error">
                <ul style={{ margin: 0, paddingLeft: 18 }}>
                  {specIssues.map((issue, index) => (
                    <li key={index}>
                      <code>{issue.field}</code>：{issue.message}
                    </li>
                  ))}
                </ul>
              </Banner>
            ) : null}
          </Card>
        </>
      )}

      <Card title="变量" subtitle="点击变量会把 ${name} 插入到最后一次聚焦的文本字段">
        <div className="callout info mb-16">
          支持 <code>{'${a.b.c}'}</code> 点号取值与 <code>{'${label:foo}'}</code> /{' '}
          <code>{'${annotation:bar}'}</code> 显式取值；未知变量渲染为空串并在 validate 的 warnings 中返回（business-rules §5）。
        </div>

        <h3 className="mb-8">内置变量（GET /api/v1/meta）</h3>
        <div className="row tight mb-16">
          {(meta.variables ?? []).map((variable) => (
            <button
              key={`${variable.group}:${variable.name}`}
              type="button"
              className="chip"
              title={`${variable.group} · ${variable.type}${variable.description ? ` · ${variable.description}` : ''}`}
              onClick={() => insertVariable(variable.name)}
            >
              {variable.name}
            </button>
          ))}
        </div>

        {kind === 'card_message' ? (
          <>
            <h3 className="mb-8">检测到的变量（来自校验）</h3>
            <div className="row tight mb-16">
              {validation ? (
                validation.detected_variables.length > 0 ? (
                  validation.detected_variables.map((variable) => (
                    <button key={variable} type="button" className="chip" onClick={() => insertVariable(variable)}>
                      {variable}
                    </button>
                  ))
                ) : (
                  <span className="faint small">未检测到变量，先运行「校验」。</span>
                )
              ) : (
                <span className="faint small">先运行「校验」以检测模板中的变量。</span>
              )}
            </div>
            {validation && validation.unknown_variables.length > 0 ? (
              <Banner tone="warn">
                未知变量（渲染为空串）：{validation.unknown_variables.join('、')}
              </Banner>
            ) : null}
            {declaredVariables.length > 0 ? (
              <div className="var-panel">
                <table className="data">
                  <thead>
                    <tr>
                      <th>声明变量</th>
                      <th>类型</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {declaredVariables.map((variable) => (
                      <tr key={variable.name}>
                        <td className="mono">{variable.name}</td>
                        <td>{variable.ty}</td>
                        <td className="actions">
                          <CopyButton value={`\${${variable.name}}`} small label="复制 ${}" />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : null}
          </>
        ) : (
          <p className="faint small mb-0">点击上方变量按钮即可插入到当前聚焦的字段。</p>
        )}
      </Card>

      <MockPanel
        meta={meta}
        channels={channels}
        alertGroups={alertGroups}
        channelId={previewChannelId}
        onChannelIdChange={setPreviewChannelId}
        sample={sample}
        onSampleChange={setSample}
        mock={mock}
        onMockChange={setMock}
      />

      <Card title="校验与预览">
        <div className="form-inline mb-16">
          <Field label="样例（sample）">
            <RadioGroup<'firing' | 'resolved'>
              value={sample}
              options={[
                { value: 'firing', label: 'firing（未解决）' },
                { value: 'resolved', label: 'resolved（已恢复）' },
              ]}
              onChange={setSample}
            />
          </Field>
          <Field label="预览用渠道（channel_id，可选）">
            <Select value={previewChannelId} onChange={(event) => setPreviewChannelId(event.target.value)} style={{ width: 240 }}>
              <option value="">（不指定）</option>
              {channels.map((channel) => (
                <option key={channel.id} value={channel.id}>
                  {channel.name}
                </option>
              ))}
            </Select>
          </Field>
          <Button onClick={() => void runValidate()} loading={validating}>
            校验
          </Button>
          <Button onClick={() => void runPreview()} loading={previewing}>
            预览
          </Button>
        </div>

        {validation ? (
          <div className="mb-16">
            <div className="row mb-8">
              <Badge tone={validation.valid ? 'ok' : 'danger'}>{validation.valid ? '校验通过' : '校验未通过'}</Badge>
              <span className="faint small">
                错误 {validation.errors.length} · 警告 {validation.warnings.length} · 检测到变量{' '}
                {validation.detected_variables.length} · 未知变量 {validation.unknown_variables.length}
              </span>
            </div>
            {validation.errors.length > 0 ? (
              <Banner tone="error">
                <ul style={{ margin: 0, paddingLeft: 18 }}>
                  {validation.errors.map((item, index) => (
                    <li key={index}>{validationText(item)}</li>
                  ))}
                </ul>
              </Banner>
            ) : null}
            {validation.warnings.length > 0 ? (
              <Banner tone="warn">
                <ul style={{ margin: 0, paddingLeft: 18 }}>
                  {validation.warnings.map((item, index) => (
                    <li key={index}>{validationText(item)}</li>
                  ))}
                </ul>
              </Banner>
            ) : null}
          </div>
        ) : null}

        {preview ? (
          <>
            <h3 className="mb-8">渲染结果</h3>
            {preview.rendered_card_json ? <JsonBlock value={preview.rendered_card_json} maxHeight={420} /> : null}
            {preview.rendered_app_feed_spec ? <JsonBlock value={preview.rendered_app_feed_spec} maxHeight={420} /> : null}
            <h3 className="mb-8 mt-16">渲染上下文变量</h3>
            <DescriptionList
              items={Object.entries(preview.variables ?? {}).map(([key, value]) => ({
                label: key,
                value: <span className="mono break-all">{typeof value === 'string' ? value : prettyJson(value)}</span>,
              }))}
            />
          </>
        ) : (
          <p className="faint small mb-0">
            预览会调用 POST /card-templates/preview，用 mock 基线 + 覆盖项渲染模板。
          </p>
        )}
      </Card>

      <Card
        title="发送测试"
        subtitle="草稿测试走 POST /card-templates/send-test（不落库）；已保存模板走 POST /card-templates/:id/send-test。两者都会产生投递记录。"
      >
        <div className="form-inline">
          <Field label="渠道（channel_id）" required>
            <Select value={testChannelId} onChange={(event) => setTestChannelId(event.target.value)} style={{ width: 260 }}>
              <option value="">（请选择渠道）</option>
              {channels.map((channel) => (
                <option key={channel.id} value={channel.id}>
                  {channel.name}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="样例（sample）" hint="选择历史分组时由该分组状态决定">
            <RadioGroup<'firing' | 'resolved'>
              value={sample}
              options={[
                { value: 'firing', label: 'firing' },
                { value: 'resolved', label: 'resolved' },
              ]}
              onChange={setSample}
            />
          </Field>
          <Button
            variant="primary"
            loading={sendingTest}
            disabled={!testChannelId}
            onClick={() => void sendDraftTest()}
          >
            发送草稿测试
          </Button>
          {isEdit && id ? (
            <Button
              loading={sendingTest}
              disabled={!testChannelId}
              onClick={() => {
                void (async () => {
                  setSendingTest(true);
                  setTestResult(null);
                  try {
                    const result = await sendTestCardTemplate(id, {
                      channel_id: testChannelId,
                      sample,
                      ...(effectiveMock() ? { mock: effectiveMock() } : {}),
                    });
                    setTestResult(result);
                    toast.success('测试消息已提交发送');
                  } catch (cause) {
                    toast.error(errorMessage(cause));
                  } finally {
                    setSendingTest(false);
                  }
                })();
              }}
            >
              用已保存模板发送
            </Button>
          ) : null}
        </div>
        <p className="faint small mt-8 mb-0">
          草稿测试使用当前编辑器里的内容（含 mock 覆盖），无需先保存；投递记录里 <code>kind=test</code>。
        </p>
        {testResult ? (
          <div className="mt-16">
            <Banner tone={testResult.state === 'failed' ? 'error' : 'success'}>
              {testResult.state === 'failed'
                ? `发送失败${testResult.last_error ? `：${testResult.last_error}` : ''}`
                : `已提交发送（state=${testResult.state}）${
                    testResult.lark_message_id ? ` · message_id ${testResult.lark_message_id}` : ''
                  }`}
            </Banner>
            <JsonBlock value={testResult} maxHeight={240} />
          </div>
        ) : null}
      </Card>
    </>
  );
}
