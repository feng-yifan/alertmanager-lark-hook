/** Small formatting / coercion helpers shared by the pages. */

import type { Delivery } from './types';

export function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ');
}

const dateTimeFormatter = new Intl.DateTimeFormat('zh-CN', {
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
  hour12: false,
});

/** RFC 3339 UTC -> local `YYYY-MM-DD HH:mm:ss`. */
export function formatDateTime(value: string | null | undefined): string {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return dateTimeFormatter.format(date).replace(/\//g, '-');
}

/** Compact relative time (`3 分钟前`), falling back to the absolute timestamp. */
export function formatRelative(value: string | null | undefined): string {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const diffSec = Math.round((Date.now() - date.getTime()) / 1000);
  if (diffSec < 0) return formatDateTime(value);
  if (diffSec < 60) return `${diffSec} 秒前`;
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)} 分钟前`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)} 小时前`;
  if (diffSec < 86400 * 30) return `${Math.floor(diffSec / 86400)} 天前`;
  return formatDateTime(value);
}

/** RFC 3339 UTC -> value for `<input type="datetime-local">` (local wall clock). */
export function isoToLocalInput(value: string | null | undefined): string {
  if (!value) return '';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '';
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(
    date.getMinutes(),
  )}`;
}

/** `<input type="datetime-local">` value -> RFC 3339 UTC string. */
export function localInputToIso(value: string): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return date.toISOString();
}

export function shortId(id: string | null | undefined, length = 8): string {
  if (!id) return '—';
  return id.length > length ? `${id.slice(0, length)}…` : id;
}

export function prettyJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2) ?? 'null';
  } catch {
    return String(value);
  }
}

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Normalises the unspecified audit `Outcome` serialization. */
export function outcomeTone(outcome: unknown): 'ok' | 'warn' | 'danger' | 'muted' {
  const text = outcomeText(outcome).toLowerCase();
  if (text.startsWith('ok') || text.includes('success')) return 'ok';
  if (text.startsWith('skip')) return 'warn';
  if (text.startsWith('fail') || text.startsWith('error')) return 'danger';
  return 'muted';
}

/**
 * Normalises the audit `Outcome` serialization.
 *
 * 真实线格式是 serde 的 internally-tagged enum：
 * `{"type":"ok"}` / `{"type":"skipped","reason":"duplicate"}` / `{"type":"failed","error":"..."}`。
 * 旧的字符串与 `{ok}/{skipped}/{failed}` 形态继续兼容。
 */
export function outcomeText(outcome: unknown): string {
  if (typeof outcome === 'string') return outcome;
  if (isPlainObject(outcome)) {
    const type = typeof outcome.type === 'string' ? outcome.type : null;
    if (type) {
      if (type === 'ok') return 'ok';
      if (type === 'skipped') return typeof outcome.reason === 'string' && outcome.reason ? `skipped:${outcome.reason}` : 'skipped';
      if (type === 'failed' || type === 'error') {
        return typeof outcome.error === 'string' && outcome.error ? `failed:${outcome.error}` : 'failed';
      }
      return type;
    }
    if (typeof outcome.skipped === 'string') return `skipped:${outcome.skipped}`;
    if (typeof outcome.failed === 'string') return `failed:${outcome.failed}`;
    if (outcome.ok === true) return 'ok';
    return prettyJson(outcome).replace(/\s+/g, ' ');
  }
  return String(outcome ?? '—');
}

export function outcomeLabel(outcome: unknown): string {
  const text = outcomeText(outcome);
  const lower = text.toLowerCase();
  if (lower === 'ok') return '成功';
  if (lower.startsWith('skipped')) return `跳过${text.includes(':') ? `：${text.split(':').slice(1).join(':')}` : ''}`;
  if (lower.startsWith('failed')) return `失败${text.includes(':') ? `：${text.split(':').slice(1).join(':')}` : ''}`;
  return text;
}

/**
 * Normalises the `AuditActor` serialization.
 *
 * 真实线格式是 serde 的 internally-tagged enum：
 * `{"type":"admin","username":"x"}` / `{"type":"alert_manager","hook_id":"..."}` / `{"type":"system","job":"..."}`。
 * 旧的 `{user|hook|job|name}` 形态继续兼容；admin 显示 username（而不是只显示角色名）。
 */
export function actorText(actor: unknown): string {
  if (actor === null || actor === undefined) return '—';
  if (typeof actor === 'string') return actor;
  if (isPlainObject(actor)) {
    const pick = (value: unknown): string | null => (typeof value === 'string' && value ? value : null);
    const type = pick(actor.type);
    const username = pick(actor.username) ?? pick(actor.user) ?? pick(actor.name);
    const hookId = pick(actor.hook_id) ?? pick(actor.hook);
    const job = pick(actor.job);
    if (type === 'admin' || type === 'user') return username ?? type;
    if (type === 'alert_manager' || type === 'hook') return hookId ? `hook:${hookId}` : type;
    if (type === 'system' || type === 'job') return job ?? type;
    if (type) return username ?? hookId ?? job ?? type;
    return username ?? hookId ?? job ?? prettyJson(actor).replace(/\s+/g, ' ');
  }
  return String(actor);
}

/** Renders `errors`/`warnings` entries whose shape `docs/api.md` leaves open. */
export function validationText(item: unknown): string {
  if (typeof item === 'string') return item;
  if (isPlainObject(item)) {
    const message = item.message ?? item.code ?? item.field ?? item.path;
    const prefix = typeof item.path === 'string' ? `${item.path}: ` : typeof item.field === 'string' ? `${item.field}: ` : '';
    if (typeof message === 'string') return `${prefix}${message}`;
    return prettyJson(item).replace(/\s+/g, ' ');
  }
  return String(item);
}

export function formatBytes(len: number): string {
  if (len < 1024) return `${len} B`;
  return `${(len / 1024).toFixed(1)} KB`;
}

/** Iterates `entries` with a stable order for label maps. */
export function sortedEntries(map: Record<string, string> | null | undefined): Array<[string, string]> {
  if (!map) return [];
  return Object.entries(map).sort(([a], [b]) => a.localeCompare(b));
}

export function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…` : text;
}

