/**
 * 画布拖拽引擎（基于 `@dnd-kit/core` 的指针拖拽 + 自定义落点计算）。
 *
 * 为什么这么设计：
 * 1. **落点由指针几何位置决定**，而不是靠「把指针挪到两个组件之间的细缝里再松手」——
 *    命中的是**整块列表区域**，指针落在某个子组件上半区/下半区就插到它前面/后面，
 *    这是最常见的拖拽心智模型（落点算法见 `dropTarget.ts`，可单独验证）。
 * 2. 列表是嵌套的（分栏、折叠面板、循环体），因此按「包含指针且面积最小」选最深的列表。
 * 3. 指针落在所有列表之外时，选距离最近的那个（拖到底部空白处也能放到末尾）。
 * 4. 拖拽浮层由 `DragOverlay` 渲染成**半透明**的一块，蓝色落点条画在列表里，
 *    不再像原生拖拽那样被不透明的拖影盖住。
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { PointerSensor, useSensor, useSensors } from '@dnd-kit/core';
import type { DragEndEvent, DragStartEvent } from '@dnd-kit/core';
import type { DragPayload, NodePath } from './model';
import { computeDropTarget, sameDropTarget, type DropTarget } from './dropTarget';

export type { DropTarget };

interface DragOptions {
  /** 画布容器：落点列表都在它内部（`[data-cb-list]`）。 */
  containerRef: React.RefObject<HTMLElement>;
  canDrop: (payload: DragPayload, parentPath: NodePath, index: number) => boolean;
  onDrop: (payload: DragPayload, parentPath: NodePath, index: number) => void;
}

export interface CardDrag {
  sensors: ReturnType<typeof useSensors>;
  payload: DragPayload | null;
  target: DropTarget | null;
  onDragStart: (event: DragStartEvent) => void;
  onDragEnd: (event: DragEndEvent) => void;
  onDragCancel: () => void;
  /**
   * 拖拽结束后浏览器还会补一个 `click`（pointerdown/pointerup 落在同一祖先上），
   * 它会误触「选中卡片设置」或「再追加一个组件」。用时间窗把它吞掉。
   */
  shouldIgnoreClick: () => boolean;
}

export function useCardDrag({ containerRef, canDrop, onDrop }: DragOptions): CardDrag {
  const [payload, setPayload] = useState<DragPayload | null>(null);
  const [target, setTarget] = useState<DropTarget | null>(null);

  const payloadRef = useRef<DragPayload | null>(null);
  const targetRef = useRef<DropTarget | null>(null);
  const pointerRef = useRef<{ x: number; y: number } | null>(null);
  const lastDragEndRef = useRef(0);
  const canDropRef = useRef(canDrop);
  const onDropRef = useRef(onDrop);
  canDropRef.current = canDrop;
  onDropRef.current = onDrop;

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
  );

  const refresh = useCallback(() => {
    const container = containerRef.current;
    const current = payloadRef.current;
    const pointer = pointerRef.current;
    if (!container || !current || !pointer) {
      targetRef.current = null;
      setTarget(null);
      return;
    }
    const next = computeDropTarget(container, pointer, current, canDropRef.current);
    if (sameDropTarget(targetRef.current, next)) return;
    targetRef.current = next;
    setTarget(next);
  }, [containerRef]);

  // 指针位置直接读原始事件：比用 `delta` 推算更准，也不受滚动/缩放影响。
  useEffect(() => {
    if (!payload) return undefined;
    const handlePointerMove = (event: PointerEvent) => {
      pointerRef.current = { x: event.clientX, y: event.clientY };
      refresh();
    };
    const handleViewportChange = () => refresh();
    window.addEventListener('pointermove', handlePointerMove, { passive: true });
    window.addEventListener('scroll', handleViewportChange, true);
    window.addEventListener('resize', handleViewportChange);
    return () => {
      window.removeEventListener('pointermove', handlePointerMove);
      window.removeEventListener('scroll', handleViewportChange, true);
      window.removeEventListener('resize', handleViewportChange);
    };
  }, [payload, refresh]);

  const reset = useCallback(() => {
    payloadRef.current = null;
    targetRef.current = null;
    pointerRef.current = null;
    setPayload(null);
    setTarget(null);
  }, []);

  const onDragStart = useCallback(
    (event: DragStartEvent) => {
      const data = event.active.data.current as DragPayload | undefined;
      if (!data || !data.kind) return;
      payloadRef.current = data;
      const activator = event.activatorEvent as MouseEvent | PointerEvent | undefined;
      if (activator && typeof activator.clientX === 'number') {
        pointerRef.current = { x: activator.clientX, y: activator.clientY };
      }
      setPayload(data);
      window.requestAnimationFrame(refresh);
    },
    [refresh],
  );

  const onDragEnd = useCallback(() => {
    const current = payloadRef.current;
    const drop = targetRef.current;
    lastDragEndRef.current = Date.now();
    reset();
    if (current && drop) onDropRef.current(current, drop.parentPath, drop.index);
  }, [reset]);

  const onDragCancel = useCallback(() => {
    lastDragEndRef.current = Date.now();
    reset();
  }, [reset]);

  const shouldIgnoreClick = useCallback(() => Date.now() - lastDragEndRef.current < 250, []);

  return { sensors, payload, target, onDragStart, onDragEnd, onDragCancel, shouldIgnoreClick };
}
