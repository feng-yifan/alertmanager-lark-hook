import { Link } from 'react-router-dom';
import { getOverview } from '../api';
import { Button, Card, EmptyRow, ErrorState, Loading, PageHeader, StatusBadge } from '../components/ui';
import { useAsync } from '../hooks';
import { formatDateTime, formatRelative } from '../utils';

function StatCard({
  label,
  value,
  sub,
  tone,
}: {
  label: string;
  value: number | string;
  sub?: string;
  tone?: 'danger' | 'ok';
}): JSX.Element {
  return (
    <div className="stat-card">
      <div className="stat-label">{label}</div>
      <div className="stat-value" style={tone === 'danger' ? { color: 'var(--danger)' } : tone === 'ok' ? { color: 'var(--ok)' } : undefined}>
        {value}
      </div>
      {sub ? <div className="stat-sub">{sub}</div> : null}
    </div>
  );
}

export default function OverviewPage(): JSX.Element {
  const { data, loading, error, reload, refreshing } = useAsync((signal) => getOverview(signal), []);

  return (
    <>
      <PageHeader
        title="概览"
        subtitle="渠道、Hook、告警分组与近 24 小时投递情况"
        actions={
          <Button onClick={reload} loading={refreshing}>
            刷新
          </Button>
        }
      />

      {loading && !data ? <Loading /> : null}
      {error && !data ? <ErrorState error={error} onRetry={reload} /> : null}

      {data ? (
        <>
          <div className="stat-grid">
            <StatCard
              label="渠道"
              value={data.channels.total}
              sub={`已启用 ${data.channels.enabled} · 已停用 ${Math.max(0, data.channels.total - data.channels.enabled)}`}
            />
            <StatCard label="Hook" value={data.hooks.total} sub={`已启用 ${data.hooks.enabled}`} />
            <StatCard
              label="告警分组"
              value={data.alert_groups.firing + data.alert_groups.resolved}
              sub={`未解决 ${data.alert_groups.firing} · 已恢复 ${data.alert_groups.resolved}`}
            />
            <StatCard label="近 24h 发送" value={data.deliveries_24h.sent} tone="ok" sub="成功投递" />
            <StatCard
              label="近 24h 失败"
              value={data.deliveries_24h.failed}
              tone={data.deliveries_24h.failed > 0 ? 'danger' : undefined}
              sub="重试耗尽 / 需人工重试"
            />
            <StatCard label="近 24h 抑制" value={data.deliveries_24h.suppressed} sub="策略判定不发送" />
          </div>

          <Card
            title="最近事件"
            subtitle="最新 10 条审计事件"
            tight
            actions={
              <>
                <Link className="btn btn-sm" to="/deliveries">
                  投递记录
                </Link>
                <Link className="btn btn-sm" to="/alert-groups">
                  告警分组
                </Link>
              </>
            }
          >
            <div className="table-wrap">
              <table className="data">
                <thead>
                  <tr>
                    <th>时间</th>
                    <th>动作</th>
                    <th>目标</th>
                    <th>结果</th>
                  </tr>
                </thead>
                <tbody>
                  {data.recent_events.length === 0 ? (
                    <EmptyRow colSpan={4} title="暂无事件" description="AlertManager 尚未向任何 Hook 投递事件。" />
                  ) : (
                    data.recent_events.map((event, index) => (
                      <tr key={`${event.at}-${event.action}-${index}`}>
                        <td className="nowrap" title={formatDateTime(event.at)}>
                          {formatRelative(event.at)}
                        </td>
                        <td className="mono">{event.action}</td>
                        <td className="mono break-all">{event.target ?? '—'}</td>
                        <td>
                          <StatusBadge kind="outcome" value={event.outcome} />
                        </td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          </Card>
        </>
      ) : null}
    </>
  );
}
