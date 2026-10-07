import { Alert, Button, Input, Space, Typography } from 'antd';
import { useRef, useState } from 'react';
import { apiPost } from '../../api.js';
import { newId } from '../../fleet/model.js';

export function launchInput(prompt, raw) {
  const input = JSON.parse(raw || '{}');
  if (!input || typeof input !== 'object' || Array.isArray(input)) throw new Error('参数必须是 JSON 对象');
  return { ...input, prompt };
}

export function CapabilityLauncher({ capability, todoId, onCreated, prompt: initialPrompt = '' }) {
  const [prompt, setPrompt] = useState(initialPrompt);
  const [raw, setRaw] = useState('{}');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const attempt = useRef(null);
  const sending = useRef(false);
  const send = async () => {
    if (!capability || sending.current) return;
    sending.current = true; setBusy(true);
    try {
      const input = launchInput(prompt, raw);
      const signature = JSON.stringify([todoId, capability.id, input]);
      if (attempt.current && attempt.current.signature !== signature) {
        throw new Error('上次提交结果尚未确认，请先重试原请求或按保留的执行 ID 查看结果');
      }
      if (!attempt.current) attempt.current = { signature, id: newId(capability.kind) };
      const body = await apiPost(`/api/project/todos/${encodeURIComponent(todoId)}/dispatch`, {
        execution_id: attempt.current.id, capability_id: capability.id, input,
      });
      setError('');
      onCreated(body.execution_id);
    } catch (failure) {
      // Definitive client rejection did not accept an execution. A lost reply
      // or failed link keeps the original id and input for safe retries.
      if (!failure.body?.accepted && [400, 404, 422].includes(failure.status)) attempt.current = null;
      setError(`${failure.message}${attempt.current ? `；执行 ID：${attempt.current.id}，重试将使用同一 ID` : ''}`);
    } finally { sending.current = false; setBusy(false); }
  };
  if (!capability) return <Alert type="error" title="所选能力已不可读取，请返回重新选择" />;
  return <Space orientation="vertical" style={{ width: '100%' }}>
    <Typography.Text strong>{capability.summary || capability.target}</Typography.Text>
    <Typography.Paragraph>{capability.input_desc}</Typography.Paragraph>
    {capability.unavailable_reason && <Alert type="error" title={capability.unavailable_reason} />}
    {error && <Alert type="error" title={error} />}
    <Input.TextArea aria-label="执行任务" rows={6} value={prompt} onChange={(event) => setPrompt(event.target.value)} disabled={busy || !!attempt.current} />
    {!!capability.required_inputs?.length && <Typography.Text>必填参数：{capability.required_inputs.join('、')}</Typography.Text>}
    <Input.TextArea aria-label="能力输入参数" rows={5} value={raw} onChange={(event) => setRaw(event.target.value)} disabled={busy || !!attempt.current} placeholder="能力需要的命名参数（JSON）" />
    <Button type="primary" loading={busy} disabled={!!capability.unavailable_reason || !prompt.trim()} onClick={send}>开始执行</Button>
  </Space>;
}
