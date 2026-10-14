/**
 * 积木识别：让画布节点显示**左侧积木的名字**，而不是抽象成「富文本 / 普通文本」。
 *
 * 用户从左侧拖「告警列表」到画布后，看到的仍然是「告警列表」——这样左右才对应得上。
 *
 * 实现方式是**结构指纹**：把节点 JSON 规范化（忽略 `element_id`）后与积木下发的
 * `nodes_json` 逐一比对。好处是零额外字段、不需要后端配合，保存/重新加载后依然有效；
 * 代价是用户改动过内容的节点会被视为「已改」，退回显示组件类型名（这是诚实的）。
 */
import type { CardPresetMeta } from '../types';
import { presetNodes } from './catalog';
import { isPlainObject } from './model';

/** 画布节点与积木的对应关系。 */
export interface NodeIdentity {
  /** 展示在画布节点标题上的积木名。 */
  blockLabel: string;
  /** 命中的积木 id。 */
  presetId: string | null;
  /** 变量版老节点可一键升级到的积木 id（没有则为 null）。 */
  legacyPresetId: string | null;
}

export interface PresetIndex {
  /** 返回节点的积木身份；没有命中且不是老节点时返回 null。 */
  describe(node: Record<string, unknown>): NodeIdentity | null;
  /** 取积木的节点数组（用于「升级为结构化循环」）。 */
  nodesOf(presetId: string): Record<string, unknown>[] | null;
}

/** 结构化指纹：忽略 `element_id`，并按键名排序，避免字段顺序影响比对。 */
function fingerprint(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(fingerprint).join(',')}]`;
  if (isPlainObject(value)) {
    return `{${Object.keys(value)
      .filter((key) => key !== 'element_id')
      .sort()
      .map((key) => `${JSON.stringify(key)}:${fingerprint(value[key])}`)
      .join(',')}}`;
  }
  return JSON.stringify(value) ?? 'null';
}

const LEGACY_VARIABLE_LABEL: Record<string, string> = {
  '${alerts_list}': '告警列表（变量版）',
  '${alerts_table}': '告警表格（变量版）',
};

/** 节点**自身**就是「变量版告警列表/表格」时返回该变量；否则 null。 */
function variableContent(node: Record<string, unknown>): string | null {
  if (node.tag !== 'markdown' || typeof node.content !== 'string') return null;
  const key = node.content.trim();
  return LEGACY_VARIABLE_LABEL[key] ? key : null;
}

/**
 * 老的「折叠详情」积木 = 面板里**只有**一个 `${alerts_list}` markdown，
 * 且标题仍是积木自带的「详情」。
 *
 * 只有这种「原封不动」的积木才允许整体升级；面板里若还有别的子组件、或标题被改过，
 * 一律不动——整体升级会覆盖用户内容，宁可让里面的 markdown 自己显示变量版徽标。
 */
function isLegacyDetailPanel(node: Record<string, unknown>): boolean {
  if (node.tag !== 'collapsible_panel') return false;
  const elements = node.elements;
  if (!Array.isArray(elements) || elements.length !== 1) return false;
  const child = elements[0];
  if (!isPlainObject(child) || variableContent(child) !== '${alerts_list}') return false;
  const header = node.header;
  if (!isPlainObject(header)) return false;
  const title = header.title;
  // 标题被改过 → 不是原封不动的积木，不能整体替换
  return isPlainObject(title) && title.content === '详情';
}

export function makePresetIndex(presets: CardPresetMeta[]): PresetIndex {
  const byFingerprint = new Map<string, { blockLabel: string; presetId: string }>();
  const nodesById = new Map<string, Record<string, unknown>[]>();

  for (const preset of presets) {
    let nodes: Record<string, unknown>[];
    try {
      nodes = presetNodes(preset);
    } catch {
      continue; // 服务端已经保证合法；单个积木坏了不应该让整块画布崩掉
    }
    nodesById.set(preset.id, nodes);
    for (const node of nodes) {
      const key = fingerprint(node);
      if (!byFingerprint.has(key)) {
        byFingerprint.set(key, { blockLabel: preset.label, presetId: preset.id });
      }
    }
  }

  return {
    describe(node) {
      const hit = byFingerprint.get(fingerprint(node));
      if (hit) return { ...hit, legacyPresetId: null };

      const variable = variableContent(node);
      if (variable) {
        const upgrade =
          variable === '${alerts_list}' ? 'alerts_list' : null;
        return {
          blockLabel: LEGACY_VARIABLE_LABEL[variable],
          presetId: null,
          legacyPresetId: upgrade,
        };
      }

      if (isLegacyDetailPanel(node)) {
        return {
          blockLabel: '折叠详情（变量版）',
          presetId: null,
          legacyPresetId: 'collapsible_detail',
        };
      }
      return null;
    },
    nodesOf(presetId) {
      return nodesById.get(presetId) ?? null;
    },
  };
}
