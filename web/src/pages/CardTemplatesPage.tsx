import { useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import { deleteCardTemplate, errorMessage, listCardTemplates, patchCardTemplate } from '../api';
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
import type { CardTemplate, CardTemplateInput, CardTemplateKind } from '../types';
import { formatDateTime, shortId } from '../utils';

function sourceLabel(template: CardTemplate): string {
  if (template.kind !== 'card_message') return '—';
  const source = template.card_message?.source;
  return source ? CARD_SOURCE_LABEL[source] ?? source : '—';
}

/**
 * PATCH 走的是「整份替换」语义（后端用请求体重新 build 模板），所以「设为默认」必须
 * 回传模板现有内容，否则后端会用内置默认卡片覆盖用户内容。
 */
function templateToInput(template: CardTemplate, isDefault: boolean): CardTemplateInput {
  if (template.kind === 'card_message') {
    const spec = template.card_message;
    return {
      name: template.name,
      kind: 'card_message',
      is_default: isDefault,
      card_message: spec
        ? {
            source: spec.source,
            card_json: spec.card_json,
            template_id: spec.template_id,
            template_version_name: spec.template_version_name,
            variables: spec.variables,
          }
        : null,
    };
  }
  return {
    name: template.name,
    kind: 'app_feed_card',
    is_default: isDefault,
    app_feed_spec: template.app_feed_spec,
  };
}

function templateReferencedHint(): string {
  return '模板被渠道策略引用时删除会被服务端以 409 conflict 拒绝，请先解除引用。';
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

  const items = data ?? [];

  const setDefault = async (template: CardTemplate) => {
    setBusyId(template.id);
    try {
      await patchCardTemplate(template.id, templateToInput(template, true));
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
      toast.error(errorMessage(cause));
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
        subtitle={`共 ${data?.length ?? 0} 个模板`}
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
                          <span title={(template.detected_variables ?? []).join(', ')}>
                            {(template.detected_variables ?? []).length} 个
                          </span>
                          {template.kind === 'card_message' && (template.card_message?.variables.length ?? 0) > 0 ? (
                            <span className="faint small"> · 声明 {template.card_message?.variables.length ?? 0}</span>
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
