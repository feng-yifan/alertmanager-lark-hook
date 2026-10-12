/**
 * Single typed fetch client for the REST contract in `docs/api.md`.
 *
 *  - every call goes through `request()`, which prefixes `/api/v1`;
 *  - `credentials: 'same-origin'` carries the `alkh_session` HttpOnly cookie;
 *  - a non-2xx response is parsed from the `{"error":{"code","message"}}` envelope
 *    and thrown as `ApiError`;
 *  - `401` on an authenticated endpoint notifies the session layer, which sends the
 *    user to `/login` (the login/session probes are exempt so the login form can
 *    surface `invalid_credentials`).
 */
import type {
  AlertGroup,
  AlertGroupListParams,
  AppSettings,
  AppSettingsUpdate,
  AuditLog,
  AuditLogListParams,
  CardTemplate,
  CardTemplateCreate,
  CardTemplateKind,
  CardTemplatePatch,
  CardTemplateSendTestRequest,
  ChangePasswordRequest,
  Channel,
  ChannelCreateRequest,
  ChannelListParams,
  ChannelPatchRequest,
  ChannelSyncRequest,
  ChannelSyncResult,
  Delivery,
  DeliveryListParams,
  Hook,
  HookCreated,
  HookTokenResponse,
  LoginRequest,
  LoginResponse,
  Meta,
  NotificationPolicy,
  NotificationPolicyInput,
  Overview,
  Paginated,
  PreviewRequest,
  PreviewResponse,
  SessionResponse,
  SettingsCheckResponse,
  TestMessageRequest,
  TestMessageResponse,
  ValidateRequest,
  ValidateResponse,
} from './types';

export const API_BASE = '/api/v1';

/** Thrown for every non-2xx response (and for malformed bodies). */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly details?: unknown;

  constructor(status: number, code: string, message: string, details?: unknown) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

export type QueryValue = string | number | boolean | null | undefined;

export interface RequestOptions {
  method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
  query?: Record<string, QueryValue>;
  body?: unknown;
  signal?: AbortSignal;
}

type UnauthorizedHandler = () => void;

let unauthorizedHandler: UnauthorizedHandler | null = null;

/** Registered once by the session provider so a 401 can drive a router redirect. */
export function setUnauthorizedHandler(handler: UnauthorizedHandler | null): void {
  unauthorizedHandler = handler;
}

/** Endpoints whose 401 is expected/probed rather than a lost session. */
const UNAUTHENTICATED_EXEMPT = new Set<string>(['/auth/login', '/auth/session']);

export function buildUrl(path: string, query?: Record<string, QueryValue>): string {
  const url = `${API_BASE}${path.startsWith('/') ? path : `/${path}`}`;
  if (!query) return url;
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null || value === '') continue;
    search.set(key, String(value));
  }
  const qs = search.toString();
  return qs ? `${url}?${qs}` : url;
}

function isErrorEnvelope(value: unknown): value is { error: { code?: string; message?: string; details?: unknown } } {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = (value as { error?: unknown }).error;
  return typeof candidate === 'object' && candidate !== null;
}

export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = 'GET', query, body, signal } = options;
  const headers: Record<string, string> = { Accept: 'application/json' };
  if (body !== undefined) headers['Content-Type'] = 'application/json';

  let response: Response;
  try {
    response = await fetch(buildUrl(path, query), {
      method,
      headers,
      credentials: 'same-origin',
      body: body === undefined ? undefined : JSON.stringify(body),
      ...(signal ? { signal } : {}),
    });
  } catch (cause) {
    if (cause instanceof DOMException && cause.name === 'AbortError') throw cause;
    throw new ApiError(0, 'network_error', '无法连接到服务器，请检查网络或后端服务是否已启动。', cause);
  }

  const text = await response.text();
  let payload: unknown = null;
  if (text.length > 0) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = null;
    }
  }

  if (!response.ok) {
    let code = `http_${response.status}`;
    let message = response.statusText || `请求失败（HTTP ${response.status}）`;
    let details: unknown;
    if (isErrorEnvelope(payload)) {
      code = payload.error.code ?? code;
      message = payload.error.message ?? message;
      details = payload.error.details;
    } else if (text.length > 0 && text.length < 500) {
      message = text;
    }
    if (response.status === 401 && !UNAUTHENTICATED_EXEMPT.has(path)) {
      unauthorizedHandler?.();
    }
    throw new ApiError(response.status, code, message, details);
  }

  if (payload === null && text.length > 0) {
    throw new ApiError(response.status, 'invalid_response', '服务器返回了非 JSON 响应。');
  }
  return payload as T;
}

