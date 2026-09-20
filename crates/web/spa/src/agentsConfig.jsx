import {
  Button, Drawer, Form, Input, Modal, Popconfirm, Segmented, Select, Space, Table, Tabs, Typography,
} from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPost } from './api.js';
import { AgentDetail } from './agentDetail.jsx';
import { AgentNfsCard } from './agentNfsCard.jsx';
import { ExecutionDetail } from './fleet/detail.jsx';
import { err } from './notice.js';
import { useMessage } from './ui/appMessage.js';
import { newId, nodeOptions } from './fleet/model.js';
import { OperatorPanel } from './operators/panel.jsx';
import { PageShell } from './shell/pageShell.jsx';
import { HarnessFields, parseEnvs } from './harness/fields.jsx';
import { RUN_MODE_HINT, RUN_MODE_OPTIONS } from './agents/runMode.js';
import { HarnessManagement } from './harness/management.jsx';
import { useStore } from './store.js';

const { Text } = Typography;

/// 新建 modal：名称和执行方式，创建后直接进入资源配置。409 重名等服务端
/// error 经 onNotice 透出。
function CreateAgentModal({ open, onClose, onCreated, onNotice }) {
  const msg = useMessage();
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  const submit = async (values) => {
    setSaving(true);
    try {
      // run_mode 始终随卡片提交（缺省 operator），会话创建请求不带该字段。
      await apiPost('/api/agents', {
        name: values.name.trim(),
        current: {},
        harness: values.harness,
        run_mode: values.run_mode || 'operator',
      });
      msg.success('已创建');
      form.resetFields();
      onCreated(values.name.trim());
    } catch (e) {
      onNotice(err('新建 agent 失败: ' + (e && e.message)));
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
        <HarnessFields environments={false} />
        <Form.Item name="run_mode" label="运行模式" initialValue="operator" extra={RUN_MODE_HINT}>
          <Segmented options={RUN_MODE_OPTIONS} aria-label="new-agent-run-mode" />
        </Form.Item>
        <Space>
          <Button type="primary" htmlType="submit" loading={saving}>创建</Button>
          <Button onClick={onClose}>取消</Button>
        </Space>
      </Form>
    </Modal>
  );
}

export function AgentsPanel({ onNotice }) {
  // 「节点总览」页签（原 Operator，admin-only 入口，产品选择）；后端允许
  // user/root 提交 operator 执行（见 users_api e2e），SPA 不在此重复拦截。
  const { identity } = useStore();
  return <PageShell page="agents"><Tabs destroyOnHidden items={[
    { key: 'agents', label: 'Agent 列表', children: <AgentListPanel onNotice={onNotice} /> },
    ...(identity?.role === 'admin' ? [
      { key: 'operator', label: '节点总览', children: <OperatorPanel onNotice={onNotice} /> },
    ] : []),
    { key: 'harnesses', label: 'Harness 管理', children: <HarnessManagement onNotice={onNotice} /> },
    { key: 'nfs', label: 'NFS 配置', children: <AgentNfsCard onNotice={onNotice} /> },
  ]} /></PageShell>;
}

function AgentListPanel({ onNotice }) {
  const msg = useMessage();
  const [agents, setAgents] = useState([]);
  const [search, setSearch] = useState('');
  const [dirty, setDirty] = useState(false);
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
      const [j, fleet] = await Promise.all([
        apiGet('/api/agents'),
        apiGet('/api/nodes'),
      ]);
      setAgents((j && j.agents) || []);
      setNodes((fleet && fleet.nodes) || []);
    } catch (e) {
      onNotice(err('获取 agent 列表失败: ' + (e && e.message)));
    } finally {
      setLoading(false);
    }
  }, [onNotice]);

  useEffect(() => {
    load();
  }, [load]);

  const remove = async (name) => {
    try {
      await apiDel(`/api/agents/${encodeURIComponent(name)}`);
      msg.success('已删除');
      load();
    } catch (e) {
      onNotice(err('删除 agent 失败: ' + (e && e.message)));
    }
  };

  const run = async (values) => {
    const request = { kind: 'agent', target: launch.name, node_id: values.node || null, input: { prompt: values.prompt, harness: values.harness || launch.harness || 'opencoder', envs: parseEnvs(values.envs) } };
    const signature = JSON.stringify(request);
    if (launchAttempt.current?.signature !== signature) launchAttempt.current = { signature, id: newId('agent') };
    setLaunching(true);
    try {
      const accepted = await apiPost('/api/executions', { ...request, id: launchAttempt.current.id });
      launchAttempt.current = null; setLaunch(null); setExecution(accepted);
    } catch (e) {
      onNotice(err(`${e.message}；再次启动会继续确认同一执行`));
    } finally { setLaunching(false); }
  };

  const columns = [
    {
      title: '名称',
      dataIndex: 'name',
      key: 'name',
      filters: agents.map((a) => ({ text: a.name, value: a.name })),
      filterSearch: true,
      onFilter: (v, r) => String(r.name).includes(v),
      render: (v) => (
        <Space size={4}>
          <Text strong>{v}</Text>
        </Space>
      ),
    },
    {
      title: '操作',
      key: 'ops',
      width: 200,
      render: (_, r) => (
        <Space size={0}>
          <Button size="small" type="link" onClick={() => setDetail(r.name)}>编辑</Button>
          <Button size="small" type="link" onClick={() => { launchForm.resetFields(); setLaunch(r); }}>启动</Button>
          <Popconfirm title={`删除 agent ${r.name}？`} okText="确认删除" onConfirm={() => remove(r.name)}>
            <Button size="small" type="link" danger>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  // 搜索框为受控组件：按名称（忽略大小写）过滤本地列表，不入服务端。
  const query = search.trim().toLowerCase();
  const visible = query
    ? agents.filter((a) => String(a.name).toLowerCase().includes(query))
    : agents;

  return (
    <div>
        <Space style={{ marginBottom: 16 }} wrap>
          <Input.Search
            allowClear
            style={{ minWidth: 220 }}
            placeholder="搜索 agent 名称"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            aria-label="agent-search"
          />
          <Button type="primary" onClick={() => setCreating(true)}>新建</Button>
        </Space>
      <Table
        rowKey="name"
        size="small"
        columns={columns}
        dataSource={visible}
        loading={loading}
        pagination={false}
        scroll={{ x: 'max-content' }}
        locale={{ emptyText: '暂无 agent' }}
      />
      <Drawer
        title={detail ? `编辑 Agent · ${detail}` : '编辑 Agent'}
        placement="right"
        open={!!detail}
        onClose={() => { if (!dirty || window.confirm('关闭会丢弃未保存内容，继续？')) { setDetail(''); setDirty(false); } }}
        size="75%"
        styles={{ wrapper: { maxWidth: '100vw' } }}
        destroyOnHidden
      >
        {detail ? <AgentDetail
          name={detail}
          onDirtyChange={setDirty}
          onNotice={onNotice}
          onChanged={load}
        /> : null}
      </Drawer>
      <CreateAgentModal
        open={creating}
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
          <HarnessFields initialHarness={launch?.harness || 'opencoder'} />
          <Form.Item name="prompt" label="任务要求" rules={[{ required: true, message: '请输入任务要求' }]}><Input.TextArea rows={5} /></Form.Item>
          <Button type="primary" htmlType="submit" loading={launching}>启动并查看</Button>
        </Form>
      </Modal>
      {execution && <ExecutionDetail id={execution.id} summary={execution} onClose={() => setExecution(null)} onNotice={onNotice} />}
    </div>
  );
}
