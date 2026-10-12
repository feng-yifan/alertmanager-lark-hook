import { useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { deleteAlertGroup, errorMessage, listAlertGroups, listChannels } from '../api';
import {
  Badge,
  Button,
  Card,
  ConfirmModal,
  EmptyRow,
  ErrorState,
  Field,
  Input,
  Loading,
  PageHeader,
  Pagination,
  Select,
  StatusBadge,
} from '../components/ui';
import { useAsync, useDebounced } from '../hooks';
import { GROUP_STATUS_LABEL } from '../labels';
import { useToast } from '../toast';
import type { AlertGroup } from '../types';
import { formatDateTime, formatRelative } from '../utils';

const LIMIT = 50;

export default function AlertGroupsPage(): JSX.Element {
  const toast = useToast();
  const navigate = useNavigate();

  const [status, setStatus] = useState<'' | 'firing' | 'resolved'>('');
  const [channelId, setChannelId] = useState('');
  const [severityInput, setSeverityInput] = useState('');
  const [search, setSearch] = useState('');
  const severity = useDebounced(severityInput, 300);
  const debouncedSearch = useDebounced(search, 300);
  const [offset, setOffset] = useState(0);

  useEffect(() => {
    setOffset(0);
  }, [status, channelId, severity, debouncedSearch]);

  const { data, loading, error, reload, refreshing } = useAsync(
    (signal) =>
      listAlertGroups(
        {
          status: status || undefined,
          channel_id: channelId || undefined,
          severity: severity || undefined,
          q: debouncedSearch || undefined,
          limit: LIMIT,
          offset,
        },
        signal,
      ),
    [status, channelId, severity, debouncedSearch, offset],
  );

  const { data: channelPage } = useAsync((signal) => listChannels({ limit: 200 }, signal), []);
  const channels = channelPage?.items ?? [];

  const [deleteTarget, setDeleteTarget] = useState<AlertGroup | null>(null);
  const [deleting, setDeleting] = useState(false);

  const items = data?.items ?? [];
  const hasFilters = Boolean(status || channelId || severity || debouncedSearch);

  const submitDelete = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await deleteAlertGroup(deleteTarget.id);
      toast.success('告警分组记录已删除（不影响已发出的消息）');
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
        title="告警分组"
        subtitle="AlertGroup 以 (hook_id, group_key) 唯一，按 last_seen_at 倒序"
        actions={
          <Button onClick={reload} loading={refreshing}>
            刷新
          </Button>
        }
      />

      <Card tight title="分组列表" subtitle={`共 ${data?.total ?? 0} 个分组`}>
        <div className="toolbar">
          <Field label="状态">
            <Select value={status} onChange={(event) => setStatus(event.target.value as '' | 'firing' | 'resolved')}>
              <option value="">全部</option>
              <option value="firing">{GROUP_STATUS_LABEL.firing}（firing）</option>
              <option value="resolved">{GROUP_STATUS_LABEL.resolved}（resolved）</option>
            </Select>
          </Field>
          <Field label="渠道">
            <Select value={channelId} onChange={(event) => setChannelId(event.target.value)}>
              <option value="">全部渠道</option>
              {channels.map((channel) => (
                <option key={channel.id} value={channel.id}>
                  {channel.name}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="severity">
            <Input
              placeholder="如 critical"
              value={severityInput}
              onChange={(event) => setSeverityInput(event.target.value)}
              style={{ width: 140 }}
            />
          </Field>
          <Field label="搜索">
            <Input
              placeholder="group_key / alertname / receiver"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              style={{ width: 240 }}
            />
          </Field>
          <Button
            size="sm"
            disabled={!hasFilters}
            onClick={() => {
              setStatus('');
              setChannelId('');
              setSeverityInput('');
              setSearch('');
            }}
          >
            清除筛选
          </Button>
        </div>

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
                    <th>状态</th>
                    <th>告警名</th>
                    <th>级别</th>
                    <th>渠道</th>
                    <th>Hook</th>
                    <th className="num">未解决</th>
                    <th className="num">已恢复</th>
                    <th className="num">事件</th>
                    <th>最近出现</th>
                    <th className="actions">操作</th>
                  </tr>
                </thead>
                <tbody>
                  {items.length === 0 ? (
                    <EmptyRow
                      colSpan={10}
                      title={hasFilters ? '没有符合条件的分组' : '暂无告警分组'}
                      description={
                        hasFilters
                          ? '尝试放宽筛选条件。'
                          : '把 Hook URL 配置到 AlertManager 的 receiver 后，这里会出现聚合分组。'
                      }
                    />
                  ) : (
                    items.map((group) => (
                      <tr key={group.id}>
                        <td>
                          <StatusBadge kind="group" value={group.status} />
                        </td>
                        <td>
                          <Link to={`/alert-groups/${group.id}`}>{group.alertname ?? '（无 alertname）'}</Link>
                          <div className="faint small mono" title={group.group_key}>
                            {group.group_key.length > 48 ? `${group.group_key.slice(0, 48)}…` : group.group_key}
                          </div>
                        </td>
                        <td>{group.severity ? <Badge tone="muted">{group.severity}</Badge> : <span className="faint">—</span>}</td>
                        <td>
                          {group.channel ? <Link to={`/channels/${group.channel.id}`}>{group.channel.name}</Link> : <span className="faint">—</span>}
                        </td>
                        <td className="small">{group.hook?.name ?? '—'}</td>
                        <td className="num">{group.firing_count}</td>
                        <td className="num">{group.resolved_count}</td>
                        <td className="num">
                          {group.event_count}
                          {group.truncated_alerts > 0 ? <span className="faint small"> +{group.truncated_alerts}</span> : null}
                        </td>
                        <td className="nowrap" title={formatDateTime(group.last_seen_at)}>
                          {formatRelative(group.last_seen_at)}
                        </td>
                        <td className="actions">
                          <div className="row tight" style={{ justifyContent: 'flex-end' }}>
                            <Button size="sm" onClick={() => navigate(`/alert-groups/${group.id}`)}>
                              详情
                            </Button>
                            <Button size="sm" variant="danger" onClick={() => setDeleteTarget(group)}>
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
            {data ? <Pagination total={data.total} limit={data.limit || LIMIT} offset={data.offset} onChange={setOffset} /> : null}
          </>
        )}
      </Card>

      <ConfirmModal
        open={deleteTarget !== null}
        title="删除告警分组记录"
        tone="danger"
        confirmLabel="删除"
        busy={deleting}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={() => void submitDelete()}
        message={
          <>
            将删除分组「{deleteTarget?.alertname ?? deleteTarget?.group_key}」及其投递记录。
            <strong>不会影响已经发出的飞书消息</strong>（docs/api.md §6）。
          </>
        }
      />
    </>
  );
}
