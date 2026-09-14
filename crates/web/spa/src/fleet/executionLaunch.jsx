import { Button, Form, Input, Modal, Select, Space } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiPost } from '../api.js';
import { CREATABLE_KINDS, newId, nodeOptions } from './model.js';
import { err } from '../notice.js';
import { HarnessFields, parseEnvs } from '../harness/fields.jsx';

// 启动执行 Modal：表单从 executions 页原样搬入，提交/幂等逻辑逐字等价；成功交
// onLaunched(result) 由父级关窗并接管（开抽屉 + 刷新列表），失败保持弹窗打开。
export function ExecutionLaunchModal({ open, nodes, onClose, onLaunched, onNotice }) {
  const [kind, setKind] = useState('agent'); const [busy, setBusy] = useState(false);
  const attempt = useRef(null); const [form] = Form.useForm();
  /// 关窗即复位：kind 回到默认 agent、幂等 attempt 清空；destroyOnHidden 负责
  /// 表单 DOM，这里只补非 DOM 态，保证下次打开是全新一次启动。
  useEffect(() => { if (!open) { setKind('agent'); attempt.current = null; } }, [open]);
  const submit = async (values) => {
    const input = { prompt: values.prompt || '' };
    if (kind === 'agent') {
      if (values.harness && values.harness !== 'default') input.harness = values.harness;
      input.envs = parseEnvs(values.envs);
    }
    if (kind === 'project') input.action = 'plan';
    const request = { kind, target: values.target || null, node_id: values.node || null, input };
    const signature = JSON.stringify(request);
    if (!attempt.current || attempt.current.signature !== signature) attempt.current = { signature, id: kind === 'project' ? `project-${values.target}` : newId(kind) };
    setBusy(true);
    try { const result = await apiPost('/api/executions', { ...request, id: attempt.current.id }); onNotice(err('')); attempt.current = null; onLaunched(result); }
    catch (e) { onNotice(err(`${e.message}；保持内容不变再次启动，会继续确认同一次执行`)); }
    finally { setBusy(false); }
  };
  return <Modal open={open} title="启动执行" width={720} footer={null} destroyOnHidden onCancel={onClose}>
    <Form form={form} layout="vertical" onFinish={submit} initialValues={{ node: '' }}>
      <Space align="start" wrap>
        <Form.Item label="执行类型"><Select style={{ width: 150 }} value={kind} onChange={setKind} options={CREATABLE_KINDS} /></Form.Item>
        <Form.Item name="target" label="Agent / 团队 / 工作流定义 / 项目任务" rules={[{ required: true }]}><Input style={{ width: 300 }} placeholder={kind === 'todos' ? '模板名/v1' : 'act / 定义名称 / 任务 ID'} /></Form.Item>
        <Form.Item name="node" label="调度节点"><Select style={{ width: 330 }} options={nodeOptions(nodes, kind)} /></Form.Item>
      </Space>
      <Form.Item name="prompt" label="任务要求"><Input.TextArea rows={3} /></Form.Item>
      {kind === 'agent' && <HarnessFields initialHarness="default" inherit />}
      <Button type="primary" htmlType="submit" loading={busy}>启动执行</Button>
    </Form>
  </Modal>;
}
