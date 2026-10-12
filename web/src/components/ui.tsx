/** Hand-rolled component kit (no component library). */
import { useEffect, useId, useState, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, type TextareaHTMLAttributes } from 'react';
import { errorMessage } from '../api';
import type { EnumOption } from '../types';
import {
  ALERT_STATUS_LABEL,
  CHANNEL_KIND_LABEL,
  CHANNEL_SOURCE_LABEL,
  DELIVERY_KIND_LABEL,
  DELIVERY_STATE_LABEL,
  GROUP_STATUS_LABEL,
  MESSAGE_TYPE_LABEL,
  label as mapLabel,
} from '../labels';
import { cx, outcomeLabel, outcomeTone, prettyJson } from '../utils';

export type Tone = 'ok' | 'warn' | 'danger' | 'info' | 'accent' | 'muted';

/* ------------------------------------------------------------------ text */

export function PageHeader({
  title,
  subtitle,
  actions,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
}): JSX.Element {
  return (
    <div className="page-header">
      <div>
        <h1>{title}</h1>
        {subtitle ? <div className="subtitle">{subtitle}</div> : null}
      </div>
      {actions ? <div className="page-actions">{actions}</div> : null}
    </div>
  );
}

export function Card({
  title,
  subtitle,
  actions,
  children,
  footer,
  bodyClassName,
  className,
  tight,
}: {
  title?: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  bodyClassName?: string;
  className?: string;
  /** `tight` removes the body padding (tables). */
  tight?: boolean;
}): JSX.Element {
  return (
    <section className={cx('card', className)}>
      {title || actions || subtitle ? (
        <header className="card-header">
          <div>
            {title ? <h2>{title}</h2> : null}
            {subtitle ? <div className="card-hint">{subtitle}</div> : null}
          </div>
          {actions ? <div className="row tight">{actions}</div> : null}
        </header>
      ) : null}
      <div className={cx('card-body', tight && 'tight', bodyClassName)}>{children}</div>
      {footer ? <footer className="card-footer">{footer}</footer> : null}
    </section>
  );
}

/* ----------------------------------------------------------------- button */

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'default' | 'primary' | 'danger' | 'ghost';
  size?: 'md' | 'sm';
  loading?: boolean;
}

export function Button({
  variant = 'default',
  size = 'md',
  loading = false,
  className,
  children,
  disabled,
  type = 'button',
  ...rest
}: ButtonProps): JSX.Element {
  return (
    <button
      {...rest}
      type={type}
      disabled={disabled || loading}
      className={cx('btn', variant !== 'default' && `btn-${variant}`, size === 'sm' && 'btn-sm', className)}
    >
      {loading ? <span className="spinner" aria-hidden /> : null}
      {children}
    </button>
  );
}

/* ------------------------------------------------------------------ form */

export function Field({
  label,
  hint,
  error,
  required,
  htmlFor,
  children,
  className,
}: {
  label?: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  required?: boolean;
  htmlFor?: string;
  children: ReactNode;
  className?: string;
}): JSX.Element {
  return (
    <div className={cx('field', className)}>
      {label ? (
        <label htmlFor={htmlFor}>
          {label}
          {required ? <span className="req">*</span> : null}
        </label>
      ) : null}
      {children}
      {hint ? <div className="hint">{hint}</div> : null}
      {error ? <div className="field-error">{error}</div> : null}
    </div>
  );
}

