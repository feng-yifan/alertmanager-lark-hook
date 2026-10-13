/**
 * 属性面板：节点属性 + 卡片级设置（header / config / body）。
 *
 * 表单完全由服务端组件目录的 `props` 驱动（`type` / `values` / `hint`），
 * 因此新增飞书组件只需要改 Rust 侧的 `card_catalog`，前端无需另写规则。
 */
import { useRef } from 'react';
import type { CardComponentMeta, CardJson, CardPropMeta, Meta, MetaVariable } from '../types';
import {
  getPath,
  nodeAt,
  patchNode,
  readProp,
  setPath,
  writeProp,
  type Catalog,
  type NodePath,
} from './model';

export interface InspectorProps {
  card: CardJson;
  catalog: Catalog;
  meta: Meta;
  path: NodePath | null;
  onChange: (card: CardJson, label: string) => void;
}

export function Inspector({ card, catalog, meta, path, onChange }: InspectorProps): JSX.Element {
  const node = path ? nodeAt(card, path) : null;

  if (!path || !node) {
    return <CardSettings card={card} meta={meta} onChange={onChange} />;
  }

  const tag = typeof node.tag === 'string' ? node.tag : '';
  const component: CardComponentMeta | null = catalog.get(tag);
  const patch = (updater: (current: Record<string, unknown>) => Record<string, unknown>, label: string) => {
    onChange(patchNode(card, catalog, path, updater), label);
  };

  return (
    <div className="cb-inspector">
      <div className="cb-panel-title">属性</div>
      <div className="row tight mb-8">
        <span className="cb-tag">{component?.label ?? tag}</span>
        <code className="faint small">{tag}</code>
      </div>
      {component?.description ? <p className="faint small">{component.description}</p> : null}

      <Field label="元素 ID" hint="全局唯一；局部更新与交互回调依赖它">
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
          variables={meta.variables}
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
  onChange: (value: unknown) => void;
}

export function PropField({ prop, value, variables, onChange }: FieldProps): JSX.Element {
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
  onChange,
}: {
  label: string;
  hint?: string;
  placeholder?: string;
  multiline?: boolean;
  value: string;
  variables: MetaVariable[];
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
          title="插入内置变量"
          onChange={(event) => {
            if (event.target.value) insert(event.target.value);
            event.target.value = '';
          }}
        >
          <option value="">＋变量</option>
          {variables.map((variable) => (
            <option key={variable.name} value={variable.name}>
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
      {hint ? <div className="hint">{hint}</div> : null}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* 卡片级设置（未选中节点时）                                          */
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

function CardSettings({
  card,
  meta,
  onChange,
}: {
  card: CardJson;
  meta: Meta;
  onChange: (card: CardJson, label: string) => void;
}): JSX.Element {
  const set = (path: string, value: unknown, label: string) => {
    onChange(setPath({ ...card }, path, value === '' ? undefined : value) as CardJson, label);
  };
  const read = (path: string): string => {
    const value = getPath(card, path);
    return typeof value === 'string' ? value : '';
  };

  return (
    <div className="cb-inspector">
      <div className="cb-panel-title">卡片设置</div>
      <p className="faint small">
        未选中组件时编辑卡片本身；点画布里的组件可编辑它的属性。
      </p>

      <VartextField
        label="标题"
        hint="支持 ${变量}，例如「【${status_text}】${alertname}」"
        value={read('header.title.content')}
        variables={meta.variables}
        onChange={(value) => set('header.title.content', value, '修改标题')}
      />
      <VartextField
        label="副标题"
        value={read('header.subtitle.content')}
        variables={meta.variables}
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
      <VartextField
        label="消息摘要"
        hint="聊天列表里显示的预览文案"
        value={read('config.summary.content')}
        variables={meta.variables}
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
