/**
 * 可视化画布：原生 HTML5 拖拽（无第三方依赖）。
 *
 * 拖放模型刻意做得**确定**：相邻子节点之间渲染一个细长的「落点条」，
 * 由落点条自己处理 `dragover/drop`，因此不存在「按坐标猜插入位置」和嵌套
 * 容器的命中歧义。拖拽载荷放在模块级变量里（`dragover` 阶段读不到
 * `dataTransfer` 的数据），同页只允许一个画布实例。
 */
import { useState } from 'react';
import type { CardComponentMeta, CardJson } from '../types';
import {
  childList,
  childrenAt,
  countElements,
  deepClone,
  ID_TAGS,
  insertNode,
  isDescendant,
  isPlainObject,
  moveNode,
  nextElementId,
  nodeAt,
  patchNode,
  removeNode,
  samePath,
  type Catalog,
  type NodePath,
} from './model';

export interface DragPayload {
  kind: 'new' | 'move';
  nodes?: Record<string, unknown>[];
  path?: NodePath;
}

/** 当前正在拖拽的载荷。 */
let currentDrag: DragPayload | null = null;

export function beginDrag(payload: DragPayload, event: React.DragEvent): void {
  currentDrag = payload;
  event.dataTransfer.effectAllowed = payload.kind === 'new' ? 'copy' : 'move';
  try {
    // 让浏览器认为这是一次合法拖拽（部分浏览器要求 setData 才会触发 drop）。
    event.dataTransfer.setData('text/plain', payload.kind);
  } catch {
    /* Safari 某些场景会抛错，忽略 */
  }
}

export function endDrag(): void {
  currentDrag = null;
}

export interface CanvasProps {
  card: CardJson;
  catalog: Catalog;
  selected: NodePath | null;
  onSelect: (path: NodePath | null) => void;
  onChange: (card: CardJson, label: string) => void;
}