export function Input({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>): JSX.Element {
  return <input {...rest} className={cx('input', className)} />;
}

export function Select({ className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>): JSX.Element {
  return (
    <select {...rest} className={cx('select', className)}>
      {children}
    </select>
  );
}

export function Textarea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>): JSX.Element {
  return <textarea {...rest} className={cx('textarea', className)} />;
}

/** `<option>` list for an enum from `GET /api/v1/meta`. */
export function Options({ options }: { options: EnumOption[] }): JSX.Element {
  return (
    <>
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </>
  );
}

export function Switch({
  checked,
  onChange,
  disabled,
  label,
  hint,
  id,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label?: ReactNode;
  hint?: ReactNode;
  id?: string;
}): JSX.Element {
  const generatedId = useId();
  const inputId = id ?? generatedId;
  return (
    <div className="switch-row">
      <label className="switch" htmlFor={inputId}>
        <input
          id={inputId}
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={(event) => onChange(event.target.checked)}
        />
        <span className="track" />
      </label>
      {label || hint ? (
        <div>
          {label ? (
            <label htmlFor={inputId} className="small" style={{ fontWeight: 600 }}>
              {label}
            </label>
          ) : null}
          {hint ? <div className="hint">{hint}</div> : null}
        </div>
      ) : null}
    </div>
  );
}

export function RadioGroup<T extends string>({
  value,
  options,
  onChange,
  disabled,
  name,
}: {
  value: T;
  options: Array<{ value: T; label: ReactNode; hint?: ReactNode }>;
  onChange: (next: T) => void;
  disabled?: boolean;
  name?: string;
}): JSX.Element {
  const generated = useId();
  const groupName = name ?? generated;
  return (
    <div className="radio-group" role="radiogroup">
      {options.map((option) => (
        <label key={option.value} className={cx('radio-option', option.value === value && 'selected')}>
          <input
            type="radio"
            name={groupName}
            value={option.value}
            checked={option.value === value}
            disabled={disabled}
            onChange={() => onChange(option.value)}
          />
          <span>
            {option.label}
            {option.hint ? <span className="hint"> {option.hint}</span> : null}
          </span>
        </label>
      ))}
    </div>
  );
}

/* ----------------------------------------------------------------- badges */

export type StatusKind =
  | 'delivery'
  | 'delivery_kind'
  | 'group'
  | 'alert'
  | 'message_type'
  | 'enabled'
  | 'outcome'
  | 'channel_source'
  | 'channel_kind';

export function Badge({ tone = 'muted', children }: { tone?: Tone; children: ReactNode }): JSX.Element {
  return <span className={cx('badge', `badge-${tone}`)}>{children}</span>;
}

export function StatusBadge({ kind, value }: { kind: StatusKind; value: unknown }): JSX.Element {
  const resolved = resolveStatus(kind, value);
  return <Badge tone={resolved.tone}>{resolved.text}</Badge>;
}

function resolveStatus(kind: StatusKind, value: unknown): { tone: Tone; text: string } {
  const raw = typeof value === 'string' ? value : value === true ? 'true' : value === false ? 'false' : String(value ?? '');
  switch (kind) {
    case 'delivery': {
      const tone: Tone =
        raw === 'active'
          ? 'ok'
          : raw === 'failed' || raw === 'recall_failed'
            ? 'danger'
            : raw === 'pending'
              ? 'info'
              : raw === 'resolved'
                ? 'info'
                : 'muted';
      return { tone, text: mapLabel(DELIVERY_STATE_LABEL, raw) };
    }
    case 'delivery_kind':
      return { tone: 'accent', text: mapLabel(DELIVERY_KIND_LABEL, raw) };
    case 'group':
      return { tone: raw === 'firing' ? 'danger' : 'ok', text: mapLabel(GROUP_STATUS_LABEL, raw) };
    case 'alert':
      return { tone: raw === 'firing' ? 'danger' : 'ok', text: mapLabel(ALERT_STATUS_LABEL, raw) };
    case 'message_type':
      return { tone: raw === 'card_message' ? 'accent' : 'info', text: mapLabel(MESSAGE_TYPE_LABEL, raw) };
    case 'enabled':
      return raw === 'true' ? { tone: 'ok', text: '已启用' } : { tone: 'muted', text: '已停用' };
    case 'outcome':
      return { tone: outcomeTone(value === true ? 'ok' : value), text: outcomeLabel(value === true ? 'ok' : value) };
    case 'channel_source':
      return { tone: 'muted', text: mapLabel(CHANNEL_SOURCE_LABEL, raw) };
    case 'channel_kind':
      return { tone: 'muted', text: mapLabel(CHANNEL_KIND_LABEL, raw) };
    default:
      return { tone: 'muted', text: raw || '—' };
  }
}

/* ---------------------------------------------------------------- states */

export function Spinner(): JSX.Element {
  return <span className="spinner" aria-hidden />;
}

export function Loading({ label }: { label?: ReactNode }): JSX.Element {
  return (
    <div className="state">
      <Spinner />
      <div>{label ?? '加载中…'}</div>
    </div>
  );
}

export function EmptyState({
  title,
  description,
  action,
}: {
  title: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
}): JSX.Element {
  return (
    <div className="state">
      <div className="state-title">{title}</div>
      {description ? <div className="state-desc">{description}</div> : null}
      {action}
    </div>
  );
}

export function EmptyRow({ colSpan, title, description }: { colSpan: number; title: ReactNode; description?: ReactNode }): JSX.Element {
  return (
    <tr>
      <td colSpan={colSpan}>
        <EmptyState title={title} description={description} />
      </td>
    </tr>
  );
}

export function Banner({
  tone = 'info',
  children,
  onClose,
}: {
  tone?: 'info' | 'success' | 'error' | 'warn';
  children: ReactNode;
  onClose?: () => void;
}): JSX.Element {
  return (
    <div className={cx('banner', `banner-${tone}`)}>
      <div style={{ flex: 1, minWidth: 0 }}>{children}</div>
      {onClose ? (
        <button type="button" className="banner-close" aria-label="关闭" onClick={onClose}>
          ×
        </button>
      ) : null}
    </div>
  );
}

export function ErrorBanner({ error, onRetry, onClose }: { error: unknown; onRetry?: () => void; onClose?: () => void }): JSX.Element {
  return (
    <Banner tone="error" onClose={onClose}>
      <div className="row tight">
        <span>{errorMessage(error)}</span>
        {onRetry ? (
          <button type="button" className="btn btn-sm" onClick={onRetry}>
            重试
          </button>
        ) : null}
      </div>
    </Banner>
  );
}

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }): JSX.Element {
  return (
    <div className="state">
      <div className="state-title">加载失败</div>
      <div className="state-desc">{errorMessage(error)}</div>
      {onRetry ? (
        <Button size="sm" onClick={onRetry}>
          重试
        </Button>
      ) : null}
    </div>
  );
}

