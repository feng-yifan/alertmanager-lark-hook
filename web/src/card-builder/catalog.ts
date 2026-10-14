/**
 * 组件面板的分组辅助。
 *
 * 组件目录与告警积木**全部来自服务端**（`GET /api/v1/meta` 的 `card_components`
 * / `card_presets`），前端只负责展示与拖拽——否则两边迟早漂移。
 */
import type { CardComponentMeta, CardPresetMeta } from '../types';

/** 组件面板分组的中文标题与顺序。 */
export const GROUP_TITLES: Array<{ group: CardComponentMeta['group']; title: string }> = [
  { group: 'display', title: '展示类' },
  { group: 'interactive', title: '交互类' },
  { group: 'container', title: '容器类' },
  { group: 'directive', title: '本地指令（预览/发送时由服务端展开）' },
];

/** 解析积木的节点数组（服务端用 JSON 文本传输，与 `default_json` 一致）。 */
export function presetNodes(preset: CardPresetMeta): Record<string, unknown>[] {
  const parsed: unknown = JSON.parse(preset.nodes_json);
  if (!Array.isArray(parsed)) {
    throw new Error(`积木「${preset.label}」的 nodes_json 不是数组`);
  }
  return parsed.filter(
    (node): node is Record<string, unknown> => typeof node === 'object' && node !== null,
  );
}
