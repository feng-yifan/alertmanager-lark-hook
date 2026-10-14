import { useState } from 'react';
import { Link } from 'react-router-dom';
import { errorMessage, listCardTemplates, sendTestMessage } from '../../api';
import { Banner, Button, Card, Field, JsonBlock, Options, RadioGroup, Select, StatusBadge } from '../../components/ui';
import { useAsync } from '../../hooks';
import { useToast } from '../../toast';
import type { Channel, Delivery } from '../../types';

interface Props {
  channel: Channel;
}

export default function TestMessageTab({ channel }: Props): JSX.Element {
  const toast = useToast();
  const [kind, setKind] = useState<'firing' | 'resolved'>('firing');
  const [templateId, setTemplateId] = useState('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Delivery | null>(null);
  const [error, setError] = useState<unknown>(null);

  const { data: templates } = useAsync((signal) => listCardTemplates(signal), []);
  const templateOptions = templates ?? [];

  const submit = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const response = await sendTestMessage(channel.id, {
        sample: kind,
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
        subtitle="走完整投递链路（会写入「测试分组」与投递记录，可重试 / 撤回）；只验证卡片样式请用卡片编辑器里的「发送测试」"
      >
        <div className="form-grid">
          <Field label="样例类型" required>
            <RadioGroup<'firing' | 'resolved'>
              value={kind}
              options={[
                { value: 'firing', label: '未解决' },
                { value: 'resolved', label: '已恢复' },
              ]}
              onChange={setKind}
            />
          </Field>
          <Field
            label="卡片模板"
            hint="留空使用渠道策略中的模板"
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
        </div>
      </Card>

      {error ? <Banner tone="error">{errorMessage(error)}</Banner> : null}

      {result ? (
        <Card title="发送结果">
          <Banner tone={result.state === 'failed' ? 'error' : 'success'}>
            {result.state === 'failed'
              ? `发送失败${result.last_error ? `：${result.last_error}` : ''}`
              : '测试消息已提交发送。'}
          </Banner>
          <div className="kv-grid mb-16">
            <dt>投递 ID</dt>
            <dd className="mono break-all">
              <Link to="/deliveries">{result.id}</Link>
            </dd>
            <dt>飞书消息 ID</dt>
            <dd className="mono break-all">{result.lark_message_id ?? '—'}</dd>
            <dt>状态</dt>
            <dd>
              <StatusBadge kind="delivery" value={result.state} />
            </dd>
            <dt>最后错误</dt>
            <dd className="mono break-all">{result.last_error ?? '—'}</dd>
          </div>
          <Field label="原始响应">
            <JsonBlock value={result} maxHeight={260} />
          </Field>
        </Card>
      ) : null}

      <Card title="说明">
        <ul className="small muted" style={{ margin: 0, paddingLeft: 18 }}>
          <li>测试消息会真实调用飞书 API，并在投递记录中生成一条测试类型的记录。</li>
          <li>
            凭据未配置或飞书返回非 0 错误码时返回 <code>502</code>，错误体里带飞书错误码与请求日志 ID。
          </li>
          <li>渠道被停用时投递会被拒绝，请先在详情页启用渠道。</li>
        </ul>
      </Card>
    </>
  );
}
