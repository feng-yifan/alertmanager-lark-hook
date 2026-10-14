import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { channelOptionLabel } from '../channels';
import { errorMessage, listChannels, listDeliveries, retryDelivery, revokeDelivery } from '../api';
import {
  Button,
  Card,
  ConfirmModal,
  CopyButton,
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
import { DELIVERY_STATE_LABEL, DELIVERY_STATE_VALUES } from '../labels';
import { useToast } from '../toast';
import type { Delivery } from '../types';
import { formatDateTime, shortId, truncate } from '../utils';

const LIMIT = 50;

type PendingAction = { type: 'retry' | 'revoke'; delivery: Delivery } | null;

export default function DeliveriesPage(): JSX.Element {
  const toast = useToast();
  const [searchParams, setSearchParams] = useSearchParams();

  const [groupInput, setGroupInput] = useState(searchParams.get('alert_group_id') ?? '');
  const [channelId, setChannelId] = useState(searchParams.get('channel_id') ?? '');
  const [state, setState] = useState(searchParams.get('state') ?? '');
  const [offset, setOffset] = useState(0);
  const groupId = useDebounced(groupInput, 300);

  useEffect(() => {
    setOffset(0);
  }, [groupId, channelId, state]);

  const { data, loading, error, reload, refreshing } = useAsync(
    (signal) =>
      listDeliveries(
        {
          alert_group_id: groupId || undefined,
          channel_id: channelId || undefined,
          state: state || undefined,
          limit: LIMIT,
          offset,
        },
        signal,
      ),
    [groupId, channelId, state, offset],
  );

  const { data: channelPage } = useAsync((signal) => listChannels({ limit: 200 }, signal), []);
  const channels = channelPage?.items ?? [];

  const [action, setAction] = useState<PendingAction>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const items = data?.items ?? [];
  const hasFilters = Boolean(groupId || channelId || state);

  const syncUrl = (next: { alert_group_id?: string; channel_id?: string; state?: string }) => {
    const params = new URLSearchParams();
    if (next.alert_group_id) params.set('alert_group_id', next.alert_group_id);
    if (next.channel_id) params.set('channel_id', next.channel_id);
    if (next.state) params.set('state', next.state);
    setSearchParams(params, { replace: true });
  };

  const runAction = async () => {
    if (!action) return;
    const { type, delivery } = action;
    setBusyId(delivery.id);
    try {
      if (type === 'retry') {
        await retryDelivery(delivery.id);
        toast.success('已提交重试，等待后台队列重新发送');
      } else {
        await revokeDelivery(delivery.id);
        toast.success('已提交撤回；失败时会标记为「撤回失败」');
      }
      setAction(null);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
      setAction(null);
    } finally {
      setBusyId(null);
    }
  };

  return (
    <>
      <PageHeader
        title="投递记录"
        subtitle="每次发送、更新、撤回都会留一条记录，状态从「待发送」开始推进"
        actions={
          <Button onClick={reload} loading={refreshing}>
            刷新
          </Button>
        }
      />

      <Card tight title="投递列表" subtitle={`共 ${data?.total ?? 0} 条`}>
        <div className="toolbar">
          <Field label="告警分组 ID">
            <Input
              className="mono"
              placeholder="告警分组 ID"
              value={groupInput}
              onChange={(event) => {
                setGroupInput(event.target.value);
                syncUrl({ alert_group_id: event.target.value, channel_id: channelId, state });
              }}
              style={{ width: 260 }}
            />
          </Field>
          <Field label="渠道">
            <Select
              value={channelId}
              onChange={(event) => {
                setChannelId(event.target.value);
                syncUrl({ alert_group_id: groupInput, channel_id: event.target.value, state });
              }}
            >
              <option value="">全部渠道</option>
              {channels.map((channel) => (
                <option key={channel.id} value={channel.id}>
                  {channelOptionLabel(channel)}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="状态">
            <Select
              value={state}
              onChange={(event) => {
                setState(event.target.value);
                syncUrl({ alert_group_id: groupInput, channel_id: channelId, state: event.target.value });
              }}
            >
              <option value="">全部状态</option>
              {DELIVERY_STATE_VALUES.map((value) => (
                <option key={value} value={value}>
                  {DELIVERY_STATE_LABEL[value] ?? value}
                </option>
              ))}
            </Select>
          </Field>
          <Button
            size="sm"
            disabled={!hasFilters}
            onClick={() => {
              setGroupInput('');
              setChannelId('');
              setState('');
              setSearchParams({}, { replace: true });
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
                    <th>类型</th>
                    <th>告警分组</th>
                    <th className="num">尝试</th>
                    <th>飞书消息 ID</th>
                    <th>最后错误</th>
                    <th>时间</th>
                    <th className="actions">操作</th>
                  </tr>
                </thead>
                <tbody>
                  {items.length === 0 ? (
                    <EmptyRow
                      colSpan={9}
                      title={hasFilters ? '没有符合条件的投递记录' : '暂无投递记录'}
                      description={hasFilters ? '尝试放宽筛选条件。' : 'AlertManager 触发告警后，这里会出现每次发送/更新/撤回的记录。'}
                    />
                  ) : (
                    items.map((delivery) => (
                      <tr key={delivery.id}>
                        <td>
                          <StatusBadge kind="delivery" value={delivery.state} />
                        </td>
                        <td>
                          <StatusBadge kind="delivery_kind" value={delivery.kind} />
                        </td>
                        <td>
                          <Link to={`/alert-groups/${delivery.alert_group_id}`} className="mono" title={delivery.alert_group_id}>
                            {shortId(delivery.alert_group_id)}
                          </Link>
                        </td>
                        <td className="num">{delivery.attempt}</td>
                        <td>
                          {delivery.lark_message_id ? (
                            <span className="row tight">
                              <span className="mono">{shortId(delivery.lark_message_id, 14)}</span>
                              <CopyButton value={delivery.lark_message_id} small label="复制" />
                            </span>
                          ) : (
                            <span className="faint">—</span>
                          )}
                        </td>
                        <td className="small" title={delivery.last_error ?? undefined}>
                          {delivery.last_error ? truncate(delivery.last_error, 60) : <span className="faint">—</span>}
                        </td>
                        <td className="nowrap small">
                          <div>{formatDateTime(delivery.created_at)}</div>
                          <div className="faint">更新 {formatDateTime(delivery.updated_at)}</div>
                        </td>
                        <td className="actions">
                          <div className="row tight" style={{ justifyContent: 'flex-end' }}>
                            <Button
                              size="sm"
                              disabled={busyId === delivery.id || delivery.state !== 'failed'}
                              onClick={() => setAction({ type: 'retry', delivery })}
                            >
                              重试
                            </Button>
                            <Button
                              size="sm"
                              variant="danger"
                              disabled={busyId === delivery.id}
                              onClick={() => setAction({ type: 'revoke', delivery })}
                            >
                              撤回
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
        open={action !== null}
        title={action?.type === 'retry' ? '重试投递' : '撤回飞书消息'}
        tone={action?.type === 'retry' ? 'primary' : 'danger'}
        confirmLabel={action?.type === 'retry' ? '重试' : '撤回'}
        busy={busyId !== null}
        onCancel={() => setAction(null)}
        onConfirm={() => void runAction()}
        message={
          action?.type === 'retry' ? (
            <>
              仅失败的投递可以重试：将 <span className="mono">{action?.delivery.id}</span> 重新加入发送队列。
            </>
          ) : (
            <>
              将撤回投递 <span className="mono">{action?.delivery.id}</span> 对应的飞书消息；撤回失败不阻塞主流程。
            </>
          )
        }
      />
    </>
  );
}
