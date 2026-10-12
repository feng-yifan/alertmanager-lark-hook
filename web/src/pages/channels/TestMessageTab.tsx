import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { errorMessage, listCardTemplates, sendTestMessage } from '../../api';
import { Banner, Button, Card, Field, JsonBlock, Options, RadioGroup, Select } from '../../components/ui';
import { useAsync } from '../../hooks';
import { MESSAGE_TYPE_LABEL } from '../../labels';
import { useToast } from '../../toast';
import type { Channel, MessageType, TestMessageResponse } from '../../types';

interface Props {
  channel: Channel;
}

export default function TestMessageTab({ channel }: Props): JSX.Element {
  const toast = useToast();
  const [kind, setKind] = useState<'firing' | 'resolved'>('firing');
  const [messageType, setMessageType] = useState<'' | MessageType>('');
  const [templateId, setTemplateId] = useState('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<TestMessageResponse | null>(null);
  const [error, setError] = useState<unknown>(null);

  const { data: templates } = useAsync((signal) => listCardTemplates({}, signal), []);

  const effectiveType: MessageType = messageType || channel.policy?.message_type || 'card_message';
  const templateOptions = useMemo(
    () => (templates?.items ?? []).filter((template) => template.kind === effectiveType),
    [templates, effectiveType],
  );

  const submit = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const response = await sendTestMessage(channel.id, {
        kind,
        ...(messageType ? { message_type: messageType } : {}),
        ...(templateId ? { card_template_id: templateId } : {}),
      });
      setResult(response);
      toast.success('测试消息已提交发送');
    } catch (cause) {
      setError(cause);
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <Card
        title="发送测试消息"
        subtitle="POST /channels/:id/test-message —— 用真实凭据向该渠道发送一条测试消息"
      >
        <div className="form-grid">
          <Field label="样例类型（kind）" required>
            <RadioGroup<'firing' | 'resolved'>
              value={kind}
              options={[
                { value: 'firing', label: 'firing（未解决）' },
                { value: 'resolved', label: 'resolved（已恢复）' },
              ]}
              onChange={setKind}
            />
          </Field>
          <Field label="消息类型（message_type）" hint="留空表示跟随渠道策略">
            <Select
              value={messageType}
              onChange={(event) => {
                setMessageType(event.target.value as '' | MessageType);
                setTemplateId('');
              }}
            >
              <option value="">（跟随渠道策略）</option>
              <option value="card_message">{MESSAGE_TYPE_LABEL.card_message}</option>
              <option value="app_feed_card">{MESSAGE_TYPE_LABEL.app_feed_card}</option>
            </Select>
          </Field>
          <Field
            label="卡片模板（card_template_id）"
            hint={`仅列出 ${MESSAGE_TYPE_LABEL[effectiveType]} 模板；留空使用渠道策略中的模板`}
          >
            <Select value={templateId} onChange={(event) => setTemplateId(event.target.value)}>
              <option value="">（使用策略默认）</option>
              <Options
                options={templateOptions.map((template) => ({
                  value: template.id,
                  label: `${template.name}${template.is_default ? ' · 默认' : ''}`,
                }))}
              />
            </Select>
          </Field>
        </div>

        <div className="row">
          <Button variant="primary" loading={busy} onClick={() => void submit()}>
            发送测试消息
          </Button>
          <span className="faint small">
            当前渠道策略的消息类型：{MESSAGE_TYPE_LABEL[channel.policy?.message_type ?? 'card_message']}
          </span>
        </div>
      </Card>

      {error ? <Banner tone="error">{errorMessage(error)}</Banner> : null}

      {result ? (
        <Card title="发送结果">
          <Banner tone={result.ok ? 'success' : 'warn'}>
            {result.ok ? '测试消息已发送' : '服务端返回 ok=false，请查看下方原始响应'}
          </Banner>
          <div className="kv-grid mb-16">
            <dt>delivery_id</dt>
            <dd className="mono break-all">
              {result.delivery_id ? <Link to="/deliveries">{result.delivery_id}</Link> : '—'}
            </dd>
            <dt>message_id</dt>
            <dd className="mono break-all">{result.message_id ?? '—'}</dd>
          </div>
          <Field label="原始响应">
            <JsonBlock value={result} maxHeight={260} />
          </Field>
        </Card>
      ) : null}

      <Card title="说明">
        <ul className="small muted" style={{ margin: 0, paddingLeft: 18 }}>
          <li>测试消息会真实调用飞书 API，并在投递记录中生成一条 kind=test 的 Delivery。</li>
          <li>
            凭据未配置或飞书返回非 0 code 时返回 <code>502 lark_api_error</code>（details 含 code / log_id）。
          </li>
          <li>渠道被禁用（enabled=false）时投递会被拒绝（R13），请先在详情页启用渠道。</li>
        </ul>
      </Card>
    </>
  );
}