/* ----------------------------------------------------------------- modal */

export function Modal({
  open,
  title,
  onClose,
  children,
  footer,
  wide,
}: {
  open: boolean;
  title: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
}): JSX.Element | null {
  useEffect(() => {
    if (!open) return undefined;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      className="modal-backdrop"
      role="dialog"
      aria-modal="true"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className={cx('modal', wide && 'wide')}>
        <header className="modal-header">
          <h2>{title}</h2>
          <button type="button" className="btn btn-ghost btn-sm" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </header>
        <div className="modal-body">{children}</div>
        {footer ? <footer className="modal-footer">{footer}</footer> : null}
      </div>
    </div>
  );
}

export function ConfirmModal({
  open,
  title,
  message,
  confirmLabel = '确认',
  tone = 'primary',
  busy,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: ReactNode;
  message: ReactNode;
  confirmLabel?: string;
  tone?: 'primary' | 'danger';
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}): JSX.Element | null {
  return (
    <Modal
      open={open}
      title={title}
      onClose={onCancel}
      footer={
        <>
          <Button onClick={onCancel} disabled={busy}>
            取消
          </Button>
          <Button variant={tone} onClick={onConfirm} loading={busy}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      <div>{message}</div>
    </Modal>
  );
}

/* ------------------------------------------------------------- copy/json */

export function CopyButton({ value, label = '复制', small }: { value: string; label?: string; small?: boolean }): JSX.Element {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(value);
      } else {
        const area = document.createElement('textarea');
        area.value = value;
        area.style.position = 'fixed';
        area.style.opacity = '0';
        document.body.appendChild(area);
        area.select();
        document.execCommand('copy');
        document.body.removeChild(area);
      }
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
    }
  };

  return (
    <Button size={small ? 'sm' : 'md'} onClick={() => void copy()} title={value}>
      {copied ? '已复制' : label}
    </Button>
  );
}

export function JsonBlock({ value, maxHeight }: { value: unknown; maxHeight?: number }): JSX.Element {
  return (
    <pre className="code-block" style={maxHeight ? { maxHeight } : undefined}>
      {typeof value === 'string' ? value : prettyJson(value)}
    </pre>
  );
}

export function DescriptionList({ items }: { items: Array<{ label: ReactNode; value: ReactNode }> }): JSX.Element {
  return (
    <dl className="kv-grid">
      {items.map((item, index) => (
        <div key={index} style={{ display: 'contents' }}>
          <dt>{item.label}</dt>
          <dd>{item.value}</dd>
        </div>
      ))}
    </dl>
  );
}

/* ------------------------------------------------------------------- tabs */

export interface TabItem<T extends string> {
  key: T;
  label: ReactNode;
}

export function Tabs<T extends string>({
  tabs,
  active,
  onChange,
}: {
  tabs: Array<TabItem<T>>;
  active: T;
  onChange: (key: T) => void;
}): JSX.Element {
  return (
    <div className="tabs" role="tablist">
      {tabs.map((tab) => (
        <button
          key={tab.key}
          type="button"
          role="tab"
          aria-selected={tab.key === active}
          className={cx(tab.key === active && 'active')}
          onClick={() => onChange(tab.key)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}

/* ------------------------------------------------------------- pagination */

export function Pagination({
  total,
  limit,
  offset,
  onChange,
}: {
  total: number;
  limit: number;
  offset: number;
  onChange: (offset: number) => void;
}): JSX.Element {
  const from = total === 0 ? 0 : offset + 1;
  const to = Math.min(offset + limit, total);
  return (
    <div className="pagination">
      <span>
        共 {total} 条{total > 0 ? ` · 第 ${from}-${to} 条` : ''}
      </span>
      <Button size="sm" disabled={offset <= 0} onClick={() => onChange(Math.max(0, offset - limit))}>
        上一页
      </Button>
      <Button size="sm" disabled={to >= total} onClick={() => onChange(offset + limit)}>
        下一页
      </Button>
    </div>
  );
}
