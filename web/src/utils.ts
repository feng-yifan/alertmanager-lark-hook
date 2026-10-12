/** Small formatting / coercion helpers shared by the pages. */

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

export function outcomeText(outcome: unknown): string {
  if (typeof outcome === 'string') return outcome;
  if (isPlainObject(outcome)) {
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

/** Normalises the unspecified `AuditActor` serialization. */
export function actorText(actor: unknown): string {
  if (actor === null || actor === undefined) return '—';
  if (typeof actor === 'string') return actor;
  if (isPlainObject(actor)) {
    if (typeof actor.type === 'string') {
      const who = actor.user ?? actor.hook ?? actor.job ?? actor.name;
      return who ? `${actor.type}:${String(who)}` : actor.type;
    }
    return prettyJson(actor).replace(/\s+/g, ' ');
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
