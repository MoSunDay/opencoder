import { Button, Form, Input, Select, Space, Table } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from '../api.js';
import { PageShell } from '../shell/pageShell.jsx';
import { StatusTag } from '../ui/statusTag.jsx';
import { TimeText } from '../ui/timeText.jsx';
import { ExecutionDetail } from './detail.jsx';
import { CREATABLE_KINDS, KIND_LABELS, KINDS, executionPagePath, newId, nodeOptions } from './model.js';

export function ExecutionsPanel({ onNotice }) {
  const [rows, setRows] = useState([]); const [nodes, setNodes] = useState([]);
  const [kind, setKind] = useState('agent'); const [detail, setDetail] = useState(null);
  const [busy, setBusy] = useState(false); const [filter, setFilter] = useState('');
  const [more, setMore] = useState(false); const [loadingMore, setLoadingMore] = useState(false);
  const attempt = useRef(null); const cursor = useRef(null); const extended = useRef(false); const [form] = Form.useForm();
  const load = useCallback(async (mode = 'reset') => {
    const append = mode === 'append';
    if (append) setLoadingMore(true);
    try {
      const [a, b] = await Promise.all([apiGet(executionPagePath(filter, append ? cursor.current : null)), apiGet('/api/nodes')]);
      const page = a.executions || [];
      if (append) {
        setRows((old) => [...old, ...page.filter((row) => !old.some((item) => item.id === row.id))]);
        extended.current = true; cursor.current = a.next_cursor || null; setMore(!!a.next_cursor);
      } else if (mode === 'poll' && extended.current) {
        setRows((old) => [...page, ...old.filter((row) => !page.some((fresh) => fresh.id === row.id))]);
      } else {
        setRows(page); extended.current = false; cursor.current = a.next_cursor || null; setMore(!!a.next_cursor);
      }
      setNodes(b.nodes || []);
    }
    catch (e) { onNotice(e.message); }
    finally { if (append) setLoadingMore(false); }
  }, [filter, onNotice]);
  useEffect(() => {
    let live = true;
    const refresh = () => { if (live) load('poll'); };
    load('reset'); const timer = setInterval(refresh, 3000);
    return () => { live = false; clearInterval(timer); };
  }, [filter, load]);
  const submit = async (values) => {
    const input = { prompt: values.prompt || '' };
    if (kind === 'project') input.action = 'plan';
    const request = { kind, target: values.target || null, node_id: values.node || null, input };
    const signature = JSON.stringify(request);
    if (!attempt.current || attempt.current.signature !== signature) attempt.current = { signature, id: kind === 'project' ? `project-${values.target}` : newId(kind) };
    setBusy(true);
    try { const result = await apiPost('/api/executions', { ...request, id: attempt.current.id }); onNotice(''); attempt.current = null; setDetail(result); await load('reset'); }
    catch (e) { onNotice(`${e.message}；保持内容不变再次启动，会继续确认同一次执行`); }
    finally { setBusy(false); }
  };
  return <PageShell page="topics">
    <Form form={form} layout="vertical" onFinish={submit} initialValues={{ node: '' }}>
      <Space align="start" wrap>
        <Form.Item label="执行类型"><Select style={{ width: 150 }} value={kind} onChange={setKind} options={CREATABLE_KINDS} /></Form.Item>
        <Form.Item name="target" label="Agent / 团队 / 工作流定义 / 项目任务" rules={[{ required: true }]}><Input style={{ width: 300 }} placeholder={kind === 'todos' ? '模板名/v1' : 'act / 定义名称 / 任务 ID'} /></Form.Item>
        <Form.Item name="node" label="调度节点"><Select style={{ width: 330 }} options={nodeOptions(nodes, kind)} /></Form.Item>
      </Space>
      <Form.Item name="prompt" label="任务要求"><Input.TextArea rows={3} /></Form.Item>
      <Button type="primary" htmlType="submit" loading={busy}>启动执行</Button>
    </Form>
    <Space style={{ margin: '20px 0 12px' }}><Select aria-label="执行类型筛选" style={{ width: 180 }} value={filter} onChange={setFilter} options={[{ value: '', label: '全部执行' }, ...KINDS]} /><Button onClick={() => load('reset')}>刷新</Button></Space>
    <Table scroll={{ x: 'max-content' }} rowKey="id" dataSource={rows} size="small" columns={[
      { title: 'ID', dataIndex: 'id', render: (id, row) => <Button type="link" style={{ fontFamily: 'var(--oc-mono, monospace)' }} onClick={() => setDetail(row)}>{id}</Button> },
      { title: '类型', dataIndex: 'kind', render: (v) => KIND_LABELS[v] || v },
      { title: '创建时间', dataIndex: 'created_at', render: (v) => <TimeText ts={v} /> },
      { title: '所属节点', dataIndex: 'node_id', render: (id) => <Space size={4}><span style={{ fontFamily: 'var(--oc-mono, monospace)' }}>{id}</span><StatusTag status={nodes.find((node) => node.id === id)?.online ? 'online' : 'offline'} /></Space> },
      { title: '状态', dataIndex: 'status', render: (v) => <StatusTag status={v} /> },
    ]} />
    {more && <Button block loading={loadingMore} onClick={() => load('append')}>加载更早的执行</Button>}
    {detail && <ExecutionDetail id={detail.id} summary={detail} onClose={() => setDetail(null)} onNotice={onNotice} />}
  </PageShell>;
}
