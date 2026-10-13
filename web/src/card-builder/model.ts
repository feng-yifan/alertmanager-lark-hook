/**
 * 本地卡片构建工具的文档模型。
 *
 * **文档就是 CardKit JSON 2.0 本身**——不引入第二套 DSL，因此画布、预览、
 * 实际投递三者必然一致（预览走 `POST /card-templates/preview`，发送走同一份 JSON）。
 *
 * 结构：卡片 → `body.elements[]` → 每个组件按目录声明的 `children_key`
 * （`elements` / `columns`）继续嵌套。节点路径 `NodePath` 是从 `body.elements`
 * 出发、逐层进入子数组的下标序列。
 *
 * 所有修改都返回**新对象**（深拷贝后改写）：单张卡片 ≤30KB，拷贝成本可忽略，
 * 换来的是不会出现半改状态、撤销/重做直接存快照。
 */
import type { CardComponentMeta, CardJson, CardPropMeta } from '../types';

/** 从 `body.elements` 出发的下标序列。`[]` 表示卡片根（`body.elements` 本身）。 */
export type NodePath = number[];

export function deepClone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** 组件目录索引：把服务端下发的数组包成查表结构。 */
export interface Catalog {
  all: CardComponentMeta[];
  byTag: Map<string, CardComponentMeta>;
  /** 组件子节点所在字段；叶组件返回 null。 */
  childrenKey(tag: unknown): string | null;
  get(tag: unknown): CardComponentMeta | null;
  /** 该组件能否作为 `parentTag` 的子节点（`parentTag=null` 表示卡片根）。 */
  canLive(tag: string, parentTag: string | null): boolean;
  /** 出现在组件面板里的组件。 */
  palette(): CardComponentMeta[];
}

export function makeCatalog(components: CardComponentMeta[]): Catalog {
  const byTag = new Map<string, CardComponentMeta>();
  for (const component of components) byTag.set(component.tag, component);
  const childrenKey = (tag: unknown): string | null =>
    typeof tag === 'string' ? byTag.get(tag)?.children_key ?? null : null;
  return {
    all: components,
    byTag,
    childrenKey,
    get: (tag) => (typeof tag === 'string' ? byTag.get(tag) ?? null : null),
    canLive(tag, parentTag) {
      const component = byTag.get(tag);
      if (!component) return true; // 未知组件：不阻断，只在画布上标记
      if (component.child_of.length === 0) return true;
      return parentTag !== null && component.child_of.includes(parentTag);
    },
    palette: () => components.filter((component) => component.palette),
  };
}

/** 新建节点的默认 JSON（服务端保证结构合法）。 */
export function defaultNode(component: CardComponentMeta): Record<string, unknown> {
  const parsed: unknown = JSON.parse(component.default_json);
  if (!isPlainObject(parsed)) throw new Error(`${component.tag} 的 default_json 不是对象`);
  return parsed;
}

/** 卡片根元素数组（必要时创建）。 */
export function rootElements(card: CardJson): unknown[] {
  const body = card.body;
  if (!isPlainObject(body)) return [];
  const elements = body.elements;
  return Array.isArray(elements) ? elements : [];
}

/** 按路径取节点（`[]` 不是节点，返回 null）。 */
export function nodeAt(card: CardJson, path: NodePath): Record<string, unknown> | null {
  if (path.length === 0) return null;
  return descend(card, path);
}

function descend(card: CardJson, path: NodePath): Record<string, unknown> | null {
  let node: Record<string, unknown> | null = null;
  for (const index of path) {
    const list: unknown[] = node === null ? rootElements(card) : childList(node);
    const next: unknown = list[index];
    if (!isPlainObject(next)) return null;
    node = next;
  }
  return node;
}

/** 容器节点的子数组（按目录的 children_key 取；取不到时按 `elements` 兜底）。 */
export function childList(node: Record<string, unknown>): unknown[] {
  for (const key of ['elements', 'columns']) {
    const value = node[key];
    if (Array.isArray(value)) return value;
  }
  return [];
}

