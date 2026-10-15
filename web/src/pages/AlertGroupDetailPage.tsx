import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import {
  deleteAlertGroup,
  errorMessage,
  getAlertGroup,
  listDeliveries,
  retryDelivery,
  revokeDelivery,
} from '../api';
import {
  Badge,
  Button,
  Card,
  ConfirmModal,
  CopyButton,
  DescriptionList,
  EmptyRow,
  EmptyState,
  ErrorState,
  Loading,
  Modal,
  PageHeader,
  StatusBadge,
} from '../components/ui';
import { useAsync } from '../hooks';
import { useToast } from '../toast';
import type { Delivery } from '../types';
import {
  cx,
  describeRetryResult,
  formatDateTime,
  prettyJson,
  safeHttpUrl,
  sortedEntries,
  truncate,
} from '../utils';

type DeliveryAction = { type: 'retry' | 'revoke'; delivery: Delivery } | null;

function deliveryTone(state: string): string {
  if (state === 'active' || state === 'resolved') return 'tone-ok';
  if (state === 'failed' || state === 'recall_failed') return 'tone-danger';
  if (state === 'pending') return 'tone-warn';
  return 'tone-muted';
}

export default function AlertGroupDetailPage(): JSX.Element {
  const { id = '' } = useParams<{ id: string }>();
  const toast = useToast();
  const navigate = useNavigate();

  const { data: group, loading, error, reload, refreshing } = useAsync((signal) => getAlertGroup(id, signal), [id]);

  const embedded = group?.deliveries;
  const needsDeliveryFetch = Boolean(group) && !embedded;
  const { data: deliveryPage, loading: deliveriesLoading } = useAsync(
    (signal) =>
      needsDeliveryFetch
        ? listDeliveries({ alert_group_id: id, limit: 100 }, signal)
        : Promise.resolve(null),
    [id, needsDeliveryFetch],
  );

  const deliveries: Delivery[] = embedded ?? deliveryPage?.items ?? [];
  const deliveriesPending = needsDeliveryFetch && deliveriesLoading && !deliveryPage;

  const [action, setAction] = useState<DeliveryAction>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [detailJson, setDetailJson] = useState<{ title: string; value: unknown } | null>(null);

  const runAction = async () => {
    if (!action) return;
    const { type, delivery } = action;
    setBusyId(delivery.id);
    try {
      if (type === 'retry') {
        // 后端同步重发一次并返回真实结果；失败原因未修复时仍是 failed，必须如实提示。
        const result = describeRetryResult(await retryDelivery(delivery.id));
        toast.push(result.message, result.tone);
      } else {
        await revokeDelivery(delivery.id);
        toast.success('已提交撤回；若超出撤回时限或权限不足，会标记为「撤回失败」');
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

  const submitDelete = async () => {
    if (!group) return;
    setDeleting(true);
    try {
      await deleteAlertGroup(group.id);
      toast.success('分组记录已删除（不影响已发出的消息）');
      navigate('/alert-groups');
    } catch (cause) {
      toast.error(errorMessage(cause));
      setDeleteOpen(false);
    } finally {
      setDeleting(false);
    }
  };

  if (loading && !group) return <Loading label="正在加载告警分组…" />;
  if (error && !group) return <ErrorState error={error} onRetry={reload} />;
  if (!group) return <ErrorState error={new Error('分组不存在')} onRetry={reload} />;

  return (
    <>
      <PageHeader
        title={
          <span className="row tight">
            <Link to="/alert-groups" className="faint small">
              ← 告警分组
            </Link>
            <span>{group.alertname ?? '（无 alertname）'}</span>
            <StatusBadge kind="group" value={group.status} />
          </span>
        }
        subtitle={
          <span className="row tight">
            <span className="mono break-all">{group.group_key}</span>
            <CopyButton value={group.group_key} small label="复制 groupKey" />
          </span>
        }
        actions={
          <>
            <Button onClick={reload} loading={refreshing}>
              刷新
            </Button>
            <Button variant="danger" onClick={() => setDeleteOpen(true)}>
              删除记录
            </Button>
          </>
        }
      />

      <div className="stat-grid">
        <div className="stat-card">
          <div className="stat-label">未解决告警</div>
          <div className="stat-value" style={{ color: group.firing_count > 0 ? 'var(--danger)' : undefined }}>
            {group.firing_count}
          </div>
        </div>
        <div className="stat-card">
          <div className="stat-label">已恢复告警</div>
          <div className="stat-value">{group.resolved_count}</div>
        </div>
        <div className="stat-card">
          <div className="stat-label">累计事件数</div>
          <div className="stat-value">{group.event_count}</div>
        </div>
        <div className="stat-card">
          <div className="stat-label">被截断的告警数</div>
          <div className="stat-value">{group.truncated_alerts}</div>
          <div className="stat-sub">AlertManager 侧已截断时，这是总数的下界</div>
        </div>
      </div>

      <Card title="分组元数据">
        <DescriptionList
          items={[
            { label: '接收器', value: <span className="mono">{group.receiver}</span> },
            {
              label: '渠道',
              value: group.channel ? <Link to={`/channels/${group.channel.id}`}>{group.channel.name}</Link> : '—',
            },
            { label: 'Hook', value: group.hook?.name ?? '—' },
            {
              label: '严重级别',
              value: group.severity ? <Badge tone="muted">{group.severity}</Badge> : '—',
            },
            { label: '首次出现', value: formatDateTime(group.first_seen_at) },
            { label: '最近出现', value: formatDateTime(group.last_seen_at) },
            { label: '最近通知', value: group.last_notified_at ? formatDateTime(group.last_notified_at) : '尚未通知' },
            { label: '恢复时间', value: group.resolved_at ? formatDateTime(group.resolved_at) : '未恢复' },
            {
              label: '当前活跃投递',
              value: group.active_delivery_id ? <span className="mono break-all">{group.active_delivery_id}</span> : '—',
            },
            {
              label: '当前 Pin 消息',
              value: group.pinned_message_id ? (
                <span className="mono break-all">{group.pinned_message_id}</span>
              ) : (
                '—'
              ),
            },
          ]}
        />
      </Card>

      <Card title="告警" subtitle={`共 ${group.alerts.length} 条（最近一次投递的快照）`} tight>
        <div className="table-wrap">
          <table className="data">
            <thead>
              <tr>
                <th>指纹</th>
                <th>状态</th>
                <th>标签</th>
                <th>注解</th>
                <th>开始时间</th>
                <th>结束时间</th>
                <th>来源地址</th>
              </tr>
            </thead>
            <tbody>
              {group.alerts.length === 0 ? (
                <EmptyRow colSpan={7} title="没有告警快照" description="该分组尚未接收过包含告警的事件。" />
              ) : (
                group.alerts.map((alert) => (
                  <tr key={alert.fingerprint}>
                    <td className="mono" title={alert.fingerprint}>
                      {truncate(alert.fingerprint, 16)}
                    </td>
                    <td>
                      <StatusBadge kind="alert" value={alert.status} />
                    </td>
                    <td>
                      <div className="row tight">
                        {sortedEntries(alert.labels).map(([key, value]) => (
                          <span className="chip" key={key} title={`${key}=${value}`}>
                            {key}={truncate(value, 24)}
                          </span>
                        ))}
                        {Object.keys(alert.labels).length === 0 ? <span className="faint">—</span> : null}
                      </div>
                    </td>
                    <td className="small">
                      {alert.annotations.summary ? (
                        <div>{alert.annotations.summary}</div>
                      ) : null}
                      {alert.annotations.description ? (
                        <div className="faint">{truncate(alert.annotations.description, 120)}</div>
                      ) : null}
                      {!alert.annotations.summary && !alert.annotations.description ? (
                        <button
                          type="button"
                          className="btn-link"
                          onClick={() => setDetailJson({ title: 'annotations', value: alert.annotations })}
                        >
                          查看全部注解
                        </button>
                      ) : null}
                    </td>
                    <td className="nowrap">{formatDateTime(alert.starts_at)}</td>
                    <td className="nowrap">{alert.ends_at ? formatDateTime(alert.ends_at) : '未恢复'}</td>
                    <td>
                      {(() => {
                        // 只放行 http(s)：告警的 generatorURL 由发送方控制，
                        // 直接当 href 会让 `javascript:` 在后台同源执行。
                        const generatorUrl = safeHttpUrl(alert.generator_url);
                        return generatorUrl ? (
                          <a href={generatorUrl} target="_blank" rel="noreferrer noopener">
                            Prometheus ↗
                          </a>
                        ) : (
                          <span className="faint">—</span>
                        );
                      })()}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </Card>

      <div className="grid-2">
        <Card title="分组标签">
          <LabelTable labels={group.group_labels} />
        </Card>
        <Card title="公共标签">
          <LabelTable labels={group.common_labels} />
        </Card>
      </div>

      <Card title="公共注解">
        <LabelTable labels={group.common_annotations} />
      </Card>

      <Card
        title="投递时间线"
        subtitle="按创建时间倒序；失败的投递可重试，撤回失败不阻塞主流程"
        actions={
          <Button size="sm" onClick={reload}>
            刷新
          </Button>
        }
      >
        {deliveriesPending ? <Loading label="正在加载投递记录…" /> : null}
        {!deliveriesPending && deliveries.length === 0 ? (
          <EmptyState title="暂无投递记录" description="该分组还没有产生投递记录（可能被策略抑制或尚未发送）。" />
        ) : null}
        {deliveries.length > 0 ? (
          <ul className="timeline">
            {[...deliveries]
              .sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime())
              .map((delivery) => (
                <li key={delivery.id} className={cx(deliveryTone(delivery.state))}>
                  <div className="row tight">
                    <span className="tl-time">{formatDateTime(delivery.created_at)}</span>
                    <StatusBadge kind="delivery" value={delivery.state} />
                    <StatusBadge kind="delivery_kind" value={delivery.kind} />
                    <span className="faint small">尝试 {delivery.attempt} 次</span>
                    <div className="spacer" />
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
                  <div className="small muted">
                    投递 <span className="mono">{delivery.id}</span>
                    {delivery.lark_message_id ? (
                      <>
                        {' · '}飞书消息 ID <span className="mono">{delivery.lark_message_id}</span>
                      </>
                    ) : null}
                    {delivery.card_entity_id ? (
                      <>
                        {' · '}卡片实体 ID <span className="mono">{delivery.card_entity_id}</span>
                      </>
                    ) : null}
                    {' · '}更新于 {formatDateTime(delivery.updated_at)}
                  </div>
                  {delivery.last_error ? (
                    <div className="banner banner-error" style={{ marginTop: 6, marginBottom: 0 }}>
                      {delivery.last_error}
                    </div>
                  ) : null}
                </li>
              ))}
          </ul>
        ) : null}
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
            <>将投递 <span className="mono">{action?.delivery.id}</span> 立即重发一次，并显示本次结果。</>
          ) : (
            <>
              将撤回投递 <span className="mono">{action?.delivery.id}</span> 对应的飞书消息。
              若超出撤回时限、消息已被删除或权限不足，撤回会失败：该投递会标记为「撤回失败」并写入审计，主流程仍视为成功。
            </>
          )
        }
      />

      <ConfirmModal
        open={deleteOpen}
        title="删除告警分组记录"
        tone="danger"
        confirmLabel="删除"
        busy={deleting}
        onCancel={() => setDeleteOpen(false)}
        onConfirm={() => void submitDelete()}
        message={<>将删除分组「{group.alertname ?? group.group_key}」及其投递记录，不会影响已经发出的飞书消息。</>}
      />

      <Modal open={detailJson !== null} title={detailJson?.title ?? ''} onClose={() => setDetailJson(null)}>
        <pre className="code-block">{detailJson ? prettyJson(detailJson.value) : ''}</pre>
      </Modal>
    </>
  );
}

function LabelTable({ labels }: { labels: Record<string, string> }): JSX.Element {
  const entries = sortedEntries(labels);
  if (entries.length === 0) return <p className="faint small mb-0">空</p>;
  return (
    <div className="row tight">
      {entries.map(([key, value]) => (
        <span className="chip" key={key}>
          {key}={value}
        </span>
      ))}
    </div>
  );
}
