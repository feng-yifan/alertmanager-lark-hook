/**
 * 组件面板 + 告警积木（拖拽源）。
 *
 * 拖拽走 `beginDrag({kind:'new'})`，落在画布的落点条上；点击则追加到卡片末尾，
 * 方便不用鼠标拖拽的场景。
 */
import type { CardComponentMeta, CardPresetMeta } from '../types';
import type { Catalog } from './model';
import { GROUP_TITLES, presetNodes } from './catalog';
import { beginDrag, endDrag } from './Canvas';

export interface PaletteProps {
  catalog: Catalog;
  presets: CardPresetMeta[];
  onAdd: (nodes: Record<string, unknown>[], label: string) => void;
}

export function Palette({ catalog, presets, onAdd }: PaletteProps): JSX.Element {
  const palette = catalog.palette();

  return (
    <div className="cb-palette">
      <div className="cb-panel-title">告警积木</div>
      <p className="faint small mb-8">
        与 AlertManager 契约对齐的高频块，拖进画布即用；变量来自 <code>GET /meta</code>。
      </p>
      {presets.map((preset) => {
        const build = () => presetNodes(preset);
        return (
          <div
            key={preset.id}
            className="cb-block"
            draggable
            title={`${preset.label}：${preset.description}`}
            onDragStart={(event) => beginDrag({ kind: 'new', nodes: build() }, event)}
            onDragEnd={endDrag}
            onClick={() => onAdd(build(), `添加积木「${preset.label}」`)}
          >
            <div className="cb-block-label">{preset.label}</div>
            <div className="cb-block-desc">{preset.description}</div>
          </div>
        );
      })}
      {presets.length === 0 ? (
        <div className="faint small">（未从 /meta 取到积木清单）</div>
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
                <ComponentChip
                  key={component.tag}
                  component={component}
                  onAdd={() => onAdd([JSON.parse(component.default_json) as Record<string, unknown>], `添加${component.label}`)}
                />
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

function ComponentChip({
  component,
  onAdd,
}: {
  component: CardComponentMeta;
  onAdd: () => void;
}): JSX.Element {
  return (
    <div
      className="cb-chip"
      draggable
      title={component.description}
      onDragStart={(event) => beginDrag({ kind: 'new', nodes: [JSON.parse(component.default_json) as Record<string, unknown>] }, event)}
      onDragEnd={endDrag}
      onClick={onAdd}
    >
      {component.label}
    </div>
  );
}
