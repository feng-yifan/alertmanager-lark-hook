import { useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import { ApiError, deleteCardTemplate, errorMessage, listCardTemplates, patchCardTemplate } from '../api';
import {
  Badge,
  Button,
  Card,
  ConfirmModal,
  EmptyRow,
  ErrorState,
  Loading,
  PageHeader,
  Select,
  StatusBadge,
} from '../components/ui';
import { useAsync } from '../hooks';
import { CARD_SOURCE_LABEL, MESSAGE_TYPE_LABEL } from '../labels';
import { useToast } from '../toast';
import type { CardTemplate, CardTemplateKind } from '../types';
import { formatDateTime, shortId } from '../utils';

function sourceLabel(template: CardTemplate): string {
  if (template.kind !== 'card_message') return '—';
  return CARD_SOURCE_LABEL[template.source] ?? template.source;
}

function templateReferencedHint(): string {
  return '模板被渠道策略引用时删除会返回 409 template_in_use（domain-model §4），请先解除引用。';
}

export default function CardTemplatesPage(): JSX.Element {
  const toast = useToast();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();

  const kindParam = searchParams.get('kind') ?? '';
  const kind = (kindParam === 'card_message' || kindParam === 'app_feed_card' ? kindParam : '') as
    | CardTemplateKind
    | '';
  const { data, loading, error, reload, refreshing } = useAsync(
    (signal) => listCardTemplates({ kind }, signal),
    [kind],
  );

  const [deleteTarget, setDeleteTarget] = useState<CardTemplate | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);

  const items = data?.items ?? [];

  const setDefault = async (template: CardTemplate) => {
    setBusyId(template.id);
    try {
      await patchCardTemplate(template.id, { is_default: true });
      toast.success(`「${template.name}」已设为默认模板`);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusyId(null);
    }
  };

  const submitDelete = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await deleteCardTemplate(deleteTarget.id);
      toast.success(`模板「${deleteTarget.name}」已删除`);
      setDeleteTarget(null);
      reload();
    } catch (cause) {
      if (cause instanceof ApiError && cause.code === 'template_in_use') {
        toast.error('该模板正被某个渠道的通知策略引用（409 template_in_use），请先在渠道的「发送规则」中解除引用。');
      } else {
        toast.error(errorMessage(cause));
      }
      setDeleteTarget(null);
    } finally {
      setDeleting(false);
    }
  };

  const setKindFilter = (next: string) => {
    if (next) setSearchParams({ kind: next });
    else setSearchParams({});
  };

  return (
    <>
      <PageHeader
        title="卡片模板"
        subtitle="卡片消息（CardKit JSON 2.0 / 搭建工具模板）与应用消息流卡片模板"
        actions={
          <>
            <Button onClick={() => navigate('/card-templates/new?kind=card_message')}>新建卡片消息模板</Button>
            <Button onClick={() => navigate('/card-templates/new?kind=app_feed_card')}>新建应用消息流模板</Button>
            <Button onClick={reload} loading={refreshing}>
              刷新
            </Button>
          </>
        }
      />

      <Card
        tight
        title="模板列表"
        subtitle={`共 ${data?.total ?? 0} 个模板`}
        actions={
          <Select value={kind} onChange={(event) => setKindFilter(event.target.value)} style={{ width: 190 }}>
            <option value="">全部类型</option>
            <option value="card_message">{MESSAGE_TYPE_LABEL.card_message}</option>
            <option value="app_feed_card">{MESSAGE_TYPE_LABEL.app_feed_card}</option>
          </Select>
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
                    <th>类型</th>
                    <th>来源</th>
                    <th>变量</th>
                    <th>默认</th>
                    <th>更新时间</th>
                    <th className="actions">操作</th>
                  </tr>
                </thead>
                <tbody>
                  {items.length === 0 ? (
                    <EmptyRow
                      colSpan={7}
                      title="暂无模板"
                      description="新建一个卡片消息模板（可导入卡片搭建工具导出的 JSON），或新建应用消息流模板。"
                    />
                  ) : (
                    items.map((template) => (
                      <tr key={template.id}>
                        <td>
                          <Link to={`/card-templates/${template.id}`}>{template.name}</Link>
                          <div className="faint small mono">{shortId(template.id)}</div>
                        </td>
                        <td>
                          <StatusBadge kind="message_type" value={template.kind} />
                        </td>
                        <td className="small">{sourceLabel(template)}</td>
                        <td>
                          <span title={template.detected_variables.join(', ')}>
                            {template.detected_variables.length} 个
                          </span>
                          {template.kind === 'card_message' && template.variables.length > 0 ? (
                            <span className="faint small"> · 声明 {template.variables.length}</span>
                          ) : null}
                        </td>
                        <td>{template.is_default ? <Badge tone="accent">默认</Badge> : <span className="faint">—</span>}</td>
                        <td className="nowrap">{formatDateTime(template.updated_at)}</td>
                        <td className="actions">
                          <div className="row tight" style={{ justifyContent: 'flex-end' }}>
                            <Button size="sm" onClick={() => navigate(`/card-templates/${template.id}`)}>
                              编辑
                            </Button>
                            <Button
                              size="sm"
                              disabled={template.is_default || busyId === template.id}
                              onClick={() => void setDefault(template)}
                            >
                              设为默认
                            </Button>
                            <Button size="sm" variant="danger" onClick={() => setDeleteTarget(template)}>
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
          </>
        )}
      </Card>

      <ConfirmModal
        open={deleteTarget !== null}
        title="删除卡片模板"
        tone="danger"
        confirmLabel="删除"
        busy={deleting}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={() => void submitDelete()}
        message={
          <>
            将删除模板「{deleteTarget?.name}」。{templateReferencedHint()}
            {deleteTarget?.is_default ? (
              <div className="callout mt-8">这是 {MESSAGE_TYPE_LABEL[deleteTarget.kind]} 的默认模板，删除后该类型可能没有可用默认模板。</div>
            ) : null}
          </>
        }
      />
    </>
  );
}
