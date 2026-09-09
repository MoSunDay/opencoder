import { Alert, Button, Form, InputNumber, Modal, Select } from 'antd';
import { useState } from 'react';
import { apiPut } from '../../api.js';
import { err } from '../../notice.js';

export function NodeSchedulingModal({ node, onClose, onSaved, onNotice }) {
  const [saving, setSaving] = useState(false);
  const save = async (settings) => {
    setSaving(true);
    try {
      await apiPut(`/api/nodes/${encodeURIComponent(node.id)}/scheduling`, settings);
      await onSaved(); onClose();
    } catch (e) { onNotice(err(`保存节点调度配置失败：${e.message}`)); }
    finally { setSaving(false); }
  };
  return <Modal open title={`节点调度 · ${node.name}`} onCancel={onClose} footer={null}>
    <Alert type="info" showIcon title="超过并发上限的任务进入 pending" description="空出执行名额后自动调度。降低上限不会中断已运行任务；设置与待执行队列在节点重启后保留。" style={{ marginBottom: 16 }} />
    <Form layout="vertical" onFinish={save} initialValues={{ max_runs: node.snapshot?.max_runs, queue_order: node.snapshot?.queue_order || 'fifo' }}>
      <Form.Item name="max_runs" label="最大并发任务数" rules={[{ required: true, type: 'number', min: 1, max: 65535, message: '请输入 1–65535 的整数' }, { validator: (_, v) => Number.isInteger(v) ? Promise.resolve() : Promise.reject(new Error('并发数必须为整数')) }]}>
        <InputNumber min={1} max={65535} precision={0} aria-label="node-max-runs" />
      </Form.Item>
      <Form.Item name="queue_order" label="排队顺序" rules={[{ required: true }]}>
        <Select aria-label="node-queue-order" options={[{ value: 'fifo', label: '先入先出 FIFO' }, { value: 'lifo', label: '后入先出 LIFO' }]} />
      </Form.Item>
      <Button type="primary" htmlType="submit" loading={saving}>保存调度配置</Button>
    </Form>
  </Modal>;
}
