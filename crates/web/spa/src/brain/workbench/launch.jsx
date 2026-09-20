import { Alert, Button, Form, Input, InputNumber, Select, Space, Typography } from 'antd';
import { useRef, useState } from 'react';
import { apiPost } from '../../api.js';
import { useNodes } from '../../fleet/useNodes.js';
import { explicitNodeOptions, newId } from '../../fleet/model.js';
import { launchBody } from './model.js';
export function Launch({ capabilities = [], onCreated }) {
  const { nodes, error: nodeError } = useNodes(); const [form] = Form.useForm();
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const attempt = useRef(null);
  const options = capabilities.map((capability) => ({ value: capability.capability_id || capability.id, label: `${capability.kind} · ${capability.target} · ${capability.summary || capability.id}` }));
  const submit = async (values) => {
    if (busy) return; setBusy(true); setError('');
    try {
      const body = launchBody(values, ''); const signature = JSON.stringify(body);
      if (attempt.current?.signature !== signature) attempt.current = { signature, id: newId('brain') };
      const index = await apiPost('/api/brain/runs', { ...body, id: attempt.current.id });
      if (index.schema_version !== 3 || index.run_id !== attempt.current.id) throw new Error('未收到匹配的 v3 运行回执');
      attempt.current = null; onCreated(index.run_id);
    } catch (e) { setError(e.message); } finally { setBusy(false); }
  };
  return <div className="brain-launch"><Typography.Title level={4}>让大脑组织这次执行</Typography.Title>
    <Typography.Paragraph type="secondary">明确目标与交付物，大脑按轮次选择能力，等待本轮执行结束后判断下一步。</Typography.Paragraph>
    {(error || nodeError) && <Alert type="error" showIcon title={error || nodeError} />}
    <Form form={form} layout="vertical" onFinish={submit} initialValues={{ engineering: [], capability_ids: [], max_rounds: 32 }} disabled={busy}>
      <Form.Item name="objective" label="目标和交付物" rules={[{ required: true, whitespace: true, message: '请输入目标和交付物' }]}><Input.TextArea rows={4} placeholder="希望完成什么？哪些结果能证明任务完成？" /></Form.Item>
      <Form.Item name="node" label="大脑所在节点" rules={[{ required: true, message: '请选择节点' }]}><Select options={explicitNodeOptions(nodes, 'brain')} placeholder="选择持久保存本次运行的节点" /></Form.Item>
      <Form.Item name="capability_ids" label="可使用的能力（可选）"><Select mode="multiple" showSearch optionFilterProp="label" options={options} placeholder="留空则从已注册能力中选择" /></Form.Item>
      <Form.Item name="max_rounds" label="最大调度轮次" rules={[{ required: true }]}><InputNumber min={1} max={256} precision={0} /></Form.Item>
      <Form.Item label="工程描述（可选）" tooltip="按名称传递工程参数，值支持 JSON；留空时使用目标作为输入。">
        <Form.List name="engineering">{(fields, { add, remove }) => <>
          {fields.map((field) => <div key={field.key} style={{ display: 'flex', gap: 8, marginBottom: 8 }}>
            <Form.Item name={[field.name, 'key']} style={{ flex: 1, marginBottom: 0 }}><Input aria-label="工程参数名" placeholder="参数名" /></Form.Item>
            <Form.Item name={[field.name, 'value']} style={{ flex: 2, marginBottom: 0 }}><Input.TextArea autoSize={{ minRows: 1, maxRows: 5 }} aria-label="工程参数值" placeholder="JSON 值" /></Form.Item>
            <Button type="text" danger onClick={() => remove(field.name)}>移除</Button>
          </div>)}
          <Button type="dashed" onClick={() => add({ key: '', value: '' })}>添加工程参数</Button>
        </>}</Form.List>
      </Form.Item>
      <Space><Button type="primary" htmlType="submit" loading={busy}>开始调度</Button></Space>
    </Form>
  </div>;
}