/* ------------------------------------------------------------------ */
/* 0. meta                                                             */
/* ------------------------------------------------------------------ */

export function getMeta(signal?: AbortSignal): Promise<Meta> {
  return request<Meta>('/meta', { signal });
}

/* ------------------------------------------------------------------ */
/* 1. auth                                                             */
/* ------------------------------------------------------------------ */

export function login(body: LoginRequest): Promise<LoginResponse> {
  return request<LoginResponse>('/auth/login', { method: 'POST', body });
}

export function logout(): Promise<void> {
  return request<void>('/auth/logout', { method: 'POST' });
}

export function getSession(signal?: AbortSignal): Promise<SessionResponse> {
  return request<SessionResponse>('/auth/session', { signal });
}

export function changePassword(body: ChangePasswordRequest): Promise<void> {
  return request<void>('/auth/password', { method: 'POST', body });
}

/* ------------------------------------------------------------------ */
/* 2. settings                                                         */
/* ------------------------------------------------------------------ */

export function getSettings(signal?: AbortSignal): Promise<AppSettings> {
  return request<AppSettings>('/settings', { signal });
}

export function updateSettings(body: AppSettingsUpdate): Promise<AppSettings> {
  return request<AppSettings>('/settings', { method: 'PUT', body });
}

export function checkSettings(): Promise<SettingsCheckResponse> {
  return request<SettingsCheckResponse>('/settings/check', { method: 'POST' });
}

/* ------------------------------------------------------------------ */
/* 3. channels                                                         */
/* ------------------------------------------------------------------ */

export function listChannels(
  params: ChannelListParams = {},
  signal?: AbortSignal,
): Promise<Paginated<Channel>> {
  return request<Paginated<Channel>>('/channels', { query: { ...params }, signal });
}

export function createChannel(body: ChannelCreateRequest): Promise<Channel> {
  return request<Channel>('/channels', { method: 'POST', body });
}

export function getChannel(id: string, signal?: AbortSignal): Promise<Channel> {
  return request<Channel>(`/channels/${id}`, { signal });
}

export function patchChannel(id: string, body: ChannelPatchRequest): Promise<Channel> {
  return request<Channel>(`/channels/${id}`, { method: 'PATCH', body });
}

export function deleteChannel(id: string, force = false): Promise<void> {
  return request<void>(`/channels/${id}`, { method: 'DELETE', query: { force: force ? true : undefined } });
}

export function syncChannels(body: ChannelSyncRequest = { create_missing: true, import_names: true }): Promise<ChannelSyncResult> {
  return request<ChannelSyncResult>('/channels/sync', { method: 'POST', body });
}

export function getChannelPolicy(id: string, signal?: AbortSignal): Promise<NotificationPolicy> {
  return request<NotificationPolicy>(`/channels/${id}/policy`, { signal });
}

export function putChannelPolicy(id: string, body: NotificationPolicyInput): Promise<NotificationPolicy> {
  return request<NotificationPolicy>(`/channels/${id}/policy`, { method: 'PUT', body });
}

export function sendTestMessage(id: string, body: TestMessageRequest): Promise<TestMessageResponse> {
  return request<TestMessageResponse>(`/channels/${id}/test-message`, { method: 'POST', body });
}

/* ------------------------------------------------------------------ */
/* 4. hooks                                                            */
/* ------------------------------------------------------------------ */

export function listHooks(signal?: AbortSignal): Promise<Paginated<Hook>> {
  return request<Paginated<Hook>>('/hooks', { signal });
}

export function createHook(body: { channel_id: string; name: string }): Promise<HookCreated> {
  return request<HookCreated>('/hooks', { method: 'POST', body });
}

export function patchHook(id: string, body: { name?: string; enabled?: boolean }): Promise<Hook> {
  return request<Hook>(`/hooks/${id}`, { method: 'PATCH', body });
}

