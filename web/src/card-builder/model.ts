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
import type { CardComponentMeta, CardJson, CardPropMeta, MetaVariable } from '../types';

/** 从 `body.elements` 出发的下标序列。`[]` 表示卡片根（`body.elements` 本身）。 */
export type NodePath = number[];

/** 与飞书硬限制对齐（服务端 `card_catalog` 同口径）。 */
export const MAX_CARD_ELEMENTS = 200;
export const MAX_BYTES = 30 * 1024;

/**
 * 画布上的选中项：
 * - `card`：卡片级设置（摘要 / 方向 / 内边距）；
 * - `header`：卡片标题（独立组件，画布顶部一块）；
 * - `node`：某个组件节点。
 */
export type Selection = { kind: 'card' } | { kind: 'header' } | { kind: 'node'; path: NodePath };

/** 画布拖拽载荷：从面板拖入（new）或移动已有节点（move）。 */
export interface DragPayload {
  kind: 'new' | 'move';
  nodes?: Record<string, unknown>[];
  path?: NodePath;
  /** 拖拽浮层上显示的文案（积木名 / 组件名）。 */
  label?: string;
}

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

  const fromParent = from.slice(0, -1);
  const sameParent = samePath(fromParent, toParent);
  // ⚠️ 源节点被摘掉后，位于它**后面**的兄弟子树会整体前移，目标容器路径必须跟着修正。
  // 否则 `insertNode` 在摘除后的卡片里找不到目标容器（`childrenAt` 返回临时数组），
  // 节点会被静默丢弃——「把第一个容器拖进第二个容器」就会触发。
  const destParent = sameParent ? toParent : pathAfterRemoval(toParent, from);
  if (destParent === null) return card;

  const without = removeNode(card, catalog, from);
  const adjusted =
    sameParent && from[from.length - 1] < index ? index - 1 : index;
  return insertNode(without, catalog, destParent, adjusted, moving);
}

/**
 * 摘掉 `removed` 之后，`path` 指向的位置要挪到哪儿。
 *
 * `path` 位于 `removed` 之内时返回 `null`（该位置已不存在）。
 */
