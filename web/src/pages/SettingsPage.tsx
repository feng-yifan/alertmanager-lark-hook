import { useEffect, useState, type FormEvent } from 'react';
import {
  changePassword,
  checkSettings,
  errorMessage,
  getSettings,
  listAuditLogs,
  updateSettings,
} from '../api';
import {
  Badge,
  Banner,
  Button,
  Card,
  ConfirmModal,
  EmptyRow,
  ErrorState,
  Field,
  Input,
  JsonBlock,
  Loading,
  Modal,
  PageHeader,
  Pagination,
  Select,
  StatusBadge,
} from '../components/ui';
import { useAsync, useDebounced } from '../hooks';
import { useSession } from '../session';
import { useToast } from '../toast';
import type { AuditLog, SettingsCheckResponse } from '../types';
import { actorText, formatDateTime } from '../utils';

const AUDIT_LIMIT = 50;

interface SettingsForm {
  lark_app_id: string;
  lark_base_url: string;
  public_base_url: string;
  alert_group_days: string;
  delivery_days: string;
  audit_days: string;
}

const EMPTY_FORM: SettingsForm = {
  lark_app_id: '',
  lark_base_url: 'https://open.feishu.cn',
  public_base_url: '',
  alert_group_days: '30',
  delivery_days: '30',
  audit_days: '14',
};

type SecretMode = 'unchanged' | 'set' | 'clear';

