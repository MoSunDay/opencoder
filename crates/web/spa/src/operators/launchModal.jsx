// launchModal.jsx — 启动 Operator 执行的表单 modal：节点只读、agent 选择
// （GET /api/agents，默认 act）、HarnessFields（默认 opencoder）、必填
// 任务要求。提交 POST /api/executions {kind:'operator', node_id, input}，
// 幂等尝试 ref 与 agentsConfig.jsx 的 run 同款（同签名复用同一 id，受理
// 成功后清空）。受理后交回 onAccepted(accepted) 打开 ExecutionDetail。

import { Button, Form, Input, Modal, Select } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from '../api.js';
import { newId } from '../fleet/model.js';
import { HarnessFields } from '../harness/fields.jsx';
import { err } from '../notice.js';

export function LaunchModal({ node, onClose, onAccepted, onNotice }) {
  const [form] = Form.useForm();
  const [agents, setAgents] = useState([]);
  const [busy, setBusy] = useState(false);
  const attempt = useRef(null); // {signature, id} — 同签名重试复用同一执行 id

  useEffect(() => {
    let active = true;
    apiGet('/api/agents')
      .then((j) => {
        if (active) {
          setAgents((j.agents || []).map((a) => ({ value: a.name, label: a.name })));
        }
      })
      .catch((e) => {
        if (active) {
          onNotice(err('加载 agent 失败: ' + (e && e.message)));
        }
      });
    return () => { active = false; };
  }, [onNotice]);

  const submit = async (values) => {
    const request = {
      kind: 'operator',
      target: values.agent || 'act',
      node_id: node.id,
      input: { prompt: values.prompt, harness: values.harness || 'opencoder' },
    };
    const signature = JSON.stringify(request);
    if (attempt.current?.signature !== signature) attempt.current = { signature, id: newId('operator') };
    setBusy(true);
    try {
      const accepted = await apiPost('/api/executions', { ...request, id: attempt.current.id });
      attempt.current = null;
      onClose();
      onAccepted(accepted);
    } catch (e) {
      onNotice(err(`${e.message}；再次启动会继续确认同一执行`));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      title={`启动 Operator · ${node?.name || ''}`}
      onCancel={onClose}
      footer={null}
      destroyOnHidden
    >
      <Form form={form} layout="vertical" onFinish={submit} initialValues={{ agent: 'act' }}>
        <Form.Item label="节点">
          <Input value={node ? `${node.name}（${node.id}）` : ''} disabled />
        </Form.Item>
        <Form.Item name="agent" label="Agent">
          <Select options={agents} aria-label="operator-agent" showSearch />
        </Form.Item>
        <HarnessFields initialHarness="opencoder" environments={false} />
        <Form.Item name="prompt" label="任务要求" rules={[{ required: true, message: '请输入任务要求' }]}>
          <Input.TextArea rows={5} />
        </Form.Item>
        <Button type="primary" htmlType="submit" loading={busy}>启动并查看</Button>
      </Form>
    </Modal>
  );
}