export function rotateHook(id: string): Promise<HookTokenResponse> {
  return request<HookTokenResponse>(`/hooks/${id}/rotate`, { method: 'POST' });
}

export function deleteHook(id: string): Promise<void> {
  return request<void>(`/hooks/${id}`, { method: 'DELETE' });
}

/* ------------------------------------------------------------------ */
/* 5. card templates                                                   */
/* ------------------------------------------------------------------ */

export function listCardTemplates(
  params: { kind?: CardTemplateKind | '' } = {},
  signal?: AbortSignal,
): Promise<Paginated<CardTemplate>> {
  return request<Paginated<CardTemplate>>('/card-templates', {
    query: { kind: params.kind ? params.kind : undefined },
    signal,
  });
}

export function createCardTemplate(body: CardTemplateCreate): Promise<CardTemplate> {
  return request<CardTemplate>('/card-templates', { method: 'POST', body });
}

export function getCardTemplate(id: string, signal?: AbortSignal): Promise<CardTemplate> {
  return request<CardTemplate>(`/card-templates/${id}`, { signal });
}

export function patchCardTemplate(id: string, body: CardTemplatePatch): Promise<CardTemplate> {
  return request<CardTemplate>(`/card-templates/${id}`, { method: 'PATCH', body });
}

export function deleteCardTemplate(id: string): Promise<void> {
  return request<void>(`/card-templates/${id}`, { method: 'DELETE' });
}

export function validateCardTemplate(body: ValidateRequest): Promise<ValidateResponse> {
  return request<ValidateResponse>('/card-templates/validate', { method: 'POST', body });
}

export function previewCardTemplate(body: PreviewRequest): Promise<PreviewResponse> {
  return request<PreviewResponse>('/card-templates/preview', { method: 'POST', body });
}

export function sendTestCardTemplate(id: string, body: CardTemplateSendTestRequest): Promise<TestMessageResponse> {
  return request<TestMessageResponse>(`/card-templates/${id}/send-test`, { method: 'POST', body });
}

/* ------------------------------------------------------------------ */
/* 6. alert groups / deliveries                                        */
/* ------------------------------------------------------------------ */

export function listAlertGroups(
  params: AlertGroupListParams = {},
  signal?: AbortSignal,
): Promise<Paginated<AlertGroup>> {
  return request<Paginated<AlertGroup>>('/alert-groups', { query: { ...params }, signal });
}

export function getAlertGroup(id: string, signal?: AbortSignal): Promise<AlertGroup> {
  return request<AlertGroup>(`/alert-groups/${id}`, { signal });
}

export function deleteAlertGroup(id: string): Promise<void> {
  return request<void>(`/alert-groups/${id}`, { method: 'DELETE' });
}

export function listDeliveries(
  params: DeliveryListParams = {},
  signal?: AbortSignal,
): Promise<Paginated<Delivery>> {
  return request<Paginated<Delivery>>('/deliveries', { query: { ...params }, signal });
}

export function retryDelivery(id: string): Promise<Delivery> {
  return request<Delivery>(`/deliveries/${id}/retry`, { method: 'POST' });
}

export function revokeDelivery(id: string): Promise<Delivery> {
  return request<Delivery>(`/deliveries/${id}/revoke`, { method: 'POST' });
}

/* ------------------------------------------------------------------ */
/* 7. overview / audit logs                                            */
/* ------------------------------------------------------------------ */

export function getOverview(signal?: AbortSignal): Promise<Overview> {
  return request<Overview>('/overview', { signal });
}

export function listAuditLogs(
  params: AuditLogListParams = {},
  signal?: AbortSignal,
): Promise<Paginated<AuditLog>> {
  return request<Paginated<AuditLog>>('/audit-logs', { query: { ...params }, signal });
}

/* ------------------------------------------------------------------ */
/* helpers                                                             */
/* ------------------------------------------------------------------ */

export function errorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 401) return '登录状态已失效，请重新登录。';
    return error.message || error.code;
  }
  if (error instanceof Error) return error.message;
  return String(error);
}

/** Best-effort field level details for `validation_failed` responses. */
export function errorDetails(error: unknown): unknown {
  return error instanceof ApiError ? error.details : undefined;
}
