/**
 * 本地卡片构建工具（P0）。
 *
 * 三栏：**组件面板 / 告警积木** —— **画布** —— **属性面板**。
 * 文档就是 CardKit JSON 2.0，`onChange` 直接把新 JSON 交给上层（编辑器页保存/预览/发送）。
 *
 * 交互要点：
 * - 拖拽走 `@dnd-kit/core`（指针拖拽 + 半透明浮层），落点由画布的几何计算决定；
 * - 画布顶部是**独立的卡片标题组件**，底部是**卡片设置的明显入口**；
 * - 左侧积木被识别后，画布节点仍显示积木名，左右一一对应。
 *
 * 撤销/重做用快照栈实现（卡片 ≤30KB，实现简单且不会出现半改状态）。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { DndContext, DragOverlay } from '@dnd-kit/core';
import type { CardJson, Meta } from '../types';
import { CardCanvas, stripElementIds, withElementIdsBatch } from './Canvas';
import { Inspector } from './Inspector';
import { Palette } from './Palette';
import { useCardDrag } from './drag';
import { makePresetIndex } from './presets';
import {
  analyzeCard,
  canDropPayload,
  cardBytes,
  childrenAt,
  collectTags,
  collectVariables,
  countElements,
  MAX_BYTES,
  MAX_CARD_ELEMENTS,
  estimatedRenderedElements,
  insertNode,
  isInsideLoop,
  isPlainObject,
  isInsideLoopBody,
  makeCatalog,
  moveNode,
  nodeAt,
  patchNode,
  pathAfterRemoval,
  removeNode,
  replaceNode,
  samePath,
  usesDirectives,
  type DragPayload,
  type NodePath,
  type Selection,
} from './model';

const HISTORY_LIMIT = 50;
const MAX_ELEMENTS = MAX_CARD_ELEMENTS;

export interface BuilderProps {
  card: CardJson;
  meta: Meta;
  /** 内置默认卡片（`GET /card-templates/default`），用于「载入起步卡片」。 */
  starter: CardJson | null;
  /** 编辑器元数据里的节点名称（key = element_id）；由页面持有并随模板落库。 */
  labels?: Record<string, string>;
  onLabelsChange?: (labels: Record<string, string>) => void;
  onChange: (card: CardJson) => void;
}

/**
 * 删除节点后把选中路径挪到正确的节点上。
 *
 * 实现与 `moveNode` 共用的 `pathAfterRemoval`（model.ts）是同一个函数：
 * 保持单一实现，避免两处逻辑漂移。
 */
export const remapPathAfterRemoval = pathAfterRemoval;

/** 在 `at` 处插入了一个节点后，`at` 之后（含）的兄弟整体后移一位。 */
export function remapPathAfterInsertion(selected: NodePath, at: NodePath): NodePath {
  const depth = at.length - 1;
  if (selected.length <= depth) return selected;
  for (let i = 0; i < depth; i += 1) {
    if (selected[i] !== at[i]) return selected;
  }
  if (selected[depth] >= at[depth]) {
    const next = [...selected];
    next[depth] += 1;
    return next;
  }
  return selected;
}

