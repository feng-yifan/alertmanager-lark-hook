/**
 * 落点计算：**纯几何**，不依赖 React / dnd-kit，便于单独验证。
 *
 * 规则（考虑嵌套容器）：
 * 1. 指针必须在画布容器附近（4px 容差），否则没有落点——从左侧面板拖出但还没进画布时不该出现落点；
 * 2. 指针**在某个列表内部**时，只认**面积最小**的那个（也就是嵌套最深的容器）。
 *    它不接受（例如移动时的「原地落点」、父组件不允许的子组件、不允许的移动）就**不落**，
 *    绝不回退到祖先列表——否则一次几像素的误拖会把节点悄悄挪出循环/分栏；
 * 3. 指针在所有列表之外但仍在画布范围内时，选**距离最近**的列表（拖到末尾空白处也能放下）；
 * 4. 每个候选都要通过 `canDrop`（含目录校验与「不能拖进自己的子孙」）。
 */
import { samePath, type DragPayload, type NodePath } from './model';

export interface DropTarget {
  parentPath: NodePath;
  index: number;
}

const LIST_SELECTOR = '[data-cb-list]';

export function distanceToRect(point: { x: number; y: number }, rect: DOMRect): number {
  const dx = Math.max(rect.left - point.x, 0, point.x - rect.right);
  const dy = Math.max(rect.top - point.y, 0, point.y - rect.bottom);
  return Math.hypot(dx, dy);
}

function insideRect(point: { x: number; y: number }, rect: DOMRect): boolean {
  return (
    point.x >= rect.left &&
    point.x <= rect.right &&
    point.y >= rect.top &&
    point.y <= rect.bottom
  );
}

function area(rect: DOMRect): number {
  return Math.max(1, rect.width * rect.height);
}

interface Candidate {
  target: DropTarget;
  rect: DOMRect;
}

/** 按指针位置算出「放进哪个容器的第几位」。 */
export function computeDropTarget(
  container: HTMLElement,
  pointer: { x: number; y: number },
  payload: DragPayload,
  canDrop: (payload: DragPayload, parentPath: NodePath, index: number) => boolean,
): DropTarget | null {
  const bounds = container.getBoundingClientRect();
  // 容差刻意小于三栏之间的 12px 间隔：拖到间隔里等于「还没进画布」。
  const margin = 4;
  if (
    pointer.x < bounds.left - margin ||
    pointer.x > bounds.right + margin ||
    pointer.y < bounds.top - margin ||
    pointer.y > bounds.bottom + margin
  ) {
    return null;
  }

  let innermost: Candidate | null = null; // 指针所在的最深列表
  let nearestOutside: { candidate: Candidate; distance: number } | null = null;

  for (const list of Array.from(container.querySelectorAll<HTMLElement>(LIST_SELECTOR))) {
    let parentPath: NodePath;
    try {
      parentPath = JSON.parse(list.dataset.cbList ?? '[]') as NodePath;
    } catch {
      continue;
    }
    if (!Array.isArray(parentPath)) continue;

    const rect = list.getBoundingClientRect();
    const horizontal = list.dataset.cbLayout === 'columns';
    const axis = horizontal ? pointer.x : pointer.y;
    const items = Array.from(
      list.querySelectorAll<HTMLElement>(':scope > [data-cb-node-index]'),
    );
    let index = items.length;
    for (let i = 0; i < items.length; i += 1) {
      const itemRect = items[i].getBoundingClientRect();
      const middle = horizontal
        ? itemRect.left + itemRect.width / 2
        : itemRect.top + itemRect.height / 2;
      if (axis < middle) {
        index = i;
        break;
      }
    }
    const candidate: Candidate = { target: { parentPath, index }, rect };

    if (insideRect(pointer, rect)) {
      if (!innermost || area(rect) < area(innermost.rect)) innermost = candidate;
    } else {
      const distance = distanceToRect(pointer, rect);
      if (!nearestOutside || distance < nearestOutside.distance) {
        nearestOutside = { candidate, distance };
      }
    }
  }

  // 指针在列表内部：只认最深的那个，被拒绝就保持「无落点」。
  if (innermost) {
    const { parentPath, index } = innermost.target;
    return canDrop(payload, parentPath, index) ? innermost.target : null;
  }

  // 指针在画布内的空白处（画布内边距、标题块、设置入口附近）：就近吸附。
  if (nearestOutside) {
    const { parentPath, index } = nearestOutside.candidate.target;
    return canDrop(payload, parentPath, index) ? nearestOutside.candidate.target : null;
  }

  return null;
}

/** 目标没变时避免触发 React 重渲染。 */
export function sameDropTarget(a: DropTarget | null, b: DropTarget | null): boolean {
  if (!a || !b) return a === b;
  return a.index === b.index && samePath(a.parentPath, b.parentPath);
}