export function pathAfterRemoval(path: NodePath, removed: NodePath): NodePath | null {
  if (isDescendant(path, removed)) return null;
  const depth = removed.length - 1;
  if (path.length <= depth) return path; // removed 在更深的另一个分支里
  for (let i = 0; i < depth; i += 1) {
    if (path[i] !== removed[i]) return path; // 祖先层就分叉了
  }
  if (path[depth] > removed[depth]) {
    const next = [...path];
    next[depth] -= 1;
    return next;
  }
  return path;
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
  if (prop.accessor === 'button_callback') {
    const behaviors = node.behaviors;
    if (!Array.isArray(behaviors)) return '';
    const cb = behaviors.find((item) => isPlainObject(item) && item.type === 'callback');
    if (!isPlainObject(cb) || cb.value === undefined) return '';
    return typeof cb.value === 'string' ? cb.value : JSON.stringify(cb.value);
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
  if (prop.accessor === 'button_callback') {
    const raw = typeof value === 'string' ? value.trim() : '';
    const behaviors = (Array.isArray(node.behaviors) ? node.behaviors : []).filter(
      (item) => !(isPlainObject(item) && item.type === 'callback'),
    );
    if (raw) {
      let parsed: unknown = raw;
      try {
        parsed = JSON.parse(raw);
      } catch {
        parsed = raw;
      }
      behaviors.push({ type: 'callback', value: parsed });
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
  repeat: 'loop',
  if: 'cond',
};

/**
 * 需要 `element_id` 的组件。
 *
 * `repeat` / `if` 也必须纳入：它们是**本地指令**，虽然不会发给飞书，但同样要满足
 * 卡片结构校验的「element_id 唯一」规则——否则复制一个「告警列表」积木、
 * 或把同一个积木拖两次，就会产出 `alerts_loop` 重复的卡片，保存直接被后端拒绝。
 */
export const ID_TAGS = [
  'markdown',
  'div',
  'button',
  'column_set',
  'collapsible_panel',
  'repeat',
  'if',
];

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

/* ------------------------------------------------------------------ */
/* 画布辅助：拖放校验 / 选中链 / 展开估算                               */
/* ------------------------------------------------------------------ */

/** 循环指令 tag（子组件里可用 `${item.*}`）。 */
export const LOOP_TAG = 'repeat';
/** 条件指令 tag。 */
export const IF_TAG = 'if';

/** `parentPath` 的父组件 tag（`[]` 表示卡片根）。 */
export function parentTagAt(card: CardJson, parentPath: NodePath): string | null {
  if (parentPath.length === 0) return null;
  const parent = nodeAt(card, parentPath);
  return parent && typeof parent.tag === 'string' ? parent.tag : null;
}

/**
 * 能否把 `payload` 放到 `parentPath` 的第 `index` 位。
 *
 * 与旧实现的落点条判断口径一致：移动时禁止拖进自己的子孙（否则结构自毁），
 * 也禁止原地原位的无意义落点；无论新增还是移动，都要通过组件目录的
 * 「允许的父/子组件」校验（否则会做出飞书或后端拒绝的结构）。
 */
export function canDropPayload(
  card: CardJson,
  catalog: Catalog,
  payload: DragPayload,
  parentPath: NodePath,
  index: number,
): boolean {
  const parentTag = parentTagAt(card, parentPath);
  const allowed = parentTag ? catalog.get(parentTag)?.children ?? [] : [];
  const fitsParent = (tag: string): boolean =>
    catalog.canLive(tag, parentTag) && (allowed.length === 0 || allowed.includes(tag));

  if (payload.kind === 'move') {
    const from = payload.path;
    if (!from || from.length === 0) return false;
    if (isDescendant(parentPath, from)) return false;
    const fromParent = from.slice(0, -1);
    if (samePath(fromParent, parentPath)) {
      const at = from[from.length - 1];
      if (at === index || at === index - 1) return false; // 原地落点
    }
    const moving = nodeAt(card, from);
    const tag = moving && typeof moving.tag === 'string' ? moving.tag : '';
    return tag !== '' && fitsParent(tag);
  }

  const nodes = payload.nodes ?? [];
  if (nodes.length === 0) return false;
  return nodes.every((node) => typeof node.tag === 'string' && fitsParent(node.tag));
}

/** 从卡片根到 `path` 的 tag 链（含自身），用于判断「当前在哪个作用域」。 */
export function pathTagChain(card: CardJson, path: NodePath): string[] {
  const chain: string[] = [];
  for (let depth = 1; depth <= path.length; depth += 1) {
    const node = nodeAt(card, path.slice(0, depth));
    if (node && typeof node.tag === 'string') chain.push(node.tag);
  }
  return chain;
}

/** 选中节点是否位于循环体内（决定 `${item.*}` 变量是否可用）。 */
export function isInsideLoop(card: CardJson, path: NodePath | null): boolean {
  if (!path || path.length <= 1) return false;
  // 「循环组件自身」不算在循环内：它的 empty_content 用的是外层上下文。
  return pathTagChain(card, path.slice(0, -1)).includes(LOOP_TAG);
}

/**
 * `parentPath` 这个**子节点列表**是否属于循环体。
 *
 * 用于插入新节点时判断要不要分配 `element_id`：循环体里的组件会被复制多份，
 * id 必然重复（保存被拒 / 飞书 300301），所以拖进去时就不带 id。
 */
export function isInsideLoopBody(card: CardJson, parentPath: NodePath): boolean {
  if (parentPath.length === 0) return false;
  return parentTagAt(card, parentPath) === LOOP_TAG || isInsideLoop(card, parentPath);
}

/** 用新节点替换 `path` 处的节点（积木升级：变量版 → 结构化循环）。 */
export function replaceNode(
  card: CardJson,
  catalog: Catalog,
  path: NodePath,
  nodes: Record<string, unknown>[],
): CardJson {
  if (path.length === 0 || nodes.length === 0) return card;
  const parentPath = path.slice(0, -1);
  const index = path[path.length - 1];
  let next = removeNode(card, catalog, path);
  nodes.forEach((node, offset) => {
    next = insertNode(next, catalog, parentPath, index + offset, node);
  });
  return next;
}

/**
 * 估算**展开后**的组件数：循环会按 `max_items`（缺省 20）复制子组件。
 *
 * 本地指令本身在渲染后就不存在了（`if` 展开成子组件、`repeat` 展开成 N 份），
 * 所以它们不计入数量。服务端还会叠加策略上限与 200 组件保护，这里只做量级提示。
 */
export function estimatedNodesCost(nodes: unknown[]): number {
  const countList = (list: unknown[]): number =>
    list.reduce<number>((sum, item) => sum + countNode(item), 0);
  const countNode = (node: unknown): number => {
    if (!isPlainObject(node)) return 0;
    const children = childList(node);
    if (node.tag === LOOP_TAG) {
      const max = loopMaxItems(node);
      return max * countList(children);
    }
    if (node.tag === IF_TAG) return countList(children);
    return 1 + countList(children);
  };
  return countList(nodes);
}

/** 循环的展开次数上限（`max_items`，缺省 20）。 */
export function loopMaxItems(node: Record<string, unknown>): number {
  return typeof node.max_items === 'number' && node.max_items > 0 ? node.max_items : 20;
}

/** 整张卡片展开后的组件数估算。 */
export function estimatedRenderedElements(card: CardJson): number {
  return estimatedNodesCost(rootElements(card));
}

/** 卡片里是否用了本地指令（循环 / 条件）。 */
export function usesDirectives(card: CardJson): boolean {
  return collectTags(card).some((tag) => tag === LOOP_TAG || tag === IF_TAG);
}

/* ------------------------------------------------------------------ */
/* 变量分析：字段级绑定 + 卡片级诊断                                     */
/* ------------------------------------------------------------------ */

/** `${...}` 占位符（与服务端 `VARIABLE_RE` 同口径）。 */
const VARIABLE_TOKEN_RE = /\$\{([A-Za-z0-9_.:\-]+)\}/g;

/**
 * 单个变量的判定结果：
 * - `ok`：在内置变量表里，且当前作用域可用；
 * - `scope`：是 `item.*`，但当前不在循环体内 → 渲染成空串；
 * - `unknown`：不在变量表里（也可能是标签/注解原名，性质上无法静态判定）。
 */
export type VariableState = 'ok' | 'scope' | 'unknown';

export interface VariableToken {
  name: string;
  state: VariableState;
  description: string;
}

/** 变量的静态判定（字段级与卡片级共用）。 */
export function classifyVariable(
  name: string,
  variables: MetaVariable[],
  inLoop: boolean,
): VariableToken {
  const declared = variables.find((v) => v.name === name);
  if (declared) {
    const itemScoped = declared.group === 'item' || name.startsWith('item.');
    if (itemScoped && !inLoop) {
      return {
        name,
        state: 'scope',
        description: '只在「循环」组件的子组件里有效；当前位置会渲染成空串',
      };
    }
    return { name, state: 'ok', description: declared.description ?? '' };
  }
  if (name.startsWith('item.')) {
    // 动态形式（item.label:x / item.labels.x …）：循环内视为可用
    return inLoop
      ? { name, state: 'ok', description: '循环内的动态取值' }
      : {
          name,
          state: 'scope',
          description: '只在「循环」组件的子组件里有效；当前位置会渲染成空串',
        };
  }
  if (name.includes(':')) {
    return { name, state: 'ok', description: '动态取值（标签 / 注解）' };
  }
  return {
    name,
    state: 'unknown',
    description: '不在内置变量表里；如果它正好是某个标签/注解名可以忽略，否则会渲染成空串',
  };
}

/** 解析一段文本里的全部 `${...}`（按出现顺序，去重）。 */
export function variableTokensInText(
  text: string,
  variables: MetaVariable[],
  inLoop: boolean,
): VariableToken[] {
  const seen = new Set<string>();
  const out: VariableToken[] = [];
  for (const match of text.matchAll(VARIABLE_TOKEN_RE)) {
    const name = match[1];
    if (seen.has(name)) continue;
    seen.add(name);
    out.push(classifyVariable(name, variables, inLoop));
  }
  return out;
}

export interface CardIssue {
  /** 出问题的节点路径；null 表示卡片级问题。 */
  path: NodePath | null;
  severity: 'error' | 'warn';
  message: string;
}

export interface CardAnalysis {
  issues: CardIssue[];
  estimated: number;
  bytes: number;
}

/** 递归收集一个节点里的所有字符串（用于变量检查）。 */
function stringsIn(value: unknown, out: string[]): void {
  if (typeof value === 'string') {
    out.push(value);
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item) => stringsIn(item, out));
    return;
  }
  if (isPlainObject(value)) Object.values(value).forEach((item) => stringsIn(item, out));
}

/**
 * 卡片级诊断：把「渲染时会出问题」的地方提前、就地指出来。
 *
 * 覆盖四类：
 * 1. 循环外用了 `${item.*}`（一定渲染成空串）；
 * 2. 未声明的变量（可能是标签原名，措辞保持克制）；
 * 3. 空的循环体 / 单个循环展开就超过 200 个组件；
 * 4. 整卡展开数或体积超限。
 */
export function analyzeCard(card: CardJson, variables: MetaVariable[]): CardAnalysis {
  const issues: CardIssue[] = [];
  const scopeIssues = new Map<string, NodePath>();
  const unknownIssues = new Map<string, NodePath>();

  walkNodes(card, (node, path) => {
    if (node.tag === LOOP_TAG) {
      const body = childList(node);
      if (body.length === 0) {
        issues.push({
          path,
          severity: 'warn',
          message: '循环体是空的：什么都不会渲染（把「富文本」等组件拖进去）',
        });
      }
      const each = loopMaxItems(node);
      const cost = each * estimatedNodesCost(body);
      if (cost > MAX_CARD_ELEMENTS) {
        issues.push({
          path,
          severity: 'error',
          message: `循环最多会展开 ${cost} 个组件（${each} × ${estimatedNodesCost(body)}），超过飞书 200 上限；调小「最多循环」`,
        });
      }
    }
    const strings: string[] = [];
    Object.entries(node).forEach(([key, value]) => {
      if (key === 'tag' || key === 'element_id') return;
      // 子节点数组交给 walkNodes 单独访问：否则父节点会把子节点的变量按自己的
      // 作用域再判一次（循环体内的 item.* 会被误报成越权）。
      if (Array.isArray(value) && value.some((item) => isPlainObject(item) && item.tag)) return;
      stringsIn(value, strings);
    });
    const inLoop = isInsideLoop(card, path);
    for (const text of strings) {
      for (const token of variableTokensInText(text, variables, inLoop)) {
        if (token.state === 'scope' && !scopeIssues.has(token.name)) scopeIssues.set(token.name, path);
        if (token.state === 'unknown' && !unknownIssues.has(token.name)) {
          unknownIssues.set(token.name, path);
        }
      }
    }
  });
  for (const [name, path] of scopeIssues) {
    issues.push({
      path,
      severity: 'warn',
      message: `\${${name}} 只在「循环」组件内有效，这里会渲染成空串`,
    });
  }
  for (const [name, path] of unknownIssues) {
    issues.push({
      path,
      severity: 'warn',
      message: `\${${name}} 不在内置变量表里（若它是标签/注解原名可忽略，否则会渲染成空串）`,
    });
  }

  const estimated = estimatedRenderedElements(card);
  const bytes = cardBytes(card);
  if (estimated > MAX_CARD_ELEMENTS) {
    issues.push({
      path: null,
      severity: 'error',
      message: `展开后约 ${estimated} 个组件，超过飞书 200 上限（会按文档顺序丢弃靠后的组件）`,
    });
  }
  if (bytes > MAX_BYTES) {
    issues.push({
      path: null,
      severity: 'error',
      message: `卡片体积 ${(bytes / 1024).toFixed(1)}KB 超过 30KB 上限，保存会被拒绝`,
    });
  }
  return { issues, estimated, bytes };
}