export function CardBuilder({
  card,
  meta,
  starter,
  labels: labelsProp,
  onLabelsChange,
  onChange,
}: BuilderProps): JSX.Element {
  const labels = labelsProp ?? {};
  const catalog = useMemo(
    () => makeCatalog(Array.isArray(meta.card_components) ? meta.card_components : []),
    [meta.card_components],
  );
  const presets = useMemo(
    () => (Array.isArray(meta.card_presets) ? meta.card_presets : []),
    [meta.card_presets],
  );
  const presetIndex = useMemo(() => makePresetIndex(presets), [presets]);

  const [selection, setSelection] = useState<Selection>({ kind: 'card' });
  const [past, setPast] = useState<CardJson[]>([]);
  const [future, setFuture] = useState<CardJson[]>([]);
  const canvasRef = useRef<HTMLDivElement>(null);

  /** 所有修改都经过这里：入历史栈 + 清空重做栈。 */
  const apply = useCallback(
    (next: CardJson, _label: string) => {
      setPast((history) => [...history, card].slice(-HISTORY_LIMIT));
      setFuture([]);
      onChange(next);
    },
    [card, onChange],
  );

  const undo = useCallback(() => {
    if (past.length === 0) return;
    const previous = past[past.length - 1];
    setPast(past.slice(0, -1));
    setFuture([card, ...future].slice(0, HISTORY_LIMIT));
    onChange(previous);
  }, [past, future, card, onChange]);

  const redo = useCallback(() => {
    if (future.length === 0) return;
    const [next, ...rest] = future;
    setPast([...past, card].slice(-HISTORY_LIMIT));
    setFuture(rest);
    onChange(next);
  }, [past, future, card, onChange]);

  // 撤销/重做快捷键（输入框内不劫持浏览器的原生撤销）
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== 'z') return;
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName?.toLowerCase();
      if (tag === 'input' || tag === 'textarea' || tag === 'select') return;
      event.preventDefault();
      if (event.shiftKey) redo();
      else undo();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [undo, redo]);

  /**
   * 节点名称的读写。
   *
   * 名称存在编辑器元数据里，key 是节点的 `element_id`——所以节点被移动 / 复制 /
   * 编辑内容之后，名字依然跟着它；这正好补上「积木靠结构指纹认人、一改就失忆」的短板。
   * 用 ref 读取最新值，避免把 labels 塞进 handleDrop 的依赖里。
   */
  const labelsRef = useRef(labels);
  labelsRef.current = labels;
  const rememberLabels = useCallback(
    (entries: Record<string, string | null>) => {
      if (!onLabelsChange) return;
      const next = { ...labelsRef.current };
      let changed = false;
      Object.entries(entries).forEach(([id, name]) => {
        if (!name) {
          if (next[id] !== undefined) {
            delete next[id];
            changed = true;
          }
          return;
        }
        if (next[id] !== name) {
          next[id] = name;
          changed = true;
        }
      });
      if (changed) onLabelsChange(next);
    },
    [onLabelsChange],
  );

  /** 记下某个节点（列表里的第 index 个）的名称，key 是它的 element_id。 */
  const rememberNodeLabel = useCallback(
    (nodes: Record<string, unknown>[], index: number, name: string) => {
      const node = nodes[index];
      if (!isPlainObject(node) || typeof node.element_id !== 'string' || !node.element_id) return;
      rememberLabels({ [node.element_id]: name });
    },
    [rememberLabels],
  );

  /** 给节点改名；节点没有 `element_id` 时先补一个（循环体内不分配 id）。 */
  const renameNode = useCallback(
    (path: NodePath, name: string) => {
      const node = nodeAt(card, path);
      if (!isPlainObject(node)) return;
      const trimmed = name.trim();
      const existing = typeof node.element_id === 'string' ? node.element_id : '';
      if (existing) {
        rememberLabels({ [existing]: trimmed });
        return;
      }
      if (isInsideLoop(card, path)) return;
      const prepared = withElementIdsBatch(card, [node])[0];
      const id = isPlainObject(prepared) && typeof prepared.element_id === 'string'
        ? prepared.element_id
        : '';
      if (!id) return;
      apply(patchNode(card, catalog, path, () => prepared), '命名组件');
      rememberLabels({ [id]: trimmed });
    },
    [apply, card, catalog, rememberLabels],
  );

  // 卡片被外部改写（例如切到 JSON 页签编辑）后，选中路径可能失效
  useEffect(() => {
    if (selection.kind === 'node' && !nodeAt(card, selection.path)) setSelection({ kind: 'card' });
  }, [card, selection]);

  /** 面板点击（不拖拽）：追加到卡片末尾并选中第一个新节点。 */
  const append = (nodes: Record<string, unknown>[], label: string) => {
    const prepared = withElementIdsBatch(card, nodes);
    rememberNodeLabel(prepared, 0, label);
    let next = card;
    let index = childrenAt(next, catalog, []).length;
    prepared.forEach((node) => {
      next = insertNode(next, catalog, [], index, node);
      index += 1;
    });
    apply(next, label);
    setSelection({ kind: 'node', path: [childrenAt(card, catalog, []).length] });
  };

  /** 拖拽落点：移动已有节点，或从面板插入新节点。 */
  const handleDrop = useCallback(
    (payload: DragPayload, parentPath: NodePath, index: number) => {
      if (payload.kind === 'move' && payload.path) {
        const from = payload.path;
        const sameParent = samePath(from.slice(0, -1), parentPath);
        // 摘掉源节点后，后面的兄弟容器会前移：选中项要用修正后的容器路径
        const destParent = sameParent
          ? parentPath
          : remapPathAfterRemoval(parentPath, from) ?? parentPath;
        const target = sameParent && from[from.length - 1] < index ? index - 1 : index;
        let next = moveNode(card, catalog, from, parentPath, index);
        // 拖进循环体：模板会复制多份，id 必然重复 → 就地剥掉（与拖入新节点一致）
        if (isInsideLoopBody(card, parentPath)) {
          next = patchNode(next, catalog, [...destParent, target], (node) => {
            const copy = { ...node };
            stripElementIds(copy);
            return copy;
          });
        }
        apply(next, '移动组件');
        setSelection({ kind: 'node', path: [...destParent, target] });
        return;
      }
      // 拖进循环体就不要 id：模板会被复制多份，id 必然重复
      const nodes = withElementIdsBatch(
        card,
        payload.nodes ?? [],
        isInsideLoopBody(card, parentPath),
      );
      if (nodes.length === 0) return;
      if (payload.label) rememberNodeLabel(nodes, 0, payload.label);
      let next = card;
      nodes.forEach((node, offset) => {
        next = insertNode(next, catalog, parentPath, index + offset, node);
      });
      apply(next, '添加组件');
      setSelection({ kind: 'node', path: [...parentPath, index] });
    },
    [apply, card, catalog],
  );

  /** 删除节点，并把选中项一起挪到仍然正确的位置。 */
  const deleteNode = useCallback(
    (path: NodePath) => {
      const removed = nodeAt(card, path);
      if (isPlainObject(removed) && typeof removed.element_id === 'string') {
        rememberLabels({ [removed.element_id]: null });
      }
      apply(removeNode(card, catalog, path), '删除组件');
      setSelection((current) => {
        if (current.kind !== 'node') return current;
        const next = remapPathAfterRemoval(current.path, path);
        return next ? { kind: 'node', path: next } : { kind: 'card' };
      });
    },
    [apply, card, catalog, rememberLabels],
  );

  /** 复制节点：副本紧跟其后，选中项之后的下标（含祖先层）要 +1。 */
  const duplicateNode = useCallback(
    (path: NodePath) => {
      const node = nodeAt(card, path);
      if (!node) return;
      const parent = path.slice(0, -1);
      const index = path[path.length - 1] + 1;
      const copy = withElementIdsBatch(card, [node], isInsideLoop(card, path))[0];
      // 名称跟着副本走（副本换了 id，所以在复制后按新 id 再记一次）
      const sourceName =
        typeof node.element_id === 'string' ? labelsRef.current[node.element_id] : undefined;
      apply(insertNode(card, catalog, parent, index, copy), '复制组件');
      if (sourceName) rememberNodeLabel([copy], 0, sourceName);
      setSelection((current) => {
        if (current.kind !== 'node') return current;
        return { kind: 'node', path: remapPathAfterInsertion(current.path, [...parent, index]) };
      });
    },
    [apply, card, catalog, rememberLabels, rememberNodeLabel],
  );

  const canDrop = useCallback(
    (payload: DragPayload, parentPath: NodePath, index: number) =>
      canDropPayload(card, catalog, payload, parentPath, index),
    [card, catalog],
  );

  const drag = useCardDrag({ containerRef: canvasRef, canDrop, onDrop: handleDrop });

  /** 变量版积木一键升级为结构化循环。 */
  const upgradeBlock = useCallback(
    (path: NodePath, presetId: string) => {
      const nodes = presetIndex.nodesOf(presetId);
      if (!nodes || nodes.length === 0) return;
      const prepared = withElementIdsBatch(card, nodes, isInsideLoop(card, path));
      apply(replaceNode(card, catalog, path, prepared), '升级为结构化循环');
      setSelection({ kind: 'node', path });
    },
    [apply, card, catalog, presetIndex],
  );

  const elements = countElements(card);
  const analysis = analyzeCard(card, meta.variables ?? []);
  // 估算只按 `max_items` 上界算，服务端还会按数据条数 / 策略上限 / 200 组件预算收口，
  // 因此显示值裁到 200，避免红色假警报。
  const estimated = Math.min(estimatedRenderedElements(card), MAX_ELEMENTS);
  const bytes = cardBytes(card);
  const variables = collectVariables(card);
  const unknownTags = collectTags(card).filter((tag) => !catalog.get(tag));
  const catalogEmpty = catalog.all.length === 0;
  const overlayLabel = drag.payload?.label
    ? drag.payload.label
    : drag.payload?.kind === 'new'
      ? '新组件'
      : '组件';

  return (
    <div className="cb-root">
      <div className="cb-toolbar">
        <button type="button" className="btn btn-sm" onClick={undo} disabled={past.length === 0}>
          撤销 {past.length > 0 ? `(${past.length})` : ''}
        </button>
        <button type="button" className="btn btn-sm" onClick={redo} disabled={future.length === 0}>
          重做
        </button>
        <button
          type="button"
          className={`btn btn-sm${selection.kind === 'card' ? ' btn-primary' : ''}`}
          onClick={() => setSelection({ kind: 'card' })}
          title="消息摘要 / 内容方向 / 内边距"
        >
          ⚙ 卡片设置
        </button>
        <button
          type="button"
          className={`btn btn-sm${selection.kind === 'header' ? ' btn-primary' : ''}`}
          onClick={() => setSelection({ kind: 'header' })}
          title="卡片标题（画布顶部那块）"
        >
          卡片标题
        </button>
        <button
          type="button"
          className="btn btn-sm"
          disabled={!starter}
          onClick={() => starter && apply(starter, '载入起步卡片')}
          title="用后端内置的告警卡骨架覆盖当前内容"
        >
          载入起步卡片
        </button>
        <span className="cb-spacer" />
        <span className={`cb-stat${elements > MAX_ELEMENTS ? ' over' : ''}`}>
          组件 {elements}/{MAX_ELEMENTS}
        </span>
        {usesDirectives(card) ? (
          <span
            className="cb-stat"
            title="按 max_items 估算的展开上限；服务端还会按实际告警条数、策略上限与 200 组件预算收口"
          >
            展开约 {estimated}
          </span>
        ) : null}
        <span className={`cb-stat${bytes > MAX_BYTES ? ' over' : ''}`}>
          体积 {(bytes / 1024).toFixed(1)}KB/30KB
        </span>
        <span className="cb-stat">变量 {variables.length}</span>
      </div>

      {catalogEmpty ? (
        <div className="banner banner-warn">
          组件目录为空（服务端没有下发组件目录）：
          请升级后端后刷新页面。
        </div>
      ) : null}

      {unknownTags.length > 0 ? (
        <div className="banner banner-warn">
          卡片里有本工具不支持的组件：{unknownTags.map((tag) => <code key={tag}>{tag} </code>)}
          ——它们会原样保留，但只能到「JSON」页签编辑。
        </div>
      ) : null}

      {elements > MAX_ELEMENTS || bytes > MAX_BYTES ? (
        <div className="banner banner-error">
          已超出飞书限制（组件 ≤{MAX_ELEMENTS}、体积 ≤30KB），保存会被拒绝。
        </div>
      ) : null}

      {analysis.issues.length > 0 ? (
        <div className="cb-diagnostics">
          <div className="cb-diagnostics-head">
            <span className="cb-panel-title mb-0">诊断</span>
            <span className="cb-stat">{analysis.issues.length}</span>
            <span className="faint small">点击可直接定位到出问题的组件</span>
          </div>
          <ul>
            {analysis.issues.map((issue, index) => (
              <li
                key={`${issue.path?.join('.') ?? 'card'}#${index}`}
                className={`cb-diagnostics-item ${issue.severity}`}
              >
                {issue.path ? (
                  <button
                    type="button"
                    onClick={() => setSelection({ kind: 'node', path: issue.path as NodePath })}
                  >
                    {issue.message}
                  </button>
                ) : (
                  <span>{issue.message}</span>
                )}
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <DndContext
        sensors={drag.sensors}
        onDragStart={drag.onDragStart}
        onDragEnd={drag.onDragEnd}
        onDragCancel={drag.onDragCancel}
      >
        <div className="cb-grid">
          <Palette
            card={card}
            catalog={catalog}
            presets={presets}
            labels={labels}
            selection={selection}
            onSelect={setSelection}
            onAdd={append}
            shouldIgnoreClick={drag.shouldIgnoreClick}
          />
          <CardCanvas
            ref={canvasRef}
            card={card}
            catalog={catalog}
            presetIndex={presetIndex}
            labels={labels}
            selection={selection}
            onSelect={setSelection}
            onChange={apply}
            onUpgradeBlock={upgradeBlock}
            onDeleteNode={deleteNode}
            onDuplicateNode={duplicateNode}
            dropTarget={drag.target}
            shouldIgnoreClick={drag.shouldIgnoreClick}
          />
          <Inspector
            card={card}
            catalog={catalog}
            meta={meta}
            selection={selection}
            labels={labels}
            onRenameNode={
              onLabelsChange && selection.kind === 'node'
                ? (name) => renameNode(selection.path, name)
                : undefined
            }
            onChange={apply}
          />
        </div>
        <DragOverlay dropAnimation={null} className="cb-drag-overlay">
          {drag.payload ? (
            <div className="cb-drag-ghost">
              <span className="cb-drag-ghost-label">{overlayLabel}</span>
              <span className="cb-drag-ghost-hint">
                {drag.target ? '松开即放下' : '拖到画布里的组件之间'}
              </span>
            </div>
          ) : null}
        </DragOverlay>
      </DndContext>
    </div>
  );
}
