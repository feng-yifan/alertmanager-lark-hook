import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { ApiError, deleteChannel, errorMessage, getChannel, patchChannel } from '../api';
import {
  Button,
  ConfirmModal,
  CopyButton,
  ErrorState,
  Field,
  Input,
  Loading,
  PageHeader,
  StatusBadge,
  Switch,
  Tabs,
} from '../components/ui';
import { useAsync } from '../hooks';
import { CHANNEL_KIND_LABEL, CHANNEL_SOURCE_LABEL, RECEIVE_ID_TYPE_LABEL } from '../labels';
import { useToast } from '../toast';
import { formatDateTime } from '../utils';
import HooksTab from './channels/HooksTab';
import PolicyTab from './channels/PolicyTab';
import TestMessageTab from './channels/TestMessageTab';

type TabKey = 'policy' | 'hooks' | 'test';

const TABS: Array<{ key: TabKey; label: string }> = [
  { key: 'policy', label: '发送规则' },
  { key: 'hooks', label: '接收 Hook' },
  { key: 'test', label: '发送测试' },
];

export default function ChannelDetailPage(): JSX.Element {
  const { id = '' } = useParams<{ id: string }>();
  const toast = useToast();
  const navigate = useNavigate();

  const { data: channel, loading, error, reload, refreshing } = useAsync((signal) => getChannel(id, signal), [id]);

  const [tab, setTab] = useState<TabKey>('policy');
  const [renameOpen, setRenameOpen] = useState(false);
  const [nameValue, setNameValue] = useState('');
  const [renaming, setRenaming] = useState(false);
  const [toggling, setToggling] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [force, setForce] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const submitRename = async () => {
    if (!channel) return;
    if (!nameValue.trim() || nameValue.trim().length > 64) {
      toast.error('渠道名称必填且不超过 64 个字符');
      return;
    }
    setRenaming(true);
    try {
      await patchChannel(channel.id, { name: nameValue.trim() });
      toast.success('渠道名称已更新');
      setRenameOpen(false);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRenaming(false);
    }
  };

  const toggleEnabled = async (next: boolean) => {
    if (!channel) return;
    setToggling(true);
    try {
      await patchChannel(channel.id, { enabled: next });
      toast.success(next ? '渠道已启用' : '渠道已停用（R13：不再发送，仅审计）');
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setToggling(false);
    }
  };

  const submitDelete = async () => {
    if (!channel) return;
    setDeleting(true);
    try {
      await deleteChannel(channel.id, force);
      toast.success(force ? '渠道已强制删除' : '渠道已删除');
      navigate('/channels');
    } catch (cause) {
      if (cause instanceof ApiError && cause.code === 'channel_has_deliveries' && !force) {
        setForce(true);
      } else {
        toast.error(errorMessage(cause));
        setDeleteOpen(false);
        setForce(false);
      }
    } finally {
      setDeleting(false);
    }
  };

  if (loading && !channel) return <Loading label="正在加载渠道…" />;
  if (error && !channel) return <ErrorState error={error} onRetry={reload} />;
  if (!channel) return <ErrorState error={new Error('渠道不存在')} onRetry={reload} />;

  return (
    <>
      <PageHeader
        title={
          <span className="row tight">
            <Link to="/channels" className="faint small">
              ← 渠道
            </Link>
            <span>{channel.name}</span>
          </span>
        }
        subtitle={
          <span className="row tight">
            <StatusBadge kind="channel_kind" value={channel.kind} />
            <StatusBadge kind="channel_source" value={channel.source} />
            <StatusBadge kind="enabled" value={channel.enabled} />
            <span className={channel.bot_can_send ? 'badge badge-ok' : 'badge badge-muted'}>
              {channel.bot_can_send ? '机器人可发送' : '机器人不可发送'}
            </span>
            <span className="faint" title={formatDateTime(channel.updated_at)}>
              更新于 {formatDateTime(channel.updated_at)}
            </span>
          </span>
        }
        actions={
          <>
            <Button onClick={reload} loading={refreshing}>
              刷新
            </Button>
            <Button
              onClick={() => {
                setNameValue(channel.name);
                setRenameOpen(true);
              }}
            >
              重命名
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                setForce(false);
                setDeleteOpen(true);
              }}
            >
              删除渠道
            </Button>
          </>
        }
      />

      <div className="card">
        <div className="card-body">
          <div className="kv-grid">
            <dt>receive_id</dt>
            <dd>
              <span className="row tight">
                <span className="mono break-all">{channel.receive_id}</span>
                <CopyButton value={channel.receive_id} small />
              </span>
            </dd>
            <dt>receive_id_type</dt>
            <dd>
              {RECEIVE_ID_TYPE_LABEL[channel.receive_id_type] ?? channel.receive_id_type}
              <span className="faint small">（{channel.kind === 'group' ? CHANNEL_KIND_LABEL.group : CHANNEL_KIND_LABEL.p2p_user}）</span>
            </dd>
            <dt>external</dt>
            <dd>{channel.external ? '外部群' : '内部'}</dd>
            <dt>source</dt>
            <dd>{CHANNEL_SOURCE_LABEL[channel.source] ?? channel.source}</dd>
            <dt>启用状态</dt>
            <dd>
              <Switch checked={channel.enabled} disabled={toggling} onChange={(next) => void toggleEnabled(next)} />
            </dd>
          </div>
        </div>
      </div>

      <Tabs<TabKey> tabs={TABS} active={tab} onChange={setTab} />

      {tab === 'policy' ? <PolicyTab channelId={channel.id} channelName={channel.name} onSaved={reload} /> : null}
      {tab === 'hooks' ? <HooksTab channel={channel} onChanged={reload} /> : null}
      {tab === 'test' ? <TestMessageTab channel={channel} /> : null}

      <ConfirmModal
        open={renameOpen}
        title="重命名渠道"
        confirmLabel="保存"
        busy={renaming}
        onCancel={() => setRenameOpen(false)}
        onConfirm={() => void submitRename()}
        message={
          <Field label="渠道名称" required hint="≤ 64 字符">
            <Input value={nameValue} autoFocus onChange={(event) => setNameValue(event.target.value)} />
          </Field>
        }
      />

      <ConfirmModal
        open={deleteOpen}
        title={force ? '强制删除渠道' : '删除渠道'}
        tone="danger"
        confirmLabel={force ? '仍要删除（force=true）' : '删除'}
        busy={deleting}
        onCancel={() => {
          setDeleteOpen(false);
          setForce(false);
        }}
        onConfirm={() => void submitDelete()}
        message={
          force ? (
            <>
              渠道「{channel.name}」已有投递记录：普通删除只会软删为 <code>enabled=false</code> 并返回 409
              channel_has_deliveries。使用 <code>?force=true</code> 会级联删除 hooks / policies。
            </>
          ) : (
            <>将删除渠道「{channel.name}」及其 hooks 与通知策略；如已有投递记录，服务端将返回 409 并改为软删。</>
          )
        }
      />
    </>
  );
}
