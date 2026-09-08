import { RunEvents } from './events.jsx';
import { Alert, Button, Collapse, Space, Typography } from 'antd';
import { useState } from 'react';
import { Markdown } from '../markdown.jsx';
import { PayloadWindows } from '../../fleet/detail/fields.jsx';
import { downloadArtifact } from '../../fleet/download.js';

export function RunText({ id, value, label }) {
  if (value?.omitted) return <PayloadWindows id={id} marker={value} label={label} />;
  return typeof value === 'string' ? <Markdown text={value} /> : null;
}
export function RunReplay({ id, detail, onOpen }) {
  const [error, setError] = useState('');
  const run = detail.run;
  const trace = detail.replay || {};
  const download = async (artifact) => {
    try { await downloadArtifact(id, artifact.id, artifact.name); setError(''); }
    catch (e) { setError(e.message); }
  };
  return <Space orientation="vertical" style={{ width: '100%' }}>
    <Typography.Title level={5}>第 {run.version} 次 · {run.kind} · {run.agent}</Typography.Title>
    {detail.retention !== 'complete' && <Alert type="warning" showIcon title={detail.retention === 'incomplete_history' ? '历史记录缺少完整输入或过程归档' : '过程归档尚未完成，仅展示已持久化的数据'} />}
    {error && <Alert type="error" title={error} />}
    <Collapse items={[
      { key: 'input', label: '本次输入与 Agent 版本', children: <RunText id={id} value={run.input_snapshot} /> },
      { key: 'plan', label: '本次计划', children: <RunText id={id} value={run.plan_md} /> },
      { key: 'output', label: '本次输出', children: <RunText id={id} value={run.output_md} /> },
      { key: 'events', label: `本次过程事件（${trace.event_count || 0} 条）`, children: <RunEvents id={id} /> },
      { key: 'models', label: `模型请求与响应（${trace.model_calls || 0} 次）`, children: <Space orientation="vertical">{(trace.files || []).sort().map((file) => <PayloadWindows key={file} id={id} marker={{ read_via: 'detail_field', field: `archive.${file}` }} label={file} />)}</Space> },
    ]} />
    {run.session_id && <Button onClick={() => onOpen(run.session_id)}>打开对应会话</Button>}
    {(trace.children || []).map((child) => <Button key={child.task_id} onClick={() => onOpen(child.child_session_id)}>子任务：{child.agent} · {child.status}</Button>)}
    {(trace.artifacts || []).map((artifact) => <Space key={artifact.id}>
      <Button onClick={() => download(artifact)}>下载 {artifact.name}</Button>
      <Typography.Text type="secondary" copyable={{ text: artifact.sha256 }}>SHA-256 · {artifact.sha256.slice(0, 12)}</Typography.Text>
    </Space>)}
  </Space>;
}
