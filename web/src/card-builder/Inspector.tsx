/**
 * 属性面板：节点属性 + 卡片级设置。
 *
 * 面板内容由**选中项**决定（`Selection`）：
 * - `header`：卡片标题（画布顶部那块独立组件）——标题不再和卡片设置混在一起；
 * - `node`：组件属性，表单完全由服务端组件目录的 `props` 驱动；
 * - `card`：卡片级设置（消息摘要 / 内容方向 / 内边距），有画布与工具栏两个明显入口。
 *
 * 变量下拉是**位置相关**的：只有位于「循环」组件内部的节点才能看到 `${item.*}`，
 * 避免用户在不该用的地方插入取不到值的变量。
 */
import { useRef } from 'react';
import type { CardComponentMeta, CardJson, CardPropMeta, Meta, MetaVariable } from '../types';
import {
  getPath,
  isInsideLoop,
  variableTokensInText,
  nodeAt,
  patchNode,
  readProp,
  setPath,
  writeProp,
  type Catalog,
  type Selection,
} from './model';

export interface InspectorProps {
  card: CardJson;
  catalog: Catalog;
  meta: Meta;
  selection: Selection;
  /** 编辑器元数据里的节点名称（key = element_id）。 */
  labels?: Record<string, string>;
  /** 给选中节点改名（会写进 `builder_meta.labels`，必要时先补一个 element_id）。 */
  onRenameNode?: (name: string) => void;
  onChange: (card: CardJson, label: string) => void;
}

/** 按所在位置过滤变量：`item.*` 只在循环体内可用。 */
export function variablesFor(variables: MetaVariable[], inLoop: boolean): MetaVariable[] {
  if (inLoop) return variables;
  return variables.filter((variable) => {
    const itemScoped = variable.group === 'item' || variable.name.startsWith('item.');
    return !itemScoped;
  });
}

export function Inspector({
  card,
  catalog,
  meta,
  selection,
  labels,
  onRenameNode,
  onChange,
}: InspectorProps): JSX.Element {
  if (selection.kind === 'header') {
    return <HeaderSettings card={card} meta={meta} onChange={onChange} />;
  }

  if (selection.kind === 'node') {
    const node = nodeAt(card, selection.path);
    if (node) {
      return (
        <NodeSettings
          card={card}
          catalog={catalog}
          meta={meta}
          path={selection.path}
          node={node}
          name={
            typeof node.element_id === 'string'
              ? (labels?.[node.element_id] ?? '')
              : ''
          }
          canRename={!isInsideLoop(card, selection.path)}
          onRenameNode={onRenameNode}
          onChange={onChange}
        />
      );
    }
  }

  return <CardSettings card={card} meta={meta} onChange={onChange} />;
}

/* ------------------------------------------------------------------ */
/* 节点属性                                                            */
/* ------------------------------------------------------------------ */

