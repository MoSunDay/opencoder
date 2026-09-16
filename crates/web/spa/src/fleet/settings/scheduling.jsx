import { Alert, Button, Form, Input, InputNumber, Modal, Select } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet, apiPut } from '../../api.js';
import { err } from '../../notice.js';

export function NodeSchedulingModal({ node, onClose, onSaved, onNotice }) {
  const [saving, setSaving] = useState(false);
  const [initials, setInitials] = useState(null);
  useEffect(() => {
    let alive = true;
    setInitials(null);
    // 节点快照（/api/nodes）不带 workdir，当前配置必须从节点的
    // maintenance "scheduling" 读接口获取；读取失败再退回快照字段。
    // workdir_supported=false 表示 multi-runtime host：会话目录由各
    // 运行时自身决定，节点级工作空间不可配置。
    apiGet(`/api/nodes/${encodeURIComponent(node.id)}/scheduling`).then((s) => {
      if (alive) setInitials({
        max_runs: s?.max_runs,
        queue_order: s?.queue_order || 'fifo',
        workdir: s?.workdir || '',
        workdir_supported: s?.workdir_supported !== false,
      });
    }).catch(() => {
      if (alive) setInitials({ max_runs: node.snapshot?.max_runs, queue_order: node.snapshot?.queue_order || 'fifo', workdir: '', workdir_supported: true });
    });
    return () => { alive = false; };
  }, [node.id]);
  const save = async (values) => {
    setSaving(true);
    try {
      const payload = { max_runs: values.max_runs, queue_order: values.queue_order };
      if (initials?.workdir_supported !== false) payload.workdir = values.workdir?.trim() ? values.workdir.trim() : null;
      await apiPut(`/api/nodes/${encodeURIComponent(node.id)}/scheduling`, payload);
      await onSaved(); onClose();
    } catch (e) { onNotice(err(`保存节点调度配置失败：${e.message}`)); }
    finally { setSaving(false); }
  };
  return <Modal open title={`节点调度 · ${node.name}`} onCancel={onClose} footer={null}>
    {initials === null ? <div style={{ textAlign: 'center', padding: 32 }}>加载中…</div> : <>
      <Alert type="info" showIcon title="超过并发上限的任务进入 pending" description={initials.workdir_supported === false
        ? '多运行时宿主不支持节点级工作空间，会话目录由各运行时自身决定。'
        : '设置工作空间后，该节点的 opencoder 会话在指定目录中运行。空出执行名额后自动调度。降低上限不会中断已运行任务；设置与待执行队列在节点重启后保留。'} style={{ marginBottom: 16 }} />
      <Form layout="vertical" onFinish={save} initialValues={initials}>
      <Form.Item name="max_runs" label="最大并发任务数" rules={[{ required: true, type: 'number', min: 1, max: 65535, message: '请输入 1–65535 的整数' }, { validator: (_, v) => Number.isInteger(v) ? Promise.resolve() : Promise.reject(new Error('并发数必须为整数')) }]}>
        <InputNumber min={1} max={65535} precision={0} aria-label="node-max-runs" />
      </Form.Item>
      <Form.Item name="queue_order" label="排队顺序" rules={[{ required: true }]}>
        <Select aria-label="node-queue-order" options={[{ value: 'fifo', label: '先入先出 FIFO' }, { value: 'lifo', label: '后入先出 LIFO' }]} />
      </Form.Item>
      {initials.workdir_supported !== false && <Form.Item name="workdir" label="工作空间（可选）" extra="opencoder 会话的工作目录；留空使用节点启动目录" rules={[{ validator: (_, v) => !v || !v.trim() || v.trim().startsWith('/') ? Promise.resolve() : Promise.reject(new Error('工作空间必须是绝对路径')) }]}>
        <Input aria-label="node-workdir" placeholder="/data/work/workspace" allowClear />
      </Form.Item>}
      <Button type="primary" htmlType="submit" loading={saving}>保存调度配置</Button>
    </Form>
    </>}
  </Modal>;
}
