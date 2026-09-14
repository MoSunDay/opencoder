import { Alert, Button, Collapse, Form, Input, Select, Segmented, Space, Typography } from 'antd';
import { useRef, useState } from 'react';
import { apiPost } from '../../api.js';
import { useNodes } from '../../fleet/useNodes.js';
import { explicitNodeOptions, newId } from '../../fleet/model.js';
import { launchBody } from './model.js';
export function Launch({ plans, onCreated, initialPlan }) {
  const { nodes, error: nodeError } = useNodes(); const [form] = Form.useForm();
  const [mode, setMode] = useState(initialPlan ? 'fixed' : 'dynamic'); const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const attempt = useRef(null);
  const options = plans.map((p) => ({ value: `${p.id}@${p.latest_version}`, label: `${p.title} · v${p.latest_version}${p.stable_version === p.latest_version ? ' · 稳定' : ' · 草稿'}` }));
  if (initialPlan && !options.some((option) => option.value === initialPlan)) options.push({ value: initialPlan, label: initialPlan });
  const submit = async (values) => {
    if (busy) return; setBusy(true); setError('');
    try {
      const body = launchBody({ ...values, mode }, ''); const signature = JSON.stringify(body);
      if (attempt.current?.signature !== signature) attempt.current = { signature, id: newId('brain') };
      const index = await apiPost('/api/brain/runs', { ...body, id: attempt.current.id });
      if (!index.id) throw new Error('未收到运行回执'); attempt.current = null; onCreated(index.id);
    } catch (e) { setError(e.message); } finally { setBusy(false); }
  };
  return <div className="brain-launch"><Typography.Title level={4}>让大脑组织这次执行</Typography.Title>
    <Typography.Paragraph type="secondary">明确目标与交付物，大脑连接现有能力，展开可并行的步骤。</Typography.Paragraph>
    {(error || nodeError) && <Alert type="error" showIcon title={error || nodeError} />}
    <Form form={form} layout="vertical" onFinish={submit} initialValues={{ inputs: '', plan: initialPlan }} disabled={busy}>
      <Form.Item label="执行方式"><Segmented value={mode} onChange={setMode} options={[{ value: 'dynamic', label: '动态规划' }, { value: 'fixed', label: '固定计划' }]} /></Form.Item>
      <Typography.Paragraph type="secondary">{mode === 'dynamic' ? '参考能力库和已有计划，一次生成完整计划，校验后自动执行。' : '直接执行指定计划版本，保留完整版本与运行记录。'}</Typography.Paragraph>
      {mode === 'fixed' ? <Form.Item name="plan" label="计划版本" rules={[{ required: true, message: '请选择计划版本' }]}><Select showSearch optionFilterProp="label" options={options} placeholder="明确选择一个版本" /></Form.Item>
        : <Form.Item name="references" label="参考已有计划（可选）"><Select mode="multiple" options={options} placeholder="用于规划参考" /></Form.Item>}
      <Form.Item name="objective" label="目标和交付物" rules={[{ required: true, whitespace: true, message: '请输入目标和交付物' }]}><Input.TextArea rows={4} placeholder="希望完成什么？哪些结果能证明任务完成？" /></Form.Item>
      <Form.Item name="node" label="大脑所在节点" rules={[{ required: true, message: '请选择节点' }]}><Select options={explicitNodeOptions(nodes, 'brain')} placeholder="选择持久保存本次运行的节点" /></Form.Item>
      <Collapse size="small" ghost className="brain-launch-advanced" items={[{ key: 'advanced', label: '高级选项', children: <Form.Item name="inputs" label="初始输入（可选）" tooltip="为计划已声明的输入端口预填参数（JSON 对象）。普通任务无需填写，运行中会按需询问。"><Input.TextArea rows={3} spellCheck={false} placeholder="留空即可，运行中会按需询问" /></Form.Item> }]} />
      <Space><Button type="primary" htmlType="submit" loading={busy}>{mode === 'dynamic' ? '规划并执行' : '执行指定版本'}</Button></Space>
    </Form>
  </div>;
}
