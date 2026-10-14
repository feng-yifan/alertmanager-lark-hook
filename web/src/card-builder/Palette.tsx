/**
 * 组件面板 + 告警积木（拖拽源）。
 *
 * 拖拽走 `@dnd-kit/core` 的 `useDraggable`（指针拖拽 + 半透明浮层），
 * 落点由画布的几何计算决定；点击则追加到卡片末尾，方便不用鼠标拖拽的场景。
 */
import type { ReactNode } from 'react';
import { useDraggable } from '@dnd-kit/core';
import type { CardComponentMeta, CardJson, CardPresetMeta } from '../types';
import type { Catalog, DragPayload, Selection } from './model';
import { GROUP_TITLES, presetNodes } from './catalog';
import { Outline } from './Outline';

export interface PaletteProps {
  card: CardJson;
  catalog: Catalog;
  presets: CardPresetMeta[];
  /** 编辑器元数据里的节点名称（key = element_id）。 */
  labels: Record<string, string>;
  selection: Selection;
  onSelect: (selection: Selection) => void;
  onAdd: (nodes: Record<string, unknown>[], label: string) => void;
  /** 拖拽刚结束时浏览器补发的 click 要忽略，避免「拖一次却加了两个」。 */
  shouldIgnoreClick: () => boolean;
}

export function Palette({
  card,
  catalog,
  presets,
  labels,
  selection,
  onSelect,
  onAdd,
  shouldIgnoreClick,
}: PaletteProps): JSX.Element {
  const palette = catalog.palette();
  const add = (nodes: Record<string, unknown>[], label: string) => {
    if (shouldIgnoreClick()) return;
    onAdd(nodes, label);
  };

  return (
    <div className="cb-palette">
      <Outline
        card={card}
        catalog={catalog}
        labels={labels}
        selection={selection}
        onSelect={onSelect}
      />
      <div className="cb-panel-title">告警积木</div>
      <p className="faint small mb-8">
        与 AlertManager 契约对齐的高频块，拖进画布即用；「告警列表」是
        <strong>可编辑的循环结构</strong>（循环 + 判断 + 文本），不再是一个黑盒变量。
      </p>
      {presets.map((preset) => {
        // 单个坏积木（服务端下发了非法 JSON）只跳过它自己，不能让整页白屏。
        const build = () => {
          try {
            return presetNodes(preset);
          } catch {
            return null;
          }
        };
        const initial = build();
        if (!initial) {
          return (
            <div key={preset.id} className="cb-block cb-block-broken">
              <div className="cb-block-label">{preset.label}</div>
              <div className="cb-block-desc warn">积木数据损坏，已跳过（nodes_json 非法）</div>
            </div>
          );
        }
        return (
          <Draggable
            key={preset.id}
            id={`cb-new:${preset.id}`}
            payload={{ kind: 'new', nodes: initial, label: preset.label }}
            className="cb-block"
            title={`${preset.label}：${preset.description}`}
            onClick={() => {
              const nodes = build();
              if (nodes) add(nodes, `添加积木「${preset.label}」`);
            }}
          >
            <div className="cb-block-label">{preset.label}</div>
            <div className="cb-block-desc">{preset.description}</div>
          </Draggable>
        );
      })}
      {presets.length === 0 ? (
        <div className="faint small">（没有取到积木清单）</div>
      ) : null}

      <div className="cb-panel-title mt-16">飞书组件</div>
      {GROUP_TITLES.map(({ group, title }) => {
        const items = palette.filter((component) => component.group === group);
        if (items.length === 0) return null;
        return (
          <div key={group} className="mb-8">
            <div className="cb-group-title">{title}</div>
            <div className="cb-components">
              {items.map((component) => (
                <ComponentChip key={component.tag} component={component} onAdd={add} />
              ))}
            </div>
          </div>
        );
      })}

      {catalog.all.some((component) => component.deprecated) ? (
        <details className="mt-8">
          <summary className="faint small">已废弃组件（JSON 2.0 不可用）</summary>
          <ul className="faint small" style={{ paddingLeft: 18, margin: '4px 0' }}>
            {catalog.all
              .filter((component) => component.deprecated)
              .map((component) => (
                <li key={component.tag}>
                  <code>{component.tag}</code> —— {component.deprecated}
                </li>
              ))}
          </ul>
        </details>
      ) : null}
    </div>
  );
}

/** 可拖拽的包装：同时保留点击行为（不拖只点 = 追加到卡片末尾）。 */
function Draggable({
  id,
  payload,
  className,
  title,
  onClick,
  children,
}: {
  id: string;
  payload: DragPayload;
  className: string;
  title?: string;
  onClick: () => void;
  children: ReactNode;
}): JSX.Element {
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({ id, data: payload });
  return (
    <div
      ref={setNodeRef}
      {...listeners}
      {...attributes}
      className={`${className}${isDragging ? ' dragging' : ''}`}
      title={title}
      onClick={onClick}
      onKeyDown={(event) => {
        // dnd-kit 的 attributes 给了 role=button，但 Enter/Space 不会自动触发 click。
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          onClick();
        }
      }}
    >
      {children}
    </div>
  );
}

function ComponentChip({
  component,
  onAdd,
}: {
  component: CardComponentMeta;
  onAdd: (nodes: Record<string, unknown>[], label: string) => void;
}): JSX.Element {
  // 单个坏组件同样只跳过它自己。
  let node: Record<string, unknown> | null = null;
  try {
    const parsed: unknown = JSON.parse(component.default_json);
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      node = parsed as Record<string, unknown>;
    }
  } catch {
    node = null;
  }
  if (!node) {
    return (
      <div className="cb-chip cb-chip-broken" title="default_json 非法，已跳过">
        {component.label}
      </div>
    );
  }
  return (
    <Draggable
      id={`cb-comp:${component.tag}`}
      payload={{ kind: 'new', nodes: [node], label: component.label }}
      className={`cb-chip${component.group === 'directive' ? ' cb-chip-directive' : ''}`}
      title={component.description}
      onClick={() => onAdd([node as Record<string, unknown>], `添加${component.label}`)}
    >
      {component.label}
    </Draggable>
  );
}
