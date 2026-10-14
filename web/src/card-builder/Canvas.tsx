/**
 * 可视化画布：**基于指针的拖拽**（`@dnd-kit/core`）+ 半透明拖拽浮层。
 *
 * 相比之前的原生 HTML5 拖拽，这里有三点不同：
 * 1. 落点由**指针所在区域**决定：落在某个子组件的上半区就插到它前面、下半区插到后面，
 *    不再要求指针精确停在两个组件之间的细缝里；
 * 2. 拖拽浮层是半透明的（`DragOverlay`），不会像原生拖影那样把蓝色落点条盖住；
 * 3. 组件标题优先显示**左侧积木的名字**（「积木识别」见 `presets.ts`），
 *    左右面板因此能一一对应；卡片标题是画布顶部一块**独立的可配置组件**。
 */
import { Fragment, forwardRef } from 'react';
import { useDraggable, useDroppable } from '@dnd-kit/core';
import type { CardComponentMeta, CardJson } from '../types';
import type { DropTarget } from './drag';
import {
  childList,
  childrenAt,
  countElements,
  deepClone,
  estimatedNodesCost,
  ID_TAGS,
  insertNode,
  isPlainObject,
  loopMaxItems,
  LOOP_TAG,
  MAX_CARD_ELEMENTS,
  moveNode,
  nextElementId,
  nodeAt,
  samePath,
  walkNodes,
  type Catalog,
  type NodePath,
  type Selection,
} from './model';
import type { PresetIndex } from './presets';

export interface CanvasProps {
  card: CardJson;
  catalog: Catalog;
  presetIndex: PresetIndex;
  /** 编辑器元数据里的节点名称（key = element_id）。 */
  labels: Record<string, string>;
  selection: Selection;
  onSelect: (selection: Selection) => void;
  onChange: (card: CardJson, label: string) => void;
  /** 把「变量版」积木节点替换成结构化循环。 */
  onUpgradeBlock: (path: NodePath, presetId: string) => void;
  onDeleteNode: (path: NodePath) => void;
  onDuplicateNode: (path: NodePath) => void;
  dropTarget: DropTarget | null;
  /** 拖拽刚结束时浏览器补发的 click 要忽略。 */
  shouldIgnoreClick: () => boolean;
}

/* ------------------------------------------------------------------ */
/* 画布外壳                                                            */
/* ------------------------------------------------------------------ */

export const CardCanvas = forwardRef<HTMLDivElement, CanvasProps>(function CardCanvas(
  {
    card,
    catalog,
    presetIndex,
    labels,
    selection,
    onSelect,
    onChange,
    onUpgradeBlock,
    onDeleteNode,
    onDuplicateNode,
    dropTarget,
    shouldIgnoreClick,
  },
  ref,
) {
  const click = (action: () => void) => () => {
    if (shouldIgnoreClick()) return;
    action();
  };

  return (
    <div className="cb-canvas" ref={ref} onClick={click(() => onSelect({ kind: 'card' }))}>
      <HeaderBlock
        card={card}
        selection={selection}
        onSelect={onSelect}
        shouldIgnoreClick={shouldIgnoreClick}
      />

      <div className="cb-body">
        <ChildrenList
          card={card}
          catalog={catalog}
          presetIndex={presetIndex}
          labels={labels}
          selection={selection}
          onSelect={onSelect}
          onChange={onChange}
          onUpgradeBlock={onUpgradeBlock}
          onDeleteNode={onDeleteNode}
          onDuplicateNode={onDuplicateNode}
          dropTarget={dropTarget}
          shouldIgnoreClick={shouldIgnoreClick}
          parentPath={[]}
          parentTag={null}
        />
        {countElements(card) === 0 ? (
          <div className="cb-empty">
            从左侧拖入组件或「告警积木」——
            推荐顺序是「汇总行」→「告警列表」（循环 + 判断 + 文本）→「详情按钮」。
          </div>
        ) : null}
      </div>

      <button
        type="button"
        className={`cb-card-entry${selection.kind === 'card' ? ' selected' : ''}`}
        onClick={(event) => {
          event.stopPropagation();
          if (shouldIgnoreClick()) return;
          onSelect({ kind: 'card' });
        }}
        title="消息摘要 / 内容方向 / 内边距等卡片级设置"
      >
        <span className="cb-card-entry-icon" aria-hidden="true">
          ⚙
        </span>
        卡片设置：消息摘要 · 内容方向 · 内边距
        <span className="cb-spacer" />
        <span className="faint small">{selection.kind === 'card' ? '正在编辑' : '点击编辑'}</span>
      </button>
    </div>
  );
});

