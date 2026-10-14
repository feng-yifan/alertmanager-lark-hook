/**
 * 结构大纲：把卡片画成一棵可点击的缩进树。
 *
 * 画布擅长「看长相」，树擅长「看层级」——深嵌套（循环 → 分栏 → 条件 → 文本）
 * 光靠画布很容易迷路，这里给出一个稳定的导航入口。
 */
import type { CardJson } from '../types';
import {
  childrenAt,
  isPlainObject,
  samePath,
  type Catalog,
  type NodePath,
  type Selection,
} from './model';

export interface OutlineProps {
  card: CardJson;
  catalog: Catalog;
  /** 编辑器元数据里的节点名称（key = element_id）。 */
  labels: Record<string, string>;
  selection: Selection;
  onSelect: (selection: Selection) => void;
}

export function Outline({ card, catalog, labels, selection, onSelect }: OutlineProps): JSX.Element {
  return (
    <details className="cb-outline" open>
      <summary className="cb-panel-title">结构</summary>
      <button
        type="button"
        className={`cb-outline-row${selection.kind === 'header' ? ' selected' : ''}`}
        onClick={() => onSelect({ kind: 'header' })}
      >
        <span className="cb-outline-tag">卡片标题</span>
      </button>
      <OutlineList
        card={card}
        catalog={catalog}
        labels={labels}
        parentPath={[]}
        selection={selection}
        onSelect={onSelect}
      />
      <button
        type="button"
        className={`cb-outline-row${selection.kind === 'card' ? ' selected' : ''}`}
        onClick={() => onSelect({ kind: 'card' })}
      >
        <span className="cb-outline-tag">卡片设置</span>
        <span className="cb-outline-preview">摘要 / 方向 / 内边距</span>
      </button>
    </details>
  );
}

interface ListProps extends OutlineProps {
  parentPath: NodePath;
}

function OutlineList({
  card,
  catalog,
  labels,
  parentPath,
  selection,
  onSelect,
}: ListProps): JSX.Element | null {
  const list = childrenAt(card, catalog, parentPath);
  if (list.length === 0) {
    return parentPath.length === 0 ? (
      <div className="faint small cb-outline-empty">（卡片还是空的）</div>
    ) : null;
  }
  return (
    <ul className="cb-outline-list">
      {list.map((node, index) => {
        const path = [...parentPath, index];
        const record = isPlainObject(node) ? node : {};
        const tag = typeof record.tag === 'string' ? record.tag : '';
        const meta = catalog.get(tag);
        // 自定义名称优先，与画布保持一致
        const custom =
          typeof record.element_id === 'string' ? (labels[record.element_id] ?? '') : '';
        const label = custom || meta?.label || (tag || '未命名');
        const hint =
          typeof record.content === 'string'
            ? record.content
            : isPlainObject(record.text) && typeof record.text.content === 'string'
              ? record.text.content
              : '';
        const selected = selection.kind === 'node' && samePath(selection.path, path);
        return (
          <li key={path.join('.')}>
            <button
              type="button"
              className={`cb-outline-row${selected ? ' selected' : ''}`}
              title={label}
              onClick={() => onSelect({ kind: 'node', path })}
            >
              <span className="cb-outline-tag">{label}</span>
              {hint ? <span className="cb-outline-preview">{hint}</span> : null}
            </button>
            <OutlineList
              card={card}
              catalog={catalog}
              labels={labels}
              parentPath={path}
              selection={selection}
              onSelect={onSelect}
            />
          </li>
        );
      })}
    </ul>
  );
}
