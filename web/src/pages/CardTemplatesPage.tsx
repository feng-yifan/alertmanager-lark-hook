import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
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
} from '../components/ui';
import { useAsync } from '../hooks';
import { CARD_SOURCE_LABEL } from '../labels';
import { useToast } from '../toast';
import type { CardTemplate, CardTemplateInput } from '../types';
import { formatDateTime, shortId } from '../utils';

function sourceLabel(template: CardTemplate): string {
  const source = template.card_message?.source;
  return source ? CARD_SOURCE_LABEL[source] ?? source : '—';
}

/**
 * PATCH 走的是「整份替换」语义（后端用请求体重新 build 模板），所以「设为默认」必须
 * 回传模板现有内容，否则后端会用内置默认卡片覆盖用户内容。
 */
function templateToInput(template: CardTemplate, isDefault: boolean): CardTemplateInput {
  const spec = template.card_message;
  return {
    name: template.name,
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

function templateReferencedHint(): string {
  return '模板被渠道策略引用时删除会被服务端以 409 conflict 拒绝，请先解除引用。';
}

export default function CardTemplatesPage(): JSX.Element {
  const toast = useToast();
  const navigate = useNavigate();

  const { data, loading, error, reload, refreshing } = useAsync(
    (signal) => listCardTemplates(signal),
    [],
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

  return (
    <>
      <PageHeader
        title="卡片模板"
        subtitle="飞书卡片 JSON 2.0：可视化拖拽或直接编辑 JSON"
        actions={
          <>
            <Button onClick={() => navigate('/card-templates/new')}>新建卡片模板</Button>
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
                      colSpan={6}
                      title="暂无模板"
                      description="新建一个卡片模板：从告警积木拖起，或粘贴搭建工具导出的 JSON。"
                    />
                  ) : (
                    items.map((template) => (
                      <tr key={template.id}>
                        <td>
                          <Link to={`/card-templates/${template.id}`}>{template.name}</Link>
                          <div className="faint small mono">{shortId(template.id)}</div>
                        </td>
                        <td className="small">{sourceLabel(template)}</td>
                        <td>
                          <span title={(template.detected_variables ?? []).join(', ')}>
                            {(template.detected_variables ?? []).length} 个
                          </span>
                          {(template.card_message?.variables.length ?? 0) > 0 ? (
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
              <div className="callout mt-8">这是当前默认模板，删除后渠道策略会回退到内置默认卡片。</div>
            ) : null}
          </>
        }
      />
    </>
  );
}
