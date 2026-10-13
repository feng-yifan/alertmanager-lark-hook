/**
 * 本地卡片构建工具（P0）。
 *
 * 三栏：**组件面板 / 告警积木** —— **画布** —— **属性面板**。
 * 文档就是 CardKit JSON 2.0，`onChange` 直接把新 JSON 交给上层（编辑器页保存/预览/发送）。
 *
 * 撤销/重做用快照栈实现（卡片 ≤30KB，实现简单且不会出现半改状态）。
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { CardJson, Meta } from '../types';
import { CardCanvas, withElementIds } from './Canvas';
import { Inspector } from './Inspector';
import { Palette } from './Palette';
import {
  cardBytes,
  childrenAt,
  collectTags,
  collectVariables,
  countElements,
  insertNode,
  makeCatalog,
  nodeAt,
  type NodePath,
} from './model';

/** 与飞书硬限制对齐（服务端 `card_catalog` 同口径）。 */
const MAX_ELEMENTS = 200;
const MAX_BYTES = 30 * 1024;
const HISTORY_LIMIT = 50;

export interface BuilderProps {
  card: CardJson;
  meta: Meta;
  /** 内置默认卡片（`GET /card-templates/default`），用于「载入起步卡片」。 */
  starter: CardJson | null;
  onChange: (card: CardJson) => void;
}

export function CardBuilder({ card, meta, starter, onChange }: BuilderProps): JSX.Element {
  const catalog = useMemo(
    () => makeCatalog(Array.isArray(meta.card_components) ? meta.card_components : []),
    [meta.card_components],
  );
  const [selected, setSelected] = useState<NodePath | null>(null);
  const [past, setPast] = useState<CardJson[]>([]);
  const [future, setFuture] = useState<CardJson[]>([]);

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

  // 卡片被外部改写（例如切到 JSON 页签编辑）后，选中路径可能失效
  useEffect(() => {
    if (selected && !nodeAt(card, selected)) setSelected(null);
  }, [card, selected]);

  const append = (nodes: Record<string, unknown>[], label: string) => {
    const prepared = nodes.map((node) => withElementIds(card, node));
    let next = card;
    let index = childrenAt(next, catalog, []).length;
    prepared.forEach((node) => {
      next = insertNode(next, catalog, [], index, node);
      index += 1;
    });
    apply(next, label);
  };

  const elements = countElements(card);
  const bytes = cardBytes(card);
  const variables = collectVariables(card);
  const unknownTags = collectTags(card).filter((tag) => !catalog.get(tag));
  const catalogEmpty = catalog.all.length === 0;

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
        <span className={`cb-stat${bytes > MAX_BYTES ? ' over' : ''}`}>
          体积 {(bytes / 1024).toFixed(1)}KB/30KB
        </span>
        <span className="cb-stat">变量 {variables.length}</span>
      </div>

      {catalogEmpty ? (
        <div className="banner banner-warn">
          组件目录为空（<code>GET /meta</code> 未返回 <code>card_components</code>）：
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

      <div className="cb-grid">
        <Palette
          catalog={catalog}
          presets={Array.isArray(meta.card_presets) ? meta.card_presets : []}
          onAdd={append}
        />
        <CardCanvas
          card={card}
          catalog={catalog}
          selected={selected}
          onSelect={setSelected}
          onChange={apply}
        />
        <Inspector
          card={card}
          catalog={catalog}
          meta={meta}
          path={selected}
          onChange={apply}
        />
      </div>
    </div>
  );
}