/** 指定容器（`[]` = 卡片根）下的子数组；`create` 为真时补齐缺失的容器结构。 */
export function childrenAt(
  card: CardJson,
  catalog: Catalog,
  parentPath: NodePath,
  create = false,
): unknown[] {
  if (parentPath.length === 0) {
    if (!isPlainObject(card.body)) {
      if (!create) return [];
      card.body = { elements: [] };
    }
    const body = card.body as Record<string, unknown>;
    if (!Array.isArray(body.elements)) {
      if (!create) return [];
      body.elements = [];
    }
    return body.elements as unknown[];
  }

  const parent = nodeAt(card, parentPath);
  if (!parent) return [];
  const key = catalog.childrenKey(parent.tag) ?? 'elements';
  if (!Array.isArray(parent[key])) {
    if (!create) return [];
    parent[key] = [];
  }
  return parent[key] as unknown[];
}

/** 在 `parentPath` 的第 `index` 位插入节点。 */
export function insertNode(
  card: CardJson,
  catalog: Catalog,
  parentPath: NodePath,
  index: number,
  node: Record<string, unknown>,
): CardJson {
  const next = deepClone(card);
  const list = childrenAt(next, catalog, parentPath, true);
  const at = Math.max(0, Math.min(index, list.length));
  list.splice(at, 0, node);
  return next;
}

/** 删除节点。 */
export function removeNode(card: CardJson, catalog: Catalog, path: NodePath): CardJson {
  const next = deepClone(card);
  const list = childrenAt(next, catalog, path.slice(0, -1));
  list.splice(path[path.length - 1], 1);
  return next;
}

/** 把节点从 `from` 移动到 `toParent` 的第 `index` 位（同容器内也安全）。 */
export function moveNode(
  card: CardJson,
  catalog: Catalog,
  from: NodePath,
  toParent: NodePath,
  index: number,
): CardJson {
  if (from.length === 0) return card;
  const moving = nodeAt(card, from);
  if (!moving) return card;
  // 不允许把容器拖进自己的子孙里（否则结构自毁）。
  if (isDescendant(toParent, from)) return card;

  const without = removeNode(card, catalog, from);
  const fromParent = from.slice(0, -1);
  const sameParent = samePath(fromParent, toParent);
  const adjusted = sameParent && from[from.length - 1] < index ? index - 1 : index;
  return insertNode(without, catalog, toParent, adjusted, moving);
}

/** `candidate` 是否等于或位于 `ancestor` 之下。 */
export function isDescendant(candidate: NodePath, ancestor: NodePath): boolean {
  if (candidate.length < ancestor.length) return false;
  return ancestor.every((step, index) => candidate[index] === step);
}

export function samePath(a: NodePath, b: NodePath): boolean {
  return a.length === b.length && a.every((step, index) => step === b[index]);
}

/** 就地改写节点（返回新卡片）。 */
export function patchNode(
  card: CardJson,
  catalog: Catalog,
  path: NodePath,
  updater: (node: Record<string, unknown>) => Record<string, unknown>,
): CardJson {
  const next = deepClone(card);
  const list = childrenAt(next, catalog, path.slice(0, -1));
  const current = list[path[path.length - 1]];
  if (!isPlainObject(current)) return card;
  list[path[path.length - 1]] = updater(current);
  return next;
}

/* ------------------------------------------------------------------ */
/* 属性读写（按目录里的 `path` / `accessor`）                          */
/* ------------------------------------------------------------------ */

/** 读取点号路径（如 `text.content`）。 */
export function getPath(node: Record<string, unknown>, path: string): unknown {
  let current: unknown = node;
  for (const key of path.split('.')) {
    if (!isPlainObject(current)) return undefined;
    current = current[key];
  }
  return current;
}

/** 写入点号路径，逐级创建中间对象。 */
export function setPath(
  node: Record<string, unknown>,
  path: string,
  value: unknown,
): Record<string, unknown> {
  const keys = path.split('.');
  const next = { ...node };
  let cursor = next;
  for (const key of keys.slice(0, -1)) {
    const child = cursor[key];
    cursor[key] = isPlainObject(child) ? { ...child } : {};
    cursor = cursor[key] as Record<string, unknown>;
  }
  const last = keys[keys.length - 1];
  if (value === undefined) delete cursor[last];
  else cursor[last] = value;
  return next;
}

