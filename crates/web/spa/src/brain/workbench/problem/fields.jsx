import { Alert, Button, Form, Image, Input, Select, Space, Typography, Upload } from 'antd';
import { useState } from 'react';
import { apiPost } from '../../../api.js';

export const isProblemPlan = (plan) => plan?.plan?.nodes?.some((node) => node.capability_ids?.some((id) => id.startsWith('pc-issue-')));

const read = (file) => new Promise((resolve, reject) => {
  const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.onerror = () => reject(new Error('图片读取失败')); reader.readAsDataURL(file);
});

function Images({ value = [], onChange, onBusy }) {
  const [previews, setPreviews] = useState({}); const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  const add = async (file) => {
    if (busy) return false;
    setError('');
    if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type) || file.size > 2 * 1024 * 1024 || value.length >= 4) {
      setError('最多 4 张 PNG、JPEG 或 WebP 图片，每张不超过 2 MiB'); return false;
    }
    setBusy(true); onBusy(true);
    try {
      const data = await read(file); const ref = await apiPost('/api/brain/attachments', { name: file.name, data_url: data });
      setPreviews((old) => ({ ...old, [ref.id]: data })); onChange([...value, ref]);
    } catch (e) { setError(e.message); } finally { setBusy(false); onBusy(false); }
    return false;
  };
  return <Space direction="vertical">{error && <Alert type="error" title={error} />}
    <Space wrap>{value.map((ref) => <Space direction="vertical" key={ref.id}>
      {previews[ref.id] && <Image width={100} src={previews[ref.id]} alt={ref.name} />}<Typography.Text>{ref.name}</Typography.Text>
      <Button disabled={busy} onClick={() => onChange(value.filter((r) => r.id !== ref.id))}>移除</Button>
    </Space>)}</Space>
    <Upload accept="image/png,image/jpeg,image/webp" beforeUpload={add} showUploadList={false} disabled={busy || value.length >= 4}><Button loading={busy}>添加问题图片</Button></Upload>
  </Space>;
}

export function ProblemFields({ onBusy, nodes = [] }) {
  const options = (kind) => nodes.filter((node) => node.online && node.kinds?.includes(kind)).map((node) => ({ value: node.id, label: node.name }));
  return <><Form.Item name="problemText" label="问题描述" rules={[{ required: true, whitespace: true, message: '请输入现象、期望和复现步骤' }]}>
    <Input.TextArea autoSize={{ minRows: 4, maxRows: 12 }} placeholder="描述实际现象、预期结果、复现步骤及已知应用版本" />
  </Form.Item><Form.Item name="problemImages" label="问题图片" initialValue={[]}><Images onBusy={onBusy} /></Form.Item>
    <Form.Item name={['settings', 'device_node']} label="设备执行节点" rules={[{ required: true }]}><Select options={options('dag')} /></Form.Item>
    <Form.Item name={['settings', 'build_node']} label="构建节点" rules={[{ required: true }]}><Select options={options('team')} /></Form.Item></>;
}