/* ------------------------------------------------------------------ */
/* 卡片标题（独立组件）                                                */
/* ------------------------------------------------------------------ */

/** 飞书 header 模板名 → 色块颜色（仅用于画布上的近似预览）。 */
const HEADER_COLORS: Record<string, string> = {
  default: '#8f959e',
  blue: '#3370ff',
  wathet: '#4e83fd',
  turquoise: '#37c2bc',
  green: '#34c724',
  yellow: '#ffc60a',
  orange: '#ff8800',
  red: '#f54a45',
  carmine: '#f5319d',
  violet: '#7f3bf5',
  purple: '#6425d0',
  indigo: '#4f54f0',
  grey: '#8f959e',
};

function plainText(value: unknown): string {
  if (!isPlainObject(value)) return '';
  return typeof value.content === 'string' ? value.content : '';
}

function HeaderBlock({
  card,
  selection,
  onSelect,
  shouldIgnoreClick,
}: {
  card: CardJson;
  selection: Selection;
  onSelect: (selection: Selection) => void;
  shouldIgnoreClick: () => boolean;
}): JSX.Element {
  const header = isPlainObject(card.header) ? card.header : {};
  const title = plainText(header.title);
  const subtitle = plainText(header.subtitle);
  const template = typeof header.template === 'string' ? header.template : '';
  const color = HEADER_COLORS[template] ?? (template.includes('${') ? '#c9cdd4' : '');
  const selected = selection.kind === 'header';

  return (
    <div
      className={`cb-node cb-header-block${selected ? ' selected' : ''}`}
      onClick={(event) => {
        event.stopPropagation();
        if (shouldIgnoreClick()) return;
        onSelect({ kind: 'header' });
      }}
      title="卡片标题：独立的可配置组件，点这里编辑"
    >
      <div className="cb-node-head">
        <span className="cb-tag">卡片标题</span>
        <span className="cb-block-badge">独立组件</span>
        <span className="cb-spacer" />
        <span className="faint small">
          {template ? `配色 ${template}` : '默认配色'}
        </span>
      </div>
      <div className="cb-header-preview">
        <span
          className="cb-header-swatch"
          style={color ? { background: color } : undefined}
          aria-hidden="true"
        />
        <div className="cb-header-text">
          <div className="cb-header-title">{title || '（未设置标题）'}</div>
          {subtitle ? <div className="cb-header-sub">{subtitle}</div> : null}
        </div>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* 子节点列表（含落点条）                                              */
/* ------------------------------------------------------------------ */

interface TreeProps {
  card: CardJson;
  catalog: Catalog;
  presetIndex: PresetIndex;
  /** 编辑器元数据里的节点名称（key = element_id）。 */
  labels: Record<string, string>;
  selection: Selection;
  onSelect: (selection: Selection) => void;
  onChange: (card: CardJson, label: string) => void;
  onUpgradeBlock: (path: NodePath, presetId: string) => void;
  /** 删除 / 复制由上层处理：它知道怎么把选中项一起挪对位置。 */
  onDeleteNode: (path: NodePath) => void;
  onDuplicateNode: (path: NodePath) => void;
  dropTarget: DropTarget | null;
  shouldIgnoreClick: () => boolean;
}

interface ListProps extends TreeProps {
  parentPath: NodePath;
  parentTag: string | null;
  layout?: 'vertical' | 'columns';
}

function ChildrenList({ layout = 'vertical', ...props }: ListProps): JSX.Element {
  const { card, catalog, parentPath, dropTarget } = props;
  const list = childrenAt(card, catalog, parentPath);
  const activeIndex =
    dropTarget && samePath(dropTarget.parentPath, parentPath) ? dropTarget.index : null;
  // 注册为 droppable：落点由自己的几何计算决定，但 dnd-kit 需要它来驱动自动滚动。
  const { setNodeRef } = useDroppable({
    id: `cb-list:${JSON.stringify(parentPath)}`,
    data: { parentPath },
  });

  return (
    <div
      ref={setNodeRef}
      className={layout === 'columns' ? 'cb-columns' : 'cb-children'}
      data-cb-list={JSON.stringify(parentPath)}
      data-cb-layout={layout}
    >
      {activeIndex === 0 ? <div className="cb-slot active" /> : null}
      {list.map((_child, index) => (
        <Fragment key={`${parentPath.join('.')}#${index}`}>
          <div className="cb-item" data-cb-node-index={index}>
            <NodeView {...props} path={[...parentPath, index]} />
          </div>
          {activeIndex === index + 1 ? <div className="cb-slot active" /> : null}
        </Fragment>
      ))}
      {list.length === 0 && parentPath.length > 0 ? (
        <div className="cb-children-empty">拖组件到这里</div>
      ) : null}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* 节点视图                                                            */
/* ------------------------------------------------------------------ */

interface NodeProps extends TreeProps {
  path: NodePath;
}

/** 本地指令的作用说明（画布上直接说清楚它到底在做什么）。 */
function directiveHint(tag: string, node: Record<string, unknown>): string {
  if (tag === 'repeat') {
    const source = typeof node.source === 'string' ? node.source : 'alerts';
    const max = loopMaxItems(node);
    const cost = max * estimatedNodesCost(childList(node));
    const empty = typeof node.empty_content === 'string' ? node.empty_content : '';
    return `对「${source}」逐条渲染子组件（最多 ${max} 次，展开约 ${cost} 个组件${
      empty ? `；没有数据时显示「${empty}」` : ''
    }）；子组件里用 \${item.xxx} 取当前告警`;
  }
  if (tag === 'if') {
    const when = typeof node.when === 'string' && node.when ? node.when : '（未设置条件）';
    const equals = typeof node.equals === 'string' && node.equals ? ` 等于 ${node.equals}` : '';
    const negate = node.negate === true ? '不满足 ' : '';
    return `${negate}${when}${equals} 时才渲染子组件；结果为空 / false / 0 视为不成立`;
  }
  return '';
}

function NodeView({
  path,
  card,
  catalog,
  presetIndex,
  labels,
  selection,
  onSelect,
  onChange,
  onUpgradeBlock,
  onDeleteNode,
  onDuplicateNode,
  dropTarget,
  shouldIgnoreClick,
}: NodeProps): JSX.Element {
  const node = nodeAt(card, path);
  const identity = isPlainObject(node) ? presetIndex.describe(node) : null;
  // 节点自定义名称（builder_meta，按 element_id 跟随节点）优先于积木识别
  const customName =
    isPlainObject(node) && typeof node.element_id === 'string'
      ? (labels[node.element_id] ?? '')
      : '';
  const label =
    customName ||
    (identity?.presetId || identity?.legacyPresetId
      ? identity.blockLabel
      : node && typeof node.tag === 'string'
        ? node.tag
        : '');
  const drag = useDraggable({
    id: `cb-node:${path.join('.')}`,
    data: { kind: 'move', path, label },
  });
  const isSelected =
    selection.kind === 'node' && samePath(selection.path, path);

  if (!isPlainObject(node)) {
    return <div className="cb-node unknown">非法节点：不是 JSON 对象</div>;
  }
  const tag = typeof node.tag === 'string' ? node.tag : '';
  const meta: CardComponentMeta | null = catalog.get(tag);

  const siblings = childrenAt(card, catalog, path.slice(0, -1)).length;
  /**
   * 上/下移一位。
   *
   * `moveNode` 的 `index` 是「**原列表**里的插入位」，所以下移要传 `位置 + 2`
   * （传 `+1` 会被内部调整成原地落点，等于没动）。移动后必须把选中项也挪到新位置，
   * 否则属性面板会继续编辑「恰好在同一个下标」的另一个节点。
   */
  const moveBy = (delta: -1 | 1) => {
    const at = path[path.length - 1];
    const target = at + delta;
    if (target < 0 || target >= siblings) return;
    const insertAt = delta > 0 ? at + 2 : at - 1;
    onChange(moveNode(card, catalog, path, path.slice(0, -1), insertAt), '调整顺序');
    onSelect({ kind: 'node', path: [...path.slice(0, -1), target] });
  };

  const head = (
    <div
      className="cb-node-head"
      ref={drag.setNodeRef}
      {...drag.listeners}
      title="按住拖动可移动位置"
    >
      {customName ? (
        <>
          <span className="cb-tag">{customName}</span>
          <span className="cb-tag-sub">
            {(identity?.presetId || identity?.legacyPresetId
              ? identity.blockLabel
              : meta?.label) ?? tag}
          </span>
        </>
      ) : identity?.presetId || identity?.legacyPresetId ? (
        <>
          <span className="cb-tag">{identity.blockLabel}</span>
          {identity.presetId ? (
            <span className="cb-tag-sub">{meta?.label ?? tag}</span>
          ) : null}
        </>
      ) : (
        <span className="cb-tag">{meta?.label ?? (tag || '未命名')}</span>
      )}
      {identity?.legacyPresetId ? (
        <button
          type="button"
          className="cb-mini cb-mini-wide"
          title="替换成结构化循环：能在画布上逐项修改（循环 + 判断 + 文本）"
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.stopPropagation();
            if (shouldIgnoreClick()) return;
            onUpgradeBlock(path, identity.legacyPresetId as string);
          }}
        >
          转为循环
        </button>
      ) : null}
      {typeof node.element_id === 'string' && node.element_id ? (
        <code className="cb-element-id">{node.element_id}</code>
      ) : null}
      <span className="cb-spacer" />
      <button
        type="button"
        className="cb-mini"
        title="上移"
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.stopPropagation();
          moveBy(-1);
        }}
      >
        ↑
      </button>
      <button
        type="button"
        className="cb-mini"
        title="下移"
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.stopPropagation();
          moveBy(1);
        }}
      >
        ↓
      </button>
      <button
        type="button"
        className="cb-mini"
        title="复制"
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.stopPropagation();
          onDuplicateNode(path);
        }}
      >
        ⧉
      </button>
      <button
        type="button"
        className="cb-mini danger"
        title="删除"
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.stopPropagation();
          onDeleteNode(path);
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
        if (shouldIgnoreClick()) return;
        onSelect({ kind: 'node', path });
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
  const directive = meta.group === 'directive';
  return shell(
    childrenKey ? (
      <div className="cb-node-body">
        {directive ? (
          <div
            className={`cb-hint directive${
              tag === LOOP_TAG &&
              loopMaxItems(node) * estimatedNodesCost(childList(node)) > MAX_CARD_ELEMENTS
                ? ' danger'
                : ''
            }`}
          >
            {directiveHint(tag, node)}
          </div>
        ) : null}
        {childrenKey === 'columns' ? (
          <>
            <ChildrenList
              card={card}
              catalog={catalog}
              presetIndex={presetIndex}
              labels={labels}
              selection={selection}
              onSelect={onSelect}
              onChange={onChange}
              onUpgradeBlock={onUpgradeBlock}
              onDeleteNode={onDeleteNode}
              onDuplicateNode={onDuplicateNode}
              dropTarget={dropTarget}
              shouldIgnoreClick={shouldIgnoreClick}
              parentPath={path}
              parentTag={tag}
              layout="columns"
            />
            <button
              type="button"
              className="cb-add-column"
              onPointerDown={(event) => event.stopPropagation()}
              onClick={(event) => {
                event.stopPropagation();
                const column = catalog.get('column');
                if (!column) return;
                const newColumn = withElementIds(
                  card,
                  JSON.parse(column.default_json) as Record<string, unknown>,
                );
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
            presetIndex={presetIndex}
            labels={labels}
            selection={selection}
            onSelect={onSelect}
            onChange={onChange}
            onUpgradeBlock={onUpgradeBlock}
            onDeleteNode={onDeleteNode}
            onDuplicateNode={onDuplicateNode}
            dropTarget={dropTarget}
            shouldIgnoreClick={shouldIgnoreClick}
            parentPath={path}
            parentTag={tag}
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

/**
 * 递归删掉 `element_id`（含数组与嵌套对象）。
 *
 * 循环体的子组件会被复制成多份，里面的 id 必然重复 —— 保存时校验就会报
 * `element_id 重复`，发送时飞书返回 `300301`。所以循环体内**不留 id**。
 */
export function stripElementIds(value: unknown): void {
  if (Array.isArray(value)) {
    value.forEach(stripElementIds);
    return;
  }
  if (!isPlainObject(value)) return;
  delete value.element_id;
  Object.values(value).forEach(stripElementIds);
}

/**
 * 递归补齐 element_id（容器/交互组件才需要，且必须全局唯一）。
 *
 * 两条规则：
 * 1. 卡片里已经存在的 id **必须重新分配**：复制节点、重复拖入同一个积木
 *    （例如两次「汇总行」都带 `element_id: summary`）都会撞车，服务端会直接拒收。
 * 2. **循环体内部不留 id**：循环会把模板复制 N 份，模板里的 id 必然重复，
 *    保存校验会报「element_id 重复」，发送时飞书返回 `300301` 拒收整张卡片。
 *    `insideLoop` 为真（拖进循环体）时直接剥掉全部 id；节点本身是 `repeat` 时
 *    保留它自己的 id、剥掉整棵子树。服务端展开时还会再剥一次，双保险。
 */
export function withElementIds(
  card: CardJson,
  node: Record<string, unknown>,
  reservedIds?: Set<string>,
  insideLoop = false,
): Record<string, unknown> {
  const copy = deepClone(node);
  if (insideLoop) {
    stripElementIds(copy);
    return copy;
  }
  const reserved = reservedIds ?? new Set<string>();
  const usedInCard = new Set<string>();
  walkNodes(card, (current) => {
    if (typeof current.element_id === 'string') usedInCard.add(current.element_id);
  });
  const assign = (current: unknown) => {
    if (!isPlainObject(current)) return;
    const tag = typeof current.tag === 'string' ? current.tag : '';
    if (tag && ID_TAGS.includes(tag)) {
      const existing = typeof current.element_id === 'string' ? current.element_id : '';
      if (!existing || reserved.has(existing) || usedInCard.has(existing)) {
        current.element_id = nextElementId(card, tag, reserved);
      }
      reserved.add(current.element_id as string);
    }
    if (tag === 'repeat') {
      // 循环体内不留 id（含用户先前拖进去的组件）
      childList(current).forEach(stripElementIds);
      return;
    }
    childList(current).forEach(assign);
  };
  assign(copy);
  return copy;
}

/** 一次插入多个节点：共用一份 reserved，避免彼此撞 id。 */
export function withElementIdsBatch(
  card: CardJson,
  nodes: Record<string, unknown>[],
  insideLoop = false,
): Record<string, unknown>[] {
  const reserved = new Set<string>();
  return nodes.map((node) => withElementIds(card, node, reserved, insideLoop));
}

/** 供检查器使用：只读地取节点（导出以便复用）。 */
export function selectedNode(card: CardJson, path: NodePath | null): Record<string, unknown> | null {
  return path ? nodeAt(card, path) : null;
}