function NodeSettings({
  card,
  catalog,
  meta,
  path,
  node,
  name,
  canRename,
  onRenameNode,
  onChange,
}: {
  card: CardJson;
  catalog: Catalog;
  meta: Meta;
  path: number[];
  node: Record<string, unknown>;
  name: string;
  canRename: boolean;
  onRenameNode?: (name: string) => void;
  onChange: (card: CardJson, label: string) => void;
}): JSX.Element {
  const tag = typeof node.tag === 'string' ? node.tag : '';
  const component: CardComponentMeta | null = catalog.get(tag);
  const inLoop = isInsideLoop(card, path);
  // 下拉用「当前位置可用」的清单；判定（胶囊）用完整清单，否则 item.* 会被误判成未知
  const variables = meta.variables;
  const patch = (
    updater: (current: Record<string, unknown>) => Record<string, unknown>,
    label: string,
  ) => {
    onChange(patchNode(card, catalog, path, updater), label);
  };
  const hasItemVariables = meta.variables.some(
    (variable) => variable.group === 'item' || variable.name.startsWith('item.'),
  );

  return (
    <div className="cb-inspector">
      <div className="cb-panel-title">属性</div>
      <div className="row tight mb-8">
        <span className="cb-tag">{component?.label ?? tag}</span>
        <code className="faint small">{tag}</code>
        {inLoop ? <span className="cb-block-badge">循环内</span> : null}
      </div>
      {component?.description ? <p className="faint small">{component.description}</p> : null}

      {onRenameNode ? (
        <Field
          label="节点名称"
          hint={
            canRename
              ? '只影响画布上的叫法，随模板保存；留空则回退显示积木 / 组件名'
              : '循环体内的组件会被复制多份，不能单独命名'
          }
        >
          <input
            // 切换选中节点时强制重挂载：否则 defaultValue 会留着上一个节点的名字
            key={path.join('.')}
            className="input"
            placeholder={component?.label ?? tag}
            defaultValue={name}
            disabled={!canRename}
            onBlur={(event) => onRenameNode(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') (event.target as HTMLInputElement).blur();
            }}
          />
        </Field>
      ) : null}

      {component?.group === 'directive' ? (
        <div className="callout info small">
          这是<strong>本地指令</strong>：保存的是指令本身，预览与发送时才由服务端展开成飞书组件。
        </div>
      ) : null}

      {inLoop ? (
        <div className="callout info small">
          当前位于「循环」内：用 <code>{'${item.xxx}'}</code> 取**当前这一条**告警
          （如 <code>{'${item.alertname}'}</code>、<code>{'${item.summary}'}</code>）。
          <code>{'${alertname}'}</code> 这类变量仍取自**代表告警**，逐条内容建议不要用。
        </div>
      ) : hasItemVariables ? (
        <div className="callout info small">
          <code>{'${item.*}'}</code> 变量只在「循环」组件的子组件里可用，这里不会出现在变量下拉中。
        </div>
      ) : null}

      <Field
        label="元素 ID"
        hint={
          inLoop
            ? '循环体内填了也会在渲染时被剥掉：一份模板会变多份，id 无法唯一定位'
            : '全局唯一；局部更新与交互回调依赖它'
        }
      >
        <input
          className="input mono"
          value={typeof node.element_id === 'string' ? node.element_id : ''}
          placeholder="（可选）"
          onChange={(event) => {
            const value = event.target.value.trim();
            patch((current) => {
              const next = { ...current };
              if (value) next.element_id = value;
              else delete next.element_id;
              return next;
            }, '修改 element_id');
          }}
        />
      </Field>

      {component?.props.map((prop) => (
        <PropField
          key={prop.name}
          prop={prop}
          value={readProp(node, prop)}
          variables={variables}
          inLoop={inLoop}
          onChange={(value) => patch((current) => writeProp(current, prop, value), `修改${prop.label}`)}
        />
      ))}

      <details className="mt-8">
        <summary className="faint small">节点 JSON</summary>
        <pre className="cb-raw">{JSON.stringify(node, null, 2)}</pre>
      </details>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* 单字段                                                              */
/* ------------------------------------------------------------------ */

interface FieldProps {
  prop: CardPropMeta;
  value: unknown;
  variables: MetaVariable[];
  inLoop: boolean;
  onChange: (value: unknown) => void;
}

export function PropField({ prop, value, variables, inLoop, onChange }: FieldProps): JSX.Element {
  const asText =
    typeof value === 'string' ? value : value === undefined || value === null ? '' : String(value);

  if (prop.type === 'bool') {
    return (
      <Field label={prop.label} hint={prop.hint}>
        <label className="cb-switch">
          <input
            type="checkbox"
            checked={value === true}
            onChange={(event) => onChange(event.target.checked)}
          />
          <span>{value === true ? '开启' : '关闭'}</span>
        </label>
      </Field>
    );
  }

  if (prop.type === 'enum') {
    return (
      <Field label={prop.label} hint={prop.hint}>
        <select className="select" value={asText} onChange={(event) => onChange(event.target.value)}>
          <option value="">（默认）</option>
          {prop.values.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
      </Field>
    );
  }

  if (prop.type === 'number') {
    return (
      <Field label={prop.label} hint={prop.hint}>
        <input
          className="input"
          type="number"
          value={asText}
          onChange={(event) => onChange(event.target.value === '' ? '' : Number(event.target.value))}
        />
      </Field>
    );
  }

  return (
    <VartextField
      label={prop.label}
      hint={prop.hint}
      placeholder={prop.placeholder}
      multiline={prop.type === 'markdown'}
      value={asText}
      variables={variables}
      inLoop={inLoop}
      onChange={(next) => onChange(next)}
    />
  );
}

/** 带「插入变量」的文本字段：变量在下拉里选，插入到光标处。 */
export function VartextField({
  label,
  hint,
  placeholder,
  multiline,
  value,
  variables,
  inLoop = false,
  onChange,
}: {
  label: string;
  hint?: string;
  placeholder?: string;
  multiline?: boolean;
  value: string;
  variables: MetaVariable[];
  /** 当前字段所在位置是否在「循环」体内（决定 item.* 的可用性）。 */
  inLoop?: boolean;
  onChange: (value: string) => void;
}): JSX.Element {
  const ref = useRef<HTMLInputElement | HTMLTextAreaElement | null>(null);
  const caret = useRef<{ start: number; end: number } | null>(null);

  const remember = () => {
    const element = ref.current;
    if (!element) return;
    caret.current = {
      start: element.selectionStart ?? element.value.length,
      end: element.selectionEnd ?? element.value.length,
    };
  };

  const insert = (name: string) => {
    const token = `\${${name}}`;
    const position = caret.current ?? { start: value.length, end: value.length };
    const end = Math.min(position.end, value.length);
    const start = Math.min(position.start, end);
    onChange(`${value.slice(0, start)}${token}${value.slice(end)}`);
    caret.current = { start: start + token.length, end: start + token.length };
  };

  const common = {
    value,
    placeholder,
    onSelect: remember,
    onKeyUp: remember,
    onClick: remember,
    onFocus: remember,
    onChange: (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) =>
      onChange(event.target.value),
  };

  return (
    <div className="field">
      <div className="cb-field-head">
        <label>{label}</label>
        <select
          className="cb-var-select"
          value=""
          title="插入变量（清单与当前组件所处位置相关）"
          onChange={(event) => {
            if (event.target.value) insert(event.target.value);
            event.target.value = '';
          }}
        >
          <option value="">＋变量</option>
          {variablesFor(variables, inLoop).map((variable) => (
            <option key={`${variable.group}:${variable.name}`} value={variable.name}>
              {variable.name}
            </option>
          ))}
        </select>
      </div>
      {multiline ? (
        <textarea
          className="input mono"
          rows={4}
          ref={(element) => {
            ref.current = element;
          }}
          {...common}
        />
      ) : (
        <input
          className="input mono"
          ref={(element) => {
            ref.current = element;
          }}
          {...common}
        />
      )}
      <BindingChips value={value} variables={variables} inLoop={inLoop} />
      {hint ? <div className="hint">{hint}</div> : null}
    </div>
  );
}

/** 字段里已写入的变量：就地显示绑定状态（可用 / 越权 / 未知）。 */
function BindingChips({
  value,
  variables,
  inLoop,
}: {
  value: string;
  variables: MetaVariable[];
  inLoop: boolean;
}): JSX.Element | null {
  const tokens = variableTokensInText(value, variables, inLoop);
  if (tokens.length === 0) return null;
  return (
    <div className="cb-binding-row">
      {tokens.map((token) => (
        <span
          key={token.name}
          className={`cb-binding cb-binding-${token.state}`}
          title={token.description}
        >
          {'${'}
          {token.name}
          {'}'}
        </span>
      ))}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* 卡片标题（独立组件）                                                */
/* ------------------------------------------------------------------ */

const HEADER_TEMPLATES = [
  'default',
  'blue',
  'wathet',
  'turquoise',
  'green',
  'yellow',
  'orange',
  'red',
  'carmine',
  'violet',
  'purple',
  'indigo',
  'grey',
];

const STATUS_COLOR_TOKEN = '${status_color}';
const SEVERITY_COLOR_TOKEN = '${severity_color}';

/** 卡片级读写（header / config / body 都是卡片对象上的字段）。 */
function cardFieldOps(card: CardJson, onChange: (card: CardJson, label: string) => void) {
  return {
    set: (path: string, value: unknown, label: string) => {
      onChange(setPath({ ...card }, path, value === '' ? undefined : value) as CardJson, label);
    },
    read: (path: string): string => {
      const value = getPath(card, path);
      return typeof value === 'string' ? value : '';
    },
  };
}

function HeaderSettings({
  card,
  meta,
  onChange,
}: {
  card: CardJson;
  meta: Meta;
  onChange: (card: CardJson, label: string) => void;
}): JSX.Element {
  const { set, read } = cardFieldOps(card, onChange);
  // 传入完整清单：下拉在 VartextField 里按作用域过滤，胶囊判定需要完整清单
  const variables = meta.variables;

  return (
    <div className="cb-inspector">
      <div className="cb-panel-title">卡片标题</div>
      <p className="faint small">
        标题是独立的可配置组件（画布顶部那一块），不再和卡片设置混在一起。
      </p>

      <VartextField
        label="标题"
        hint="支持 ${变量}，例如「【${status_text}】${alertname}」"
        value={read('header.title.content')}
        variables={variables}
        onChange={(value) => set('header.title.content', value, '修改标题')}
      />
      <VartextField
        label="副标题"
        value={read('header.subtitle.content')}
        variables={variables}
        onChange={(value) => set('header.subtitle.content', value, '修改副标题')}
      />
      <Field
        label="标题颜色"
        hint="按状态配色（恢复后变绿）请选 ${status_color}；只按严重级别配色请选 ${severity_color}"
      >
        <select
          className="select"
          value={read('header.template')}
          onChange={(event) => set('header.template', event.target.value, '修改标题颜色')}
        >
          <option value="">（默认）</option>
          <option value={STATUS_COLOR_TOKEN}>{`${STATUS_COLOR_TOKEN}（按状态，恢复变绿）`}</option>
          <option value={SEVERITY_COLOR_TOKEN}>{`${SEVERITY_COLOR_TOKEN}（按严重级别）`}</option>
          {HEADER_TEMPLATES.map((template) => (
            <option key={template} value={template}>
              {template}
            </option>
          ))}
        </select>
      </Field>

      <details className="mt-8">
        <summary className="faint small">header JSON</summary>
        <pre className="cb-raw">{JSON.stringify(card.header ?? {}, null, 2)}</pre>
      </details>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* 卡片级设置（未选中组件/标题时）                                     */
/* ------------------------------------------------------------------ */

function CardSettings({
  card,
  meta,
  onChange,
}: {
  card: CardJson;
  meta: Meta;
  onChange: (card: CardJson, label: string) => void;
}): JSX.Element {
  const { set, read } = cardFieldOps(card, onChange);
  // 传入完整清单：下拉在 VartextField 里按作用域过滤，胶囊判定需要完整清单
  const variables = meta.variables;

  return (
    <div className="cb-inspector">
      <div className="cb-panel-title">卡片设置</div>
      <p className="faint small">
        卡片级设置（标题请点画布顶部的「卡片标题」块）。
      </p>

      <VartextField
        label="消息摘要"
        hint="聊天列表里显示的预览文案"
        value={read('config.summary.content')}
        variables={variables}
        onChange={(value) => set('config.summary.content', value, '修改摘要')}
      />
      <div className="grid-2">
        <Field label="内容方向">
          <select
            className="select"
            value={read('body.direction')}
            onChange={(event) => set('body.direction', event.target.value, '修改内容方向')}
          >
            <option value="">垂直（默认）</option>
            <option value="vertical">垂直</option>
            <option value="horizontal">水平</option>
          </select>
        </Field>
        <Field label="内边距">
          <input
            className="input"
            placeholder="12px"
            value={read('body.padding')}
            onChange={(event) => set('body.padding', event.target.value, '修改内边距')}
          />
        </Field>
      </div>
      <div className="callout info small">
        卡片配置里的「多端同步更新」固定为开启：原地更新旧卡片依赖它，JSON 2.0 也只支持开启。
      </div>
      <div className="callout info small">
        提示：标题在画布顶部的「卡片标题」块里单独编辑；循环 / 条件组件在左侧「本地指令」分组。
      </div>
    </div>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}): JSX.Element {
  return (
    <div className="field">
      <label>{label}</label>
      {children}
      {hint ? <div className="hint">{hint}</div> : null}
    </div>
  );
}