export default function SettingsPage(): JSX.Element {
  const toast = useToast();
  const session = useSession();

  /* ------------------------------------------------------------- settings */

  const { data: settings, loading, error, reload, refreshing } = useAsync((signal) => getSettings(signal), []);

  const [form, setForm] = useState<SettingsForm>(EMPTY_FORM);
  const [secretMode, setSecretMode] = useState<SecretMode>('unchanged');
  const [secretValue, setSecretValue] = useState('');
  const [clearConfirm, setClearConfirm] = useState(false);
  const [saving, setSaving] = useState(false);
  const [checking, setChecking] = useState(false);
  const [checkResult, setCheckResult] = useState<SettingsCheckResponse | null>(null);
  const [checkError, setCheckError] = useState<unknown>(null);

  useEffect(() => {
    if (!settings) return;
    setForm({
      lark_app_id: settings.lark_app_id ?? '',
      lark_base_url: settings.lark_base_url ?? 'https://open.feishu.cn',
      public_base_url: settings.public_base_url ?? '',
      alert_group_days: String(settings.retention.alert_group_days),
      delivery_days: String(settings.retention.delivery_days),
      audit_days: String(settings.retention.audit_days),
    });
    setSecretMode('unchanged');
    setSecretValue('');
  }, [settings]);

  const saveSettings = async (event?: FormEvent) => {
    event?.preventDefault();
    const alertDays = Number(form.alert_group_days);
    const deliveryDays = Number(form.delivery_days);
    const auditDays = Number(form.audit_days);
    for (const [label, value] of [
      ['告警分组保留天数', alertDays],
      ['投递记录保留天数', deliveryDays],
      ['审计日志保留天数', auditDays],
    ] as const) {
      if (!Number.isInteger(value) || value < 1) {
        toast.error(`${label}需为 ≥ 1 的整数`);
        return;
      }
    }
    setSaving(true);
    try {
      await updateSettings({
        lark_app_id: form.lark_app_id.trim() || null,
        app_secret: secretMode === 'unchanged' ? null : secretMode === 'clear' ? '' : secretValue,
        lark_base_url: form.lark_base_url.trim(),
        public_base_url: form.public_base_url.trim() || null,
        retention: { alert_group_days: alertDays, delivery_days: deliveryDays, audit_days: auditDays },
      });
      toast.success('设置已保存');
      setSecretMode('unchanged');
      setSecretValue('');
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  };

  const runCheck = async () => {
    setChecking(true);
    setCheckError(null);
    setCheckResult(null);
    try {
      const result = await checkSettings();
      setCheckResult(result);
      if (result.ok) toast.success('飞书凭据可用');
      else toast.error(result.message ?? `飞书返回错误 ${result.error_code ?? ''}`);
    } catch (cause) {
      setCheckError(cause);
      toast.error(errorMessage(cause));
    } finally {
      setChecking(false);
    }
  };

  /* ------------------------------------------------------------- password */

  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [changing, setChanging] = useState(false);

  const submitPassword = async (event: FormEvent) => {
    event.preventDefault();
    if (!currentPassword || !newPassword) {
      toast.error('请填写当前密码与新密码');
      return;
    }
    if (newPassword !== confirmPassword) {
      toast.error('两次输入的新密码不一致');
      return;
    }
    if (newPassword.length < 8) {
      toast.error('新密码至少 8 个字符');
      return;
    }
    setChanging(true);
    try {
      await changePassword({ current_password: currentPassword, new_password: newPassword });
      toast.success('密码已修改');
      setCurrentPassword('');
      setNewPassword('');
      setConfirmPassword('');
      session.markPasswordChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setChanging(false);
    }
  };

  /* ------------------------------------------------------------ audit logs */

  const [actionFilter, setActionFilter] = useState('');
  const [outcomeFilter, setOutcomeFilter] = useState('');
  const [auditOffset, setAuditOffset] = useState(0);
  const debouncedAction = useDebounced(actionFilter, 300);

  useEffect(() => {
    setAuditOffset(0);
  }, [debouncedAction, outcomeFilter]);

  const {
    data: auditPage,
    loading: auditLoading,
    error: auditError,
    reload: reloadAudit,
    refreshing: auditRefreshing,
  } = useAsync(
    (signal) =>
      listAuditLogs(
        {
          action: debouncedAction || undefined,
          outcome: outcomeFilter || undefined,
          limit: AUDIT_LIMIT,
          offset: auditOffset,
        },
        signal,
      ),
    [debouncedAction, outcomeFilter, auditOffset],
  );

  const [auditDetail, setAuditDetail] = useState<AuditLog | null>(null);
  const auditItems = auditPage?.items ?? [];

  return (
    <>
      <PageHeader
        title="设置"
        subtitle="飞书应用凭据、数据保留、管理员密码与审计日志"
        actions={
          <Button onClick={reload} loading={refreshing}>
            刷新设置
          </Button>
        }
      />

      {session.user?.must_change_password ? (
        <div className="callout danger mb-16">
          当前账号需要修改初始密码（<code>must_change_password = true</code>）。请在下方「修改密码」中设置新密码。
        </div>
      ) : null}

      {loading && !settings ? <Loading label="正在加载设置…" /> : null}
      {error && !settings ? <ErrorState error={error} onRetry={reload} /> : null}

      {settings ? (
        <Card
          title="飞书应用设置"
          subtitle="GET /api/v1/settings 永不回显 app_secret，只返回 app_secret_set"
          actions={
            <>
              <Button onClick={() => void runCheck()} loading={checking}>
                测试飞书凭据
              </Button>
              <Button variant="primary" loading={saving} onClick={() => void saveSettings()}>
                保存设置
              </Button>
            </>
          }
        >
          <form onSubmit={(event) => void saveSettings(event)}>
            <div className="form-grid">
              <Field label="lark_app_id" hint="形如 cli_*（仅提示，不强制，business-rules §6）">
                <Input
                  className="mono"
                  value={form.lark_app_id}
                  placeholder="cli_xxxxxxxxxxxxxxxx"
                  onChange={(event) => setForm({ ...form, lark_app_id: event.target.value })}
                />
              </Field>
              <Field
                label="lark_app_secret（write-only）"
                hint="留空表示不修改；填入新值表示覆盖；点击「清除密钥」提交空串以清除"
              >
                <div className="row tight">
                  <Input
                    type="password"
                    autoComplete="new-password"
                    value={secretValue}
                    disabled={secretMode === 'clear'}
                    placeholder={
                      secretMode === 'clear'
                        ? '保存后将清除'
                        : settings.app_secret_set
                          ? '已配置 · 留空则不修改'
                          : '未配置 · 请输入 app_secret'
                    }
                    onChange={(event) => {
                      setSecretValue(event.target.value);
                      setSecretMode(event.target.value ? 'set' : 'unchanged');
                    }}
                  />
                  {secretMode === 'clear' ? (
                    <Button size="sm" onClick={() => setSecretMode('unchanged')}>
                      撤销清除
                    </Button>
                  ) : (
                    <Button size="sm" variant="danger" disabled={!settings.app_secret_set} onClick={() => setClearConfirm(true)}>
                      清除密钥
                    </Button>
                  )}
                </div>
                <div className="hint">
                  {settings.app_secret_set ? (
                    <Badge tone="ok">已配置</Badge>
                  ) : (
                    <Badge tone="warn">未配置</Badge>
                  )}{' '}
                  {secretMode === 'set' ? '· 保存后将写入新密钥' : null}
                  {secretMode === 'clear' ? '· 保存后将清除已存储的密钥' : null}
                </div>
              </Field>
            </div>

            <div className="form-grid">
              <Field label="lark_base_url" hint="飞书开放平台基址，默认 https://open.feishu.cn">
                <Input
                  className="mono"
                  value={form.lark_base_url}
                  onChange={(event) => setForm({ ...form, lark_base_url: event.target.value })}
                />
              </Field>
              <Field label="public_base_url" hint="用于生成 Hook 绝对地址：{public_base_url}/api/v1/hooks/alertmanager/{token}">
                <Input
                  className="mono"
                  value={form.public_base_url}
                  placeholder="https://alkh.example.com"
                  onChange={(event) => setForm({ ...form, public_base_url: event.target.value })}
                />
              </Field>
            </div>

            <h3 className="mb-8 mt-8">数据保留（retention，单位：天）</h3>
            <div className="grid-3">
              <Field label="alert_group_days">
                <Input
                  type="number"
                  min={1}
                  value={form.alert_group_days}
                  onChange={(event) => setForm({ ...form, alert_group_days: event.target.value })}
                />
              </Field>
              <Field label="delivery_days">
                <Input
                  type="number"
                  min={1}
                  value={form.delivery_days}
                  onChange={(event) => setForm({ ...form, delivery_days: event.target.value })}
                />
              </Field>
              <Field label="audit_days">
                <Input
                  type="number"
                  min={1}
                  value={form.audit_days}
                  onChange={(event) => setForm({ ...form, audit_days: event.target.value })}
                />
              </Field>
            </div>

            <div className="row">
              <Button variant="primary" type="submit" loading={saving}>
                保存设置
              </Button>
              <span className="faint small">app_secret 传 null = 不变，传 "" = 清除（docs/api.md §2）</span>
            </div>
          </form>

          {checkResult ? (
            <div className="mt-16">
              {checkResult.ok ? (
                <Banner tone="success">
                  凭据可用 · 机器人：{checkResult.bot_name ?? '未知'} · 可触达群 {checkResult.chat_count ?? 0} 个
                </Banner>
              ) : (
                <Banner tone="error">
                  凭据不可用 · error_code：{String(checkResult.error_code ?? '—')} · {checkResult.message ?? '无详细信息'}
                </Banner>
              )}
            </div>
          ) : null}
          {checkError ? <Banner tone="error">{errorMessage(checkError)}</Banner> : null}
        </Card>
      ) : null}

      <Card title="修改密码" subtitle="POST /api/v1/auth/password">
        <form onSubmit={(event) => void submitPassword(event)} style={{ maxWidth: 420 }}>
          <Field label="当前密码" required>
            <Input
              type="password"
              autoComplete="current-password"
              value={currentPassword}
              onChange={(event) => setCurrentPassword(event.target.value)}
            />
          </Field>
          <Field label="新密码" required hint="至少 8 个字符">
            <Input
              type="password"
              autoComplete="new-password"
              value={newPassword}
              onChange={(event) => setNewPassword(event.target.value)}
            />
          </Field>
          <Field label="确认新密码" required>
            <Input
              type="password"
              autoComplete="new-password"
              value={confirmPassword}
              onChange={(event) => setConfirmPassword(event.target.value)}
            />
          </Field>
          <Button variant="primary" type="submit" loading={changing}>
            修改密码
          </Button>
        </form>
      </Card>

      <Card
        title="审计日志"
        subtitle={`共 ${auditPage?.total ?? 0} 条（只追加；detail 为任意 JSON）`}
        tight
        actions={
          <Button onClick={reloadAudit} loading={auditRefreshing}>
            刷新
          </Button>
        }
      >
        <div className="toolbar">
          <Field label="action">
            <Input
              className="mono"
              placeholder="如 message.sent"
              value={actionFilter}
              onChange={(event) => setActionFilter(event.target.value)}
              style={{ width: 200 }}
            />
          </Field>
          <Field label="outcome">
            <Select value={outcomeFilter} onChange={(event) => setOutcomeFilter(event.target.value)}>
              <option value="">全部</option>
              <option value="ok">ok</option>
              <option value="skipped">skipped</option>
              <option value="failed">failed</option>
            </Select>
          </Field>
          <Button
            size="sm"
            disabled={!actionFilter && !outcomeFilter}
            onClick={() => {
              setActionFilter('');
              setOutcomeFilter('');
            }}
          >
            清除筛选
          </Button>
        </div>

        {auditLoading && !auditPage ? (
          <Loading />
        ) : auditError && !auditPage ? (
          <ErrorState error={auditError} onRetry={reloadAudit} />
        ) : (
          <>
            <div className="table-wrap">
              <table className="data">
                <thead>
                  <tr>
                    <th>时间</th>
                    <th>操作人</th>
                    <th>动作</th>
                    <th>目标</th>
                    <th>结果</th>
                    <th className="actions">详情</th>
                  </tr>
                </thead>
                <tbody>
                  {auditItems.length === 0 ? (
                    <EmptyRow
                      colSpan={6}
                      title="暂无审计日志"
                      description="Hook 接收、消息发送、渠道同步、策略变更等都会写入审计。"
                    />
                  ) : (
                    auditItems.map((log) => (
                      <tr key={log.id}>
                        <td className="nowrap small">{formatDateTime(log.at)}</td>
                        <td className="small">{actorText(log.actor)}</td>
                        <td className="mono">{log.action}</td>
                        <td className="mono break-all small">{log.target ?? '—'}</td>
                        <td>
                          <StatusBadge kind="outcome" value={log.outcome} />
                        </td>
                        <td className="actions">
                          <Button size="sm" onClick={() => setAuditDetail(log)}>
                            查看
                          </Button>
                        </td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
            {auditPage ? (
              <Pagination
                total={auditPage.total}
                limit={auditPage.limit || AUDIT_LIMIT}
                offset={auditPage.offset}
                onChange={setAuditOffset}
              />
            ) : null}
          </>
        )}
      </Card>

      <ConfirmModal
        open={clearConfirm}
        title="清除飞书 app_secret"
        tone="danger"
        confirmLabel="清除"
        onCancel={() => setClearConfirm(false)}
        onConfirm={() => {
          setSecretMode('clear');
          setSecretValue('');
          setClearConfirm(false);
          toast.warn('已标记为清除，点击「保存设置」后生效');
        }}
        message={<>保存后服务端将删除已加密存储的 app_secret（提交空串）。在重新配置前，消息发送会失败。</>}
      />

      <Modal open={auditDetail !== null} title={`审计详情 · ${auditDetail?.action ?? ''}`} onClose={() => setAuditDetail(null)} wide>
        {auditDetail ? (
          <>
            <div className="kv-grid mb-16">
              <dt>时间</dt>
              <dd>{formatDateTime(auditDetail.at)}</dd>
              <dt>操作人</dt>
              <dd>{actorText(auditDetail.actor)}</dd>
              <dt>目标</dt>
              <dd className="mono break-all">{auditDetail.target ?? '—'}</dd>
              <dt>结果</dt>
              <dd>
                <StatusBadge kind="outcome" value={auditDetail.outcome} />
              </dd>
            </div>
            <JsonBlock value={auditDetail.detail} maxHeight={420} />
          </>
        ) : null}
      </Modal>
    </>
  );
}
