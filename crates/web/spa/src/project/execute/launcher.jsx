import { EditButton } from '../../ui/permissions.jsx';
import { Alert, Form, Input, Typography } from 'antd';
import { useId, useRef, useState } from 'react';
import { apiPost } from '../../api.js';
import { newId } from '../../fleet/model.js';
import { TodoActions, TodoSection } from '../forms/section.jsx';

export function launchInput(prompt, raw) {
  const input = JSON.parse(raw || '{}');
  if (!input || typeof input !== 'object' || Array.isArray(input)) throw new Error('参数必须是 JSON 对象');
  return { ...input, prompt };
}

export function CapabilityLauncher({ capability, todoId, onCreated, prompt: initialPrompt = '' }) {
  const id = useId();
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
  return <Form layout="vertical" className="project-todo-form" onFinish={send}>
    <TodoSection title={capability.summary || capability.target || '执行任务'}>
    {capability.input_desc && <Typography.Paragraph className="project-todo-field-full" style={{ margin: 0 }}>{capability.input_desc}</Typography.Paragraph>}
    {capability.unavailable_reason && <Alert className="project-todo-field-full" type="error" title={capability.unavailable_reason} />}
    {error && <Alert className="project-todo-field-full" type="error" title={error} />}
    <Form.Item className="project-todo-field-full" label="执行任务" htmlFor={`${id}-prompt`}>
      <Input.TextArea id={`${id}-prompt`} aria-label="执行任务" autoSize={{ minRows: 6, maxRows: 18 }} value={prompt} onChange={(event) => setPrompt(event.target.value)} disabled={busy || !!attempt.current} />
    </Form.Item>
    <Form.Item className="project-todo-field-full" label="能力输入参数" htmlFor={`${id}-inputs`} extra={capability.required_inputs?.length ? `必填参数：${capability.required_inputs.join('、')}` : '使用 JSON 对象填写命名参数'}>
      <Input.TextArea id={`${id}-inputs`} aria-label="能力输入参数" autoSize={{ minRows: 5, maxRows: 16 }} value={raw} onChange={(event) => setRaw(event.target.value)} disabled={busy || !!attempt.current} placeholder="能力需要的命名参数（JSON）" />
    </Form.Item>
    </TodoSection>
    <TodoActions><EditButton type="primary" htmlType="submit" loading={busy} disabled={!!capability.unavailable_reason || !prompt.trim()}>开始执行</EditButton></TodoActions>
  </Form>;
}
