import { useState } from 'react';
import { createHook, deleteHook, errorMessage, patchHook, rotateHook } from '../../api';
import {
  Banner,
  Button,
  Card,
  ConfirmModal,
  CopyButton,
  EmptyRow,
  Field,
  Input,
  Modal,
  StatusBadge,
  Switch,
} from '../../components/ui';
import { useToast } from '../../toast';
import type { Channel, Hook } from '../../types';
import { formatDateTime, formatRelative } from '../../utils';

interface Props {
  channel: Channel;
  onChanged: () => void;
}

interface OneTimeCredential {
  title: string;
  token: string;
  hookUrl?: string;
}

const MAX_NAME = 64;

export default function HooksTab({ channel, onChanged }: Props): JSX.Element {
  const toast = useToast();

  const [createOpen, setCreateOpen] = useState(false);
  const [newName, setNewName] = useState('');
  const [creating, setCreating] = useState(false);
  const [oneTime, setOneTime] = useState<OneTimeCredential | null>(null);

  const [busyId, setBusyId] = useState<string | null>(null);
  const [renameTarget, setRenameTarget] = useState<Hook | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [renaming, setRenaming] = useState(false);
  const [rotateTarget, setRotateTarget] = useState<Hook | null>(null);
  const [rotating, setRotating] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<Hook | null>(null);
  const [deleting, setDeleting] = useState(false);

  const hooks = channel.hooks ?? [];

  const toggleEnabled = async (hook: Hook, next: boolean) => {
    setBusyId(hook.id);
    try {
      await patchHook(hook.id, { enabled: next });
      toast.success(next ? `Hook「${hook.name}」已启用` : `Hook「${hook.name}」已停用`);
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusyId(null);
    }
  };

  const submitCreate = async () => {
    if (!newName.trim()) {
      toast.error('Hook 名称不能为空');
      return;
    }
    if (newName.trim().length > MAX_NAME) {
      toast.error(`Hook 名称长度不能超过 ${MAX_NAME} 个字符`);
      return;
    }
    setCreating(true);
    try {
      const created = await createHook({ channel_id: channel.id, name: newName.trim() });
      toast.success(`Hook「${created.name}」已创建`);
      setCreateOpen(false);
      setNewName('');
      setOneTime({ title: `新 Hook「${created.name}」`, token: created.hook_token, hookUrl: created.hook_url });
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setCreating(false);
    }
  };

  const submitRename = async () => {
    if (!renameTarget) return;
    if (!renameValue.trim() || renameValue.trim().length > MAX_NAME) {
      toast.error(`名称必填且不超过 ${MAX_NAME} 个字符`);
      return;
    }
    setRenaming(true);
    try {
      await patchHook(renameTarget.id, { name: renameValue.trim() });
      toast.success('已重命名');
      setRenameTarget(null);
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRenaming(false);
    }
  };

  const submitRotate = async () => {
    if (!rotateTarget) return;
    setRotating(true);
    try {
      const rotated = await rotateHook(rotateTarget.id);
      toast.success(`Hook「${rotateTarget.name}」的 token 已轮换`);
      setOneTime({ title: `轮换后的 Hook「${rotateTarget.name}」`, token: rotated.token, hookUrl: rotated.hook_url });
      setRotateTarget(null);
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRotating(false);
    }
  };

  const submitDelete = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await deleteHook(deleteTarget.id);
      toast.success(`Hook「${deleteTarget.name}」已删除`);
      setDeleteTarget(null);
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
      setDeleteTarget(null);
    } finally {
      setDeleting(false);
    }
  };

  return (
    <>
      <Card
        title="接收 Hook"
        subtitle={`每个 Hook 对应 AlertManager 的一个 receiver；渠道「${channel.name}」共 ${hooks.length} 个`}
        tight
        actions={
          <Button
            variant="primary"
            onClick={() => {
              setNewName('');
              setCreateOpen(true);
            }}
          >
            新建 Hook
          </Button>
        }
      >
        <div style={{ padding: '0 16px' }}>
          <div className="callout info mt-8 mb-8">
            列表接口只返回 <code>hook_path</code> 的占位形式（<code>&lt;token&gt;</code>）与 <code>token_prefix</code>；
            明文 token 与完整 hook_url <strong>仅在创建 / 轮换的响应中出现一次</strong>。轮换后旧 token 立即失效
            （business-rules §6）。
          </div>
        </div>
        <div className="table-wrap">
          <table className="data">
            <thead>
              <tr>
                <th>名称</th>
                <th>token_prefix</th>
                <th>hook_path</th>
                <th>启用</th>
                <th className="num">调用次数</th>
                <th>最近调用</th>
                <th>创建时间</th>
                <th className="actions">操作</th>
              </tr>
            </thead>
            <tbody>
              {hooks.length === 0 ? (
                <EmptyRow colSpan={8} title="暂无 Hook" description="新建一个 Hook 并把它配置到 AlertManager 的 webhook_configs。" />
              ) : (
                hooks.map((hook) => (
                  <tr key={hook.id}>
                    <td>{hook.name}</td>
                    <td className="mono">{hook.token_prefix}</td>
                    <td>
                      <div className="row tight">
                        <span className="mono break-all">{hook.hook_path}</span>
                        <CopyButton value={`${window.location.origin}${hook.hook_path}`} small label="复制 URL" />
                      </div>
                    </td>
                    <td>
                      <Switch
                        checked={hook.enabled}
                        disabled={busyId === hook.id}
                        onChange={(next) => void toggleEnabled(hook, next)}
                      />
                    </td>
                    <td className="num">{hook.call_count}</td>
                    <td className="nowrap" title={formatDateTime(hook.last_called_at)}>
                      {hook.last_called_at ? formatRelative(hook.last_called_at) : '从未调用'}
                    </td>
                    <td className="nowrap">{formatDateTime(hook.created_at)}</td>
                    <td className="actions">
                      <div className="row tight" style={{ justifyContent: 'flex-end' }}>
                        <Button
                          size="sm"
                          onClick={() => {
                            setRenameTarget(hook);
                            setRenameValue(hook.name);
                          }}
                        >
                          重命名
                        </Button>
                        <Button size="sm" onClick={() => setRotateTarget(hook)}>
                          轮换 token
                        </Button>
                        <Button size="sm" variant="danger" onClick={() => setDeleteTarget(hook)}>
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
      </Card>

      <Modal
        open={createOpen}
        title="新建 Hook"
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
        <Field label="Hook 名称" required hint={`≤ ${MAX_NAME} 字符，建议与 AlertManager receiver 名称一致`}>
          <Input value={newName} autoFocus onChange={(event) => setNewName(event.target.value)} />
        </Field>
        <p className="faint small mb-0">
          创建后立即返回明文 token 与 hook_url，只显示一次；请复制到 AlertManager 的{' '}
          <code>webhook_configs[].url</code>。
        </p>
      </Modal>

      <Modal
        open={oneTime !== null}
        title="⚠ Hook 凭证（只显示一次）"
        onClose={() => setOneTime(null)}
        footer={
          <Button variant="primary" onClick={() => setOneTime(null)}>
            我已保存
          </Button>
        }
      >
        {oneTime ? (
          <>
            <Banner tone="warn">
              <strong>只显示一次</strong>：关闭本对话框后无法再次查看明文 token / hook_url（服务端只存 SHA-256
              哈希与 8 字符前缀）。请立即复制保存，并更新 AlertManager 配置。
            </Banner>
            <div className="small mb-8">{oneTime.title}</div>
            <Field label="明文 token">
              <div className="copy-row">
                <span className="copy-value">{oneTime.token}</span>
                <CopyButton value={oneTime.token} small />
              </div>
            </Field>
            {oneTime.hookUrl ? (
              <Field label="hook_url">
                <div className="copy-row">
                  <span className="copy-value">{oneTime.hookUrl}</span>
                  <CopyButton value={oneTime.hookUrl} small />
                </div>
              </Field>
            ) : null}
            <div className="callout mt-8">
              AlertManager 配置示例：
              <pre className="code-block mt-8">{`receivers:
  - name: 'lark-ops'
    webhook_configs:
      - url: '${oneTime.hookUrl ?? `${window.location.origin}/api/v1/hooks/alertmanager/<token>`}'`}</pre>
            </div>
          </>
        ) : null}
      </Modal>

      <ConfirmModal
        open={renameTarget !== null}
        title="重命名 Hook"
        confirmLabel="保存"
        busy={renaming}
        onCancel={() => setRenameTarget(null)}
        onConfirm={() => void submitRename()}
        message={
          <Field label="Hook 名称" required>
            <Input value={renameValue} onChange={(event) => setRenameValue(event.target.value)} />
          </Field>
        }
      />

      <ConfirmModal
        open={rotateTarget !== null}
        title="轮换 Hook token"
        tone="danger"
        confirmLabel="轮换并失效旧 token"
        busy={rotating}
        onCancel={() => setRotateTarget(null)}
        onConfirm={() => void submitRotate()}
        message={
          <>
            将立即为 Hook「{rotateTarget?.name}」生成新 token，<strong>旧 token 立即失效</strong>
            ，使用旧 URL 的 AlertManager 将收到 404 unknown_hook。轮换后新凭证只显示一次。
          </>
        }
      />

      <ConfirmModal
        open={deleteTarget !== null}
        title="删除 Hook"
        tone="danger"
        confirmLabel="删除"
        busy={deleting}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={() => void submitDelete()}
        message={<>删除 Hook「{deleteTarget?.name}」后其 token 立即失效，使用该 URL 的 AlertManager 将收到 404。</>}
      />

      <div className="row">
        <StatusBadge kind="enabled" value={channel.enabled} />
        <span className="faint small">
          渠道禁用时（channel.enabled=false）该渠道所有 Hook 的投递都会被拒绝，仅记录审计（R13）。
        </span>
      </div>
    </>
  );
}