/**
 * 「重试投递」的真实结果。
 *
 * 手动重试是**当场重发一次**（不是把记录塞回后台队列）：接口返回的那条 `Delivery`
 * 就是本次尝试的落库结果。必须按 `state` 决定提示，否则「重试又失败了」会被
 * 无脑报成「已提交重试，等待后台队列重新发送」——用户会一直收不到消息且毫无线索。
 *
 * - `active` / `resolved`：真的发出去了；
 * - `pending`：本次没成功，但错误可重试，已回到后台退避队列；
 * - `failed`：终态，后台不会再自动重试，必须先修外部原因再手动重试。
 */
export function describeRetryResult(delivery: Pick<Delivery, 'state' | 'last_error'>): {
  tone: 'success' | 'warn' | 'error';
  message: string;
} {
  const reason = delivery.last_error ? `：${truncate(delivery.last_error, 160)}` : '';
  if (delivery.state === 'active' || delivery.state === 'resolved') {
    return { tone: 'success', message: '重试成功，消息已发送' };
  }
  if (delivery.state === 'pending') {
    return {
      tone: 'warn',
      message: `本次重发未成功${reason}。已回到后台重试队列，会继续退避重试。`,
    };
  }
  return {
    tone: 'error',
    message: `重试失败${reason}。这是终态，后台不会再自动重试；原因（权限 / 群 / 模板 / 凭据）修复前重试也不会发出消息。`,
  };
}

/**
 * 只放行 `http(s)://` 的绝对 URL，其余返回 `null`。
 *
 * 告警的 `generator_url` 完全由发送方控制（任何持有 hook token 的人都能构造）。
 * React 18 **不会**拦截 `javascript:` 这类协议，直接放进 `<a href>` 会让脚本在
 * 管理后台同源执行 ⇒ 一次点击等于交出后台。后端已在归一化阶段丢弃非法值，
 * 这里是第二道防线（也覆盖升级前就入库的历史数据）。
 *
 * 调用方用返回值是否存在来决定「渲染链接 / 渲染占位符」。
 */
export function safeHttpUrl(value: string | null | undefined): string | null {
  if (!value) return null;
  const trimmed = value.trim();
  const lower = trimmed.toLowerCase();
  return lower.startsWith('http://') || lower.startsWith('https://') ? trimmed : null;
}