/** 按属性元数据读取值（含按钮跳转这类派生字段）。 */
export function readProp(node: Record<string, unknown>, prop: CardPropMeta): unknown {
  if (prop.accessor === 'button_url') {
    const behaviors = node.behaviors;
    if (!Array.isArray(behaviors)) return '';
    const open = behaviors.find(
      (item) => isPlainObject(item) && item.type === 'open_url',
    );
    return isPlainObject(open) ? (open.default_url as string | undefined) ?? '' : '';
  }
  return getPath(node, prop.path);
}

/** 按属性元数据写值。返回新节点；值等于空时删除该字段（保持卡片精简）。 */
export function writeProp(
  node: Record<string, unknown>,
  prop: CardPropMeta,
  value: unknown,
): Record<string, unknown> {
  if (prop.accessor === 'button_url') {
    const url = typeof value === 'string' ? value.trim() : '';
    const behaviors = (Array.isArray(node.behaviors) ? node.behaviors : []).filter(
      (item) => !(isPlainObject(item) && item.type === 'open_url'),
    );
    if (url) {
      behaviors.unshift({
        type: 'open_url',
        default_url: url,
        pc_url: '',
        ios_url: '',
        android_url: '',
      });
    }
    const next = { ...node };
    if (behaviors.length > 0) next.behaviors = behaviors;
    else delete next.behaviors;
    return next;
  }
  const empty = value === '' || value === undefined || value === null;
  return setPath(node, prop.path, empty ? undefined : value);
}

/* ------------------------------------------------------------------ */
/* element_id / 统计                                                    */
/* ------------------------------------------------------------------ */

const ID_PREFIX: Record<string, string> = {
  markdown: 'md',
  div: 'txt',
  hr: 'hr',
  button: 'btn',
  column_set: 'cols',
  column: 'col',
  collapsible_panel: 'panel',
  plain_text: 'txt',
};

/** 需要 `element_id` 的组件（容器与交互组件：局部更新/交互回调依赖它）。 */
export const ID_TAGS = ['markdown', 'div', 'button', 'column_set', 'collapsible_panel'];

/** 生成全局唯一、符合飞书校验（字母开头、仅字母数字下划线、≤20）的 element_id。 */
export function nextElementId(
  card: CardJson,
  tag: string,
  reserved: Set<string> = new Set(),
): string {
  const prefix = ID_PREFIX[tag] ?? 'el';
  const used = new Set<string>(reserved);
  walkNodes(card, (node) => {
    if (typeof node.element_id === 'string') used.add(node.element_id);
  });
  for (let i = 1; i < 10_000; i += 1) {
    const candidate = `${prefix}_${i}`;
    if (!used.has(candidate)) return candidate;
  }
  return `${prefix}_${Date.now() % 100000}`;
}

/** 深度遍历所有节点。 */
export function walkNodes(
  card: CardJson,
  visit: (node: Record<string, unknown>, path: NodePath) => void,
): void {
  const visitList = (list: unknown[], base: NodePath) => {
    list.forEach((item, index) => {
      if (!isPlainObject(item)) return;
      const path = [...base, index];
      visit(item, path);
      const children = childList(item);
      if (children.length > 0) visitList(children, path);
    });
  };
  visitList(rootElements(card), []);
}

/** 元素数量（与飞书 200 上限对齐；服务端同口径）。 */
export function countElements(card: CardJson): number {
  let count = 0;
  walkNodes(card, () => {
    count += 1;
  });
  return count;
}

/** 卡片 JSON 体积（字节，UTF-8 近似：按 JSON 字符串长度）。 */
export function cardBytes(card: CardJson): number {
  return new TextEncoder().encode(JSON.stringify(card)).length;
}

/** 收集卡片里用到的所有 `${变量}`。 */
export function collectVariables(card: CardJson): string[] {
  const found = new Set<string>();
  const scan = (value: unknown) => {
    if (typeof value === 'string') {
      for (const match of value.matchAll(/\$\{([A-Za-z0-9_.:\-]+)\}/g)) found.add(match[1]);
      return;
    }
    if (Array.isArray(value)) {
      value.forEach(scan);
      return;
    }
    if (isPlainObject(value)) Object.values(value).forEach(scan);
  };
  scan(card);
  return [...found].sort();
}

/** 汇总卡片中的 tag（画布上用不到，供「不支持的组件」提示）。 */
export function collectTags(card: CardJson): string[] {
  const tags = new Set<string>();
  walkNodes(card, (node) => {
    if (typeof node.tag === 'string') tags.add(node.tag);
  });
  return [...tags];
}