export function CardCanvas({ card, catalog, selected, onSelect, onChange }: CanvasProps): JSX.Element {
  return (
    <div className="cb-canvas" onDragEnd={endDrag} onClick={() => onSelect(null)}>
      <ChildrenList
        card={card}
        catalog={catalog}
        parentPath={[]}
        parentTag={null}
        selected={selected}
        onSelect={onSelect}
        onChange={onChange}
      />
      {countElements(card) === 0 ? (
        <div className="cb-empty">
          从左侧拖入组件，或先拖一块「告警积木」——
          最省事的做法是「汇总行」+「告警列表」+「详情按钮」。
        </div>
      ) : null}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* 子节点列表（含落点条）                                              */
/* ------------------------------------------------------------------ */

interface ListProps {
  card: CardJson;
  catalog: Catalog;
  parentPath: NodePath;
  parentTag: string | null;
  selected: NodePath | null;
  onSelect: (path: NodePath) => void;
  onChange: (card: CardJson, label: string) => void;
  layout?: 'vertical' | 'columns';
}

function ChildrenList({ layout = 'vertical', ...props }: ListProps): JSX.Element {
  const { card, catalog, parentPath } = props;
  const list = childrenAt(card, catalog, parentPath);
  return (
    <div className={layout === 'columns' ? 'cb-columns' : 'cb-children'}>
      <DropSlot {...props} index={0} />
      {list.map((_child, index) => (
        <div key={`${parentPath.join('.')}#${index}`} className="cb-item">
          <NodeView {...props} path={[...parentPath, index]} />
          <DropSlot {...props} index={index + 1} />
        </div>
      ))}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* 落点条                                                              */
/* ------------------------------------------------------------------ */

interface SlotProps extends Omit<ListProps, 'layout'> {
  index: number;
}

function DropSlot({
  card,
  catalog,
  parentPath,
  parentTag,
  index,
  onChange,
}: SlotProps): JSX.Element {
  const [active, setActive] = useState(false);

  const accepts = (payload: DragPayload): boolean => {
    if (payload.kind === 'move') {
      const from = payload.path;
      if (!from || from.length === 0) return false;
      if (isDescendant(parentPath, from)) return false; // 不能拖进自己的子孙
      const parent = from.slice(0, -1);
      const noop = samePath(parent, parentPath) && from[from.length - 1] === index;
      const afterNoop = samePath(parent, parentPath) && from[from.length - 1] === index - 1;
      return !noop && !afterNoop;
    }
    const nodes = payload.nodes ?? [];
    if (nodes.length === 0) return false;
    const allowed = (parentTag ? catalog.get(parentTag)?.children : undefined) ?? [];
    if (allowed.length === 0) return true;
    return nodes.every((node) => typeof node.tag === 'string' && allowed.includes(node.tag));
  };

  const drop = (event: React.DragEvent) => {
    event.preventDefault();
    event.stopPropagation();
    setActive(false);
    const payload = currentDrag;
    currentDrag = null;
    if (!payload || !accepts(payload)) return;

    if (payload.kind === 'move' && payload.path) {
      onChange(moveNode(card, catalog, payload.path, parentPath, index), '移动组件');
      return;
    }
    const nodes = (payload.nodes ?? []).map((node) => withElementIds(card, node));
    let next = card;
    nodes.forEach((node, offset) => {
      next = insertNode(next, catalog, parentPath, index + offset, node);
    });
    onChange(next, '添加组件');
  };

  return (
    <div
      className={`cb-slot${active ? ' active' : ''}`}
      onDragEnter={(event) => {
        if (currentDrag && accepts(currentDrag)) {
          event.preventDefault();
          setActive(true);
        }
      }}
      onDragOver={(event) => {
        if (currentDrag && accepts(currentDrag)) {
          event.preventDefault();
          event.dataTransfer.dropEffect = currentDrag.kind === 'new' ? 'copy' : 'move';
          setActive(true);
        }
      }}
      onDragLeave={() => setActive(false)}
      onDrop={drop}
      aria-label="拖放到此处"
    />
  );
}

/** 递归补齐 element_id（容器/交互组件才需要，且必须全局唯一）。 */
export function withElementIds(
  card: CardJson,
  node: Record<string, unknown>,
): Record<string, unknown> {
  const copy = deepClone(node);
  const reserved = new Set<string>();
  const assign = (current: unknown) => {
    if (!isPlainObject(current)) return;
    const tag = typeof current.tag === 'string' ? current.tag : '';
    if (tag && ID_TAGS.includes(tag)) {
      const existing = typeof current.element_id === 'string' ? current.element_id : '';
      if (!existing || reserved.has(existing)) {
        current.element_id = nextElementId(card, tag, reserved);
      }
      reserved.add(current.element_id as string);
    }
    childList(current).forEach(assign);
  };
  assign(copy);
  return copy;
}

/* ------------------------------------------------------------------ */
/* 节点视图                                                            */
/* ------------------------------------------------------------------ */

interface NodeProps extends Omit<ListProps, 'layout'> {
  path: NodePath;
}

function NodeView({
  path,
  card,
  catalog,
  selected,
  onSelect,
  onChange,
}: NodeProps): JSX.Element {
  const node = nodeAt(card, path);
  if (!isPlainObject(node)) {
    return <div className="cb-node unknown">非法节点：不是 JSON 对象</div>;
  }
  const tag = typeof node.tag === 'string' ? node.tag : '';
  const meta: CardComponentMeta | null = catalog.get(tag);
  const isSelected = selected !== null && samePath(selected, path);

  const siblings = childrenAt(card, catalog, path.slice(0, -1)).length;
  const move = (delta: number) => {
    const target = path[path.length - 1] + delta;
    if (target < 0 || target >= siblings) return;
    onChange(moveNode(card, catalog, path, path.slice(0, -1), target), '调整顺序');
  };

  const duplicate = () => {
    const copy = withElementIds(card, deepClone(node));
    onChange(insertNode(card, catalog, path.slice(0, -1), path[path.length - 1] + 1, copy), '复制组件');
  };

  const head = (
    <div
      className="cb-node-head"
      draggable
      onDragStart={(event) => beginDrag({ kind: 'move', path }, event)}
      title="按住拖动可移动位置"
    >
      <span className="cb-tag">{meta?.label ?? (tag || '未命名')}</span>
      {typeof node.element_id === 'string' && node.element_id ? (
        <code className="cb-element-id">{node.element_id}</code>
      ) : null}
      <span className="cb-spacer" />
      <button type="button" className="cb-mini" title="上移" onClick={(e) => { e.stopPropagation(); move(-1); }}>↑</button>
      <button type="button" className="cb-mini" title="下移" onClick={(e) => { e.stopPropagation(); move(1); }}>↓</button>
      <button type="button" className="cb-mini" title="复制" onClick={(e) => { e.stopPropagation(); duplicate(); }}>⧉</button>
      <button
        type="button"
        className="cb-mini danger"
        title="删除"
        onClick={(e) => {
          e.stopPropagation();
          onChange(removeNode(card, catalog, path), '删除组件');
        }}
      >
        ✕
      </button>
    </div>
  );

  const shell = (body: JSX.Element, extraClass = '') => (
    <div
      className={['cb-node', isSelected ? 'selected' : '', extraClass].filter(Boolean).join(' ')}
      onClick={(event) => {
        event.stopPropagation();
        onSelect(path);
      }}
    >
      {head}
      {body}
    </div>
  );

  if (!meta) {
    return shell(
      <>
        <div className="cb-hint warn">
          未知组件 <code>tag={tag}</code>：不支持可视化编辑，请到「JSON」页签修改（内容不会被丢弃）
        </div>
        <pre className="cb-raw">{JSON.stringify(node, null, 2)}</pre>
      </>,
      'unknown',
    );
  }

  if (meta.deprecated) {
    return shell(
      <>
        <div className="cb-hint danger">{meta.deprecated}</div>
        <pre className="cb-raw">{JSON.stringify(node, null, 2)}</pre>
      </>,
      'deprecated',
    );
  }

  const childrenKey = meta.children_key;
  return shell(
    childrenKey ? (
      <div className="cb-node-body">
        {childrenKey === 'columns' ? (
          <>
            <ChildrenList
              card={card}
              catalog={catalog}
              parentPath={path}
              parentTag={tag}
              selected={selected}
              onSelect={onSelect}
              onChange={onChange}
              layout="columns"
            />
            <button
              type="button"
              className="cb-add-column"
              onClick={(e) => {
                e.stopPropagation();
                const column = catalog.get('column');
                if (!column) return;
                const newColumn = withElementIds(card, JSON.parse(column.default_json) as Record<string, unknown>);
                onChange(
                  insertNode(card, catalog, path, childList(node).length, newColumn),
                  '添加列',
                );
              }}
            >
              + 增加一列
            </button>
          </>
        ) : (
          <ChildrenList
            card={card}
            catalog={catalog}
            parentPath={path}
            parentTag={tag}
            selected={selected}
            onSelect={onSelect}
            onChange={onChange}
          />
        )}
      </div>
    ) : (
      <NodePreview node={node} />
    ),
  );
}

/** 叶组件的近似预览（真实渲染以飞书为准，这里只求「看得懂」）。 */
function NodePreview({ node }: { node: Record<string, unknown> }): JSX.Element {
  const tag = typeof node.tag === 'string' ? node.tag : '';
  if (tag === 'hr') return <hr className="cb-hr" />;

  const text = (() => {
    if (tag === 'markdown') return typeof node.content === 'string' ? node.content : '';
    if (tag === 'div' || tag === 'plain_text') {
      if (isPlainObject(node.text) && typeof node.text.content === 'string') return node.text.content;
      return typeof node.content === 'string' ? node.content : '';
    }
    if (tag === 'button') {
      const label = isPlainObject(node.text) && typeof node.text.content === 'string' ? node.text.content : '';
      const url = Array.isArray(node.behaviors)
        ? node.behaviors
            .map((b) => (isPlainObject(b) && typeof b.default_url === 'string' ? b.default_url : ''))
            .find(Boolean)
        : '';
      return `[ ${label || '按钮'} ]${url ? ` → ${url}` : ''}`;
    }
    return '';
  })();

  return (
    <div className={`cb-preview cb-preview-${tag}`}>
      {text ? <span className="cb-preview-text">{text}</span> : <span className="faint">（空）</span>}
    </div>
  );
}

/** 供检查器使用：只读地取节点（导出以便复用）。 */
export function selectedNode(card: CardJson, path: NodePath | null): Record<string, unknown> | null {
  return path ? nodeAt(card, path) : null;
}

/** 供检查器使用：改写节点属性。 */
export function updateNode(
  card: CardJson,
  catalog: Catalog,
  path: NodePath,
  updater: (node: Record<string, unknown>) => Record<string, unknown>,
): CardJson {
  return patchNode(card, catalog, path, updater);
}
