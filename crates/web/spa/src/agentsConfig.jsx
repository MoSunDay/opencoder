// agentsConfig.jsx — 菜单页「Agent 配置」：生效 agent 选择（PATCH
// /api/agents/active；清空 = null 跟随默认链，预检失败 400 经 onNotice
// 透出）、agent 卡片表（四类引用 tag、生效徽标、配置/删除 —— 删除生效
// 卡片合法，服务端自动清 marker）、新建 modal（name + 四类资源 Select，
// 数据来自各池 GET /api/agents/resources/:cat）与 NFS 导出卡片。详情视图
// 在本页内切换到 agentDetail.jsx，不新增路由页。

import {
  Badge, Button, Card, Col, Form, Input, Modal, Popconfirm, Row, Select, Space, Table, Tag, Typography, message,
} from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPatch, apiPost } from './api.js';
import { REF_FIELDS, refCells, resourceOptions } from './agentsItems.js';
import { AgentDetail } from './agentDetail.jsx';
import { AgentNfsCard } from './agentNfsCard.jsx';
import { ExecutionDetail } from './fleet/detail.jsx';
import { newId, nodeOptions } from './fleet/model.js';
import { PageShell } from './shell/pageShell.jsx';

const { Text } = Typography;

/// 新建卡片 modal：name + 四类引用（可清空 ⇒ null）。409 重名等服务端
/// error 经 onNotice 透出。
function CreateAgentModal({ open, resources, onClose, onCreated, onNotice }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  const submit = async (values) => {
    setSaving(true);
    try {
      const current = {};
      REF_FIELDS.forEach(({ field }) => {
        current[field] = values[field] || null;
      });
      await apiPost('/api/agents', { name: values.name, current });
      message.success('已创建');
      onCreated(values.name);
    } catch (e) {
      onNotice('新建 agent 失败: ' + (e && e.message));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal open={open} title="新建 Agent" onCancel={onClose} footer={null} destroyOnHidden>
      <Form form={form} layout="vertical" onFinish={submit}>
        <Form.Item name="name" label="名称" rules={[{ required: true, message: '请输入名称' }]}>
          <Input placeholder="reviewer" aria-label="new-agent-name" />
        </Form.Item>
        {REF_FIELDS.map(({ field, label, cat }) => (
          <Form.Item key={field} name={field} label={label} initialValue={undefined}>
            <Select
              allowClear
              placeholder="不引用"
              options={resourceOptions(resources[cat])}
              aria-label={`new-agent-${field}`}
            />
          </Form.Item>
        ))}
        <Space>
          <Button type="primary" htmlType="submit" loading={saving}>创建</Button>
          <Button onClick={onClose}>取消</Button>
        </Space>
      </Form>
    </Modal>
  );
}

export function AgentsPanel({ onNotice }) {
  const [agents, setAgents] = useState([]);
  const [active, setActive] = useState(null);
  const [resources, setResources] = useState({ prompts: [], skills: [], tools: [], memory: [] });
  const [loading, setLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [detail, setDetail] = useState(''); // 打开详情的 agent 名；'' = 列表视图
  const [nodes, setNodes] = useState([]);
  const [launch, setLaunch] = useState(null);
  const [execution, setExecution] = useState(null);
  const [launching, setLaunching] = useState(false);
  const [launchForm] = Form.useForm();
  const launchAttempt = useRef(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [j, prompts, skills, tools, memory, fleet] = await Promise.all([
        apiGet('/api/agents'),
        apiGet('/api/agents/resources/prompts'),
        apiGet('/api/agents/resources/skills'),
        apiGet('/api/agents/resources/tools'),
        apiGet('/api/agents/resources/memory'),
        apiGet('/api/nodes'),
      ]);
      setAgents((j && j.agents) || []);
      setActive((j && j.active) || null);
      setNodes((fleet && fleet.nodes) || []);
      setResources({
        prompts: (prompts && prompts.resources) || [],
        skills: (skills && skills.resources) || [],
        tools: (tools && tools.resources) || [],
        memory: (memory && memory.resources) || [],
      });
    } catch (e) {
      onNotice('获取 agent 列表失败: ' + (e && e.message));
    } finally {
      setLoading(false);
    }
  }, [onNotice]);

  useEffect(() => {
    load();
  }, [load]);

  const patchActive = async (value) => {
    try {
      const j = await apiPatch('/api/agents/active', { active: value || null });
      setActive((j && j.active) || null);
      message.success(value ? `已激活 ${value}` : '已恢复默认链');
      load();
    } catch (e) {
      onNotice('切换生效 agent 失败: ' + (e && e.message));
      load(); // 回滚到服务端视角
    }
  };

  const remove = async (name) => {
    try {
      await apiDel(`/api/agents/${encodeURIComponent(name)}`);
      message.success('已删除');
      load();
    } catch (e) {
      onNotice('删除 agent 失败: ' + (e && e.message));
    }
  };

  const run = async (values) => {
    const request = { kind: 'agent', target: launch.name, node_id: values.node || null, input: { prompt: values.prompt } };
    const signature = JSON.stringify(request);
    if (launchAttempt.current?.signature !== signature) launchAttempt.current = { signature, id: newId('agent') };
    setLaunching(true);
    try {
      const accepted = await apiPost('/api/executions', { ...request, id: launchAttempt.current.id });
      launchAttempt.current = null; setLaunch(null); setExecution(accepted);
    } catch (e) {
      onNotice(`${e.message}；再次启动会继续确认同一执行`);
    } finally { setLaunching(false); }
  };

  if (detail) {
    return (
      <AgentDetail
        name={detail}
        resources={resources}
        onNotice={onNotice}
        onChanged={load}
        onBack={() => setDetail('')}
      />
    );
  }

  const columns = [
    {
      title: '名称',
      dataIndex: 'name',
      key: 'name',
      render: (v) => (
        <Space size={4}>
          <Text strong>{v}</Text>
          {active === v ? <Tag color="blue">生效中</Tag> : null}
        </Space>
      ),
    },
    {
      title: '当前引用',
      key: 'refs',
      render: (_, r) => (
        <Space size={[4, 4]} wrap>
          {refCells(r.current).map(({ field, label, value }) => (
            <Tag key={field} color={value ? 'geekblue' : 'default'}>{label}: {value || '—'}</Tag>
          ))}
        </Space>
      ),
    },
    { title: '更新时间', dataIndex: 'updated_at', key: 'updated_at', render: (v) => v || '-' },
    {
      title: '操作',
      key: 'ops',
      render: (_, r) => (
        <Space size={0}>
          <Button size="small" type="link" onClick={() => setDetail(r.name)}>配置</Button>
          <Button size="small" type="link" onClick={() => { launchForm.resetFields(); setLaunch(r); }}>启动</Button>
          <Popconfirm title={`删除 agent ${r.name}？`} okText="确认删除" onConfirm={() => remove(r.name)}>
            <Button size="small" type="link" danger>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <PageShell page="agents">
        <Row gutter={[16, 16]}>
          <Col xs={24} xl={17}>
            <Card
            size="small"
            title="生效 Agent"
            extra={<Button size="small" onClick={load}>刷新</Button>}
          >
            <Space wrap>
              <Select
                allowClear
                style={{ minWidth: 260 }}
                placeholder="跟随默认链（未指定）"
                value={active || undefined}
                onChange={patchActive}
                options={agents.map((a) => ({ value: a.name, label: a.name }))}
                aria-label="active-agent"
              />
              {active
                ? <Badge status="success" text={`当前: ${active}`} />
                : <Badge status="default" text={<Text type="secondary">跟随默认链</Text>} />}
              <Text type="secondary" style={{ fontSize: 12 }}>
                切换后，新会话将优先使用该 Agent；未选择时使用默认配置。
              </Text>
            </Space>
          </Card>
          <Card
            size="small"
            title="Agent 列表"
            style={{ marginTop: 16 }}
            extra={<Button size="small" onClick={() => setCreating(true)}>新建</Button>}
          >
            <Table
              rowKey="name"
              size="small"
              columns={columns}
              dataSource={agents}
              loading={loading}
              pagination={false}
              scroll={{ x: 'max-content' }}
              locale={{ emptyText: '暂无 agent' }}
            />
          </Card>
        </Col>
        <Col xs={24} xl={7}>
          <AgentNfsCard onNotice={onNotice} />
        </Col>
        <CreateAgentModal
          open={creating}
          resources={resources}
          onNotice={onNotice}
          onClose={() => {
            setCreating(false);
          }}
          onCreated={(name) => {
            setCreating(false);
            setDetail(name);
            load();
          }}
        />
        <Modal open={!!launch} title={`启动 Agent · ${launch?.name || ''}`} onCancel={() => setLaunch(null)} footer={null} destroyOnHidden>
          <Form form={launchForm} layout="vertical" onFinish={run} initialValues={{ node: '' }}>
            <Form.Item name="node" label="执行节点"><Select options={nodeOptions(nodes, 'agent')} /></Form.Item>
            <Form.Item name="prompt" label="任务要求" rules={[{ required: true, message: '请输入任务要求' }]}><Input.TextArea rows={5} /></Form.Item>
            <Button type="primary" htmlType="submit" loading={launching}>启动并查看</Button>
          </Form>
        </Modal>
        {execution && <ExecutionDetail id={execution.id} summary={execution} onClose={() => setExecution(null)} onNotice={onNotice} />}
      </Row>
    </PageShell>
  );
}
