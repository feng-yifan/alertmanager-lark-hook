import { useEffect, useState, type FormEvent } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ApiError, createChannel, deleteChannel, errorMessage, listChannels, patchChannel, syncChannels } from '../api';
import {
  Banner,
  Button,
  Card,
  ConfirmModal,
  CopyButton,
  EmptyRow,
  ErrorState,
  Field,
  Input,
  Loading,
  Modal,
  PageHeader,
  Pagination,
  Select,
  StatusBadge,
  Switch,
} from '../components/ui';
import { useAsync, useDebounced } from '../hooks';
import { CHANNEL_KIND_LABEL, CHANNEL_KIND_VALUES, RECEIVE_ID_TYPE_LABEL, RECEIVE_ID_TYPE_VALUES } from '../labels';
import { useToast } from '../toast';
import type { Channel, ChannelKind, ReceiveIdType } from '../types';
import {
  formatDateTime,
  formatRelative,
  safeHttpUrl,
} from '../utils';

const LIMIT = 50;

interface CreateForm {
  name: string;
  receive_id: string;
  receive_id_type: ReceiveIdType;
  kind: ChannelKind;
  enabled: boolean;
}

const EMPTY_FORM: CreateForm = {
  name: '',
  receive_id: '',
  receive_id_type: 'chat_id',
  kind: 'group',
  enabled: true,
};

interface OneTimeCredential {
  channelName: string;
  token: string;
  hookUrl?: string;
}

