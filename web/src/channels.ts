/**
 * 渠道选择器的口径。
 *
 * 后端把渠道的 `enabled` / `bot_can_send` 原样下发，前端不另立规则：
 * - **发送 / 预览类**选择器只列可用渠道（禁用渠道选了也发不出去）；
 * - **筛选类**（投递记录、告警分组）保留全部渠道，但用文案标出已禁用，
 *   否则历史数据就筛不到了。
 */
import type { Channel } from './types';

/** 可用于发送/预览的渠道。 */
export function selectableChannels(channels: Channel[]): Channel[] {
  return channels.filter((channel) => channel.enabled);
}

/** 下拉选项文案：禁用 / 机器人不可发送时追加说明。 */
export function channelOptionLabel(channel: Channel): string {
  if (!channel.enabled) return `${channel.name}（已禁用）`;
  if (!channel.bot_can_send) return `${channel.name}（机器人不可发送）`;
  return channel.name;
}