export default function ChannelsPage(): JSX.Element {
  const toast = useToast();
  const navigate = useNavigate();

  const [search, setSearch] = useState('');
  const debouncedSearch = useDebounced(search, 300);
  const [enabledFilter, setEnabledFilter] = useState<'' | 'true' | 'false'>('');
  const [offset, setOffset] = useState(0);

  const [createOpen, setCreateOpen] = useState(false);
  const [form, setForm] = useState<CreateForm>(EMPTY_FORM);
  const [formError, setFormError] = useState<unknown>(null);
  const [creating, setCreating] = useState(false);

  const [syncing, setSyncing] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Channel | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [oneTime, setOneTime] = useState<OneTimeCredential | null>(null);

  useEffect(() => {
    setOffset(0);
  }, [debouncedSearch, enabledFilter]);

  const { data, loading, error, reload, refreshing } = useAsync(
    (signal) =>
      listChannels(
        {
          q: debouncedSearch || undefined,
          enabled: enabledFilter === '' ? undefined : enabledFilter === 'true',
          limit: LIMIT,
          offset,
        },
        signal,
      ),
    [debouncedSearch, enabledFilter, offset],
  );

  const items = data?.items ?? [];

  const toggleEnabled = async (channel: Channel, next: boolean) => {
    setBusyId(channel.id);
    try {
      await patchChannel(channel.id, { enabled: next });
      toast.success(next ? `已启用「${channel.name}」` : `已停用「${channel.name}」`);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusyId(null);
    }
  };

  const runSync = async () => {
    setSyncing(true);
    try {
      const result = await syncChannels({ create_missing: true, import_names: true });
      toast.success(
        `同步完成：发现 ${result.discovered} · 新建 ${result.created} · 更新 ${result.updated} · 跳过 ${result.skipped}`,
      );
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setSyncing(false);
    }
  };

  const submitCreate = async (event?: FormEvent) => {
    event?.preventDefault();
    setFormError(null);
    if (!form.name.trim()) {
      setFormError(new Error('渠道名称不能为空'));
      return;
    }
    if (form.name.trim().length > 64) {
      setFormError(new Error('渠道名称长度不能超过 64 个字符'));
      return;
    }
    if (!form.receive_id.trim()) {
      setFormError(new Error('receive_id 不能为空'));
      return;
    }
    setCreating(true);
    try {
      const created = await createChannel({
        name: form.name.trim(),
        receive_id: form.receive_id.trim(),
        receive_id_type: form.receive_id_type,
        kind: form.kind,
        enabled: form.enabled,
      });
      toast.success(`渠道「${created.name}」已创建`);
      setCreateOpen(false);
      setForm(EMPTY_FORM);
      if (created.hook_token) {
        setOneTime({
          channelName: created.name,
          token: created.hook_token,
          ...(created.hook_url ? { hookUrl: created.hook_url } : {}),
        });
      }
      reload();
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 409) {
        setFormError(new Error(`创建失败（409 conflict）：${cause.message}`));
      } else {
        setFormError(cause);
      }
    } finally {
      setCreating(false);
    }
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await deleteChannel(deleteTarget.id);
      toast.success(`渠道「${deleteTarget.name}」已删除`);
      setDeleteTarget(null);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
      setDeleteTarget(null);
    } finally {
      setDeleting(false);
    }
  };

  return (
    <>
      <PageHeader
        title="渠道"
        subtitle="机器人可触达的群聊 / 单聊；每个渠道可签发多个 Hook"
        actions={
          <>
            <Button onClick={() => void runSync()} loading={syncing}>
              从飞书同步
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                setForm(EMPTY_FORM);
                setFormError(null);
                setCreateOpen(true);
              }}
            >
              新建渠道
            </Button>
            <Button onClick={reload} loading={refreshing}>
              刷新
            </Button>
          </>
        }
      />

      {oneTime ? (
        <Banner tone="warn" onClose={() => setOneTime(null)}>
          <div>
            <strong>⚠ 请立即保存渠道「{oneTime.channelName}」的默认 Hook 凭证 —— 明文 token 只显示一次。</strong>
            <div className="mt-8">
              <div className="small mb-8">Token</div>
              <div className="copy-row">
                <span className="copy-value">{oneTime.token}</span>
                <CopyButton value={oneTime.token} small />
              </div>
              {oneTime.hookUrl ? (
                <>
                  <div className="small mb-8 mt-8">Hook URL</div>
                  <div className="copy-row">
                    <span className="copy-value">{oneTime.hookUrl}</span>
                    <CopyButton value={oneTime.hookUrl} small />
                  </div>
                </>
              ) : null}
            </div>
          </div>
        </Banner>
      ) : null}

      <Card
        tight
        title="渠道列表"
        subtitle={`共 ${data?.total ?? 0} 个渠道`}
        actions={
          <>
            <Select
              value={enabledFilter}
              onChange={(event) => setEnabledFilter(event.target.value as '' | 'true' | 'false')}
              style={{ width: 130 }}
            >
              <option value="">全部状态</option>
              <option value="true">已启用</option>
              <option value="false">已停用</option>
            </Select>
            <Input
              placeholder="搜索名称 / 接收方 ID"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              style={{ width: 220 }}
            />
          </>
        }
      >
        {loading && !data ? (
          <Loading />
        ) : error && !data ? (
          <ErrorState error={error} onRetry={reload} />
        ) : (
          <>
            <div className="table-wrap">
              <table className="data">
                <thead>
                  <tr>
                    <th>名称</th>
                    <th>接收方</th>
                    <th>类型</th>
                    <th>来源</th>
                    <th>机器人可发送</th>
                    <th className="num">Hook</th>
                    <th>启用</th>
                    <th>更新时间</th>
                    <th className="actions">操作</th>
                  </tr>
                </thead>
                <tbody>
                  {items.length === 0 ? (
                    <EmptyRow
                      colSpan={9}
                      title="暂无渠道"
                      description="点击「从飞书同步」导入机器人可触达的群聊，或手动新建渠道。"
                    />
                  ) : (
                    items.map((channel) => (
                      <tr key={channel.id}>
                        <td>
                          <div className="row tight">
                            {channel.avatar_url ? (
                              <img
                                src={safeHttpUrl(channel.avatar_url) ?? undefined}
                                alt=""
                                width={20}
                                height={20}
                                style={{ borderRadius: 4, objectFit: 'cover' }}
                              />
                            ) : null}
                            <Link to={`/channels/${channel.id}`}>{channel.name}</Link>
                          </div>
                        </td>
                        <td>
                          <div className="mono break-all">{channel.receive_id}</div>
                          <div className="faint small">
                            {RECEIVE_ID_TYPE_LABEL[channel.receive_id_type] ?? channel.receive_id_type}
                          </div>
                        </td>
                        <td>
                          <StatusBadge kind="channel_kind" value={channel.kind} />
                        </td>
                        <td>
                          <StatusBadge kind="channel_source" value={channel.source} />
                        </td>
                        <td>
                          <span className={channel.bot_can_send ? 'badge badge-ok' : 'badge badge-muted'}>
                            {channel.bot_can_send ? '可发送' : '不可发送'}
                          </span>
                        </td>
                        <td className="num">{channel.hook_count}</td>
                        <td>
                          <Switch
                            checked={channel.enabled}
                            disabled={busyId === channel.id}
                            onChange={(next) => void toggleEnabled(channel, next)}
                          />
                        </td>
                        <td className="nowrap" title={formatDateTime(channel.updated_at)}>
                          {formatRelative(channel.updated_at)}
                        </td>
                        <td className="actions">
                          <div className="row tight" style={{ justifyContent: 'flex-end' }}>
                            <Button size="sm" onClick={() => navigate(`/channels/${channel.id}`)}>
                              详情
                            </Button>
                            <Button
                              size="sm"
                              variant="danger"
                              onClick={() => setDeleteTarget(channel)}
                            >
                              删除
                            </Button>
                          </div>
                        </td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
            {data ? (
              <Pagination total={data.total} limit={data.limit} offset={data.offset} onChange={setOffset} />
            ) : null}
          </>
        )}
      </Card>

      <Modal
        open={createOpen}
        title="新建渠道"
        onClose={() => setCreateOpen(false)}
        footer={
          <>
            <Button onClick={() => setCreateOpen(false)} disabled={creating}>
              取消
            </Button>
            <Button variant="primary" loading={creating} onClick={() => void submitCreate()}>
              创建
            </Button>
          </>
        }
      >
        <form onSubmit={(event) => void submitCreate(event)}>
          {formError ? <Banner tone="error">{errorMessage(formError)}</Banner> : null}
          <Field label="渠道名称" required hint="≤ 64 字符，可与飞书群名不同">
            <Input
              value={form.name}
              maxLength={80}
              required
              onChange={(event) => setForm({ ...form, name: event.target.value })}
            />
          </Field>
          <Field label="接收方 ID" required hint="群聊填 oc_ 开头的群 ID；用户填 open_id">
            <Input
              className="mono"
              value={form.receive_id}
              required
              onChange={(event) => setForm({ ...form, receive_id: event.target.value })}
            />
          </Field>
          <div className="form-grid">
            <Field label="接收方 ID 类型" required>
              <Select
                value={form.receive_id_type}
                onChange={(event) => setForm({ ...form, receive_id_type: event.target.value as ReceiveIdType })}
              >
                {RECEIVE_ID_TYPE_VALUES.map((value) => (
                  <option key={value} value={value}>
                    {RECEIVE_ID_TYPE_LABEL[value]}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label="类型" hint="选「群聊」时接收方 ID 必须是群 ID">
              <Select value={form.kind} onChange={(event) => setForm({ ...form, kind: event.target.value as ChannelKind })}>
                {CHANNEL_KIND_VALUES.map((value) => (
                  <option key={value} value={value}>
                    {CHANNEL_KIND_LABEL[value]}
                  </option>
                ))}
              </Select>
            </Field>
          </div>
          <Switch
            checked={form.enabled}
            onChange={(next) => setForm({ ...form, enabled: next })}
            label="创建后立即启用"
          />
          <p className="faint small mt-8 mb-0">
            创建时会自动生成默认通知策略与 1 个默认 Hook，响应中的明文 token 仅返回一次。
          </p>
        </form>
      </Modal>

      <ConfirmModal
        open={deleteTarget !== null}
        title="删除渠道"
        tone="danger"
        confirmLabel="删除"
        busy={deleting}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={() => void confirmDelete()}
        message={
          <>
            将删除渠道「{deleteTarget?.name}」及其 hooks 与通知策略。若服务端因已存在投递记录等原因拒绝，会返回 409
            conflict 并保持数据不变（当前后台未提供「强制删除」入口）。
          </>
        }
      />
    </>
  );
}
