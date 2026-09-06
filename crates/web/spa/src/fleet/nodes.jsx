import { Button, Input, Modal, Select, Space, Table, Tag } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from '../api.js';
import { setState } from '../store.js';
import { ExecutionDetail } from './detail.jsx';
import { newId } from './model.js';

export function FleetNodesPanel({ onNotice }) {
  const [rows, setRows] = useState([]); const [selected, setSelected] = useState(null);
  const [action, setAction] = useState('status'); const [input, setInput] = useState('');
  const [result, setResult] = useState(null); const [busy, setBusy] = useState(false); const [detail, setDetail] = useState(null);
  const attempt = useRef(null);
  const load = useCallback(async () => {
    try { const j = await apiGet('/api/nodes'); setRows(j.nodes); setState({ nodes: j.nodes }); }
    catch (e) { onNotice(e.message); }
  }, [onNotice]);
  useEffect(() => { load(); const timer = setInterval(load, 3000); return () => clearInterval(timer); }, [load]);
  const perform = async () => {
    setBusy(true);
    try {
      let body = {};
      if (action === 'ask') {
        const signature = `${selected.id}:${input}`;
        if (attempt.current?.signature !== signature) attempt.current = { signature, id: newId('maintenance') };
        body = { id: attempt.current.id, prompt: input };
      } else if (action === 'configure') body = JSON.parse(input);
      const reply = await apiPost(`/api/nodes/${encodeURIComponent(selected.id)}/maintenance`, { action, input: body });
      onNotice(''); setResult(reply); if (action === 'ask') { setDetail(reply); attempt.current = null; } await load();
    } catch (e) { onNotice(e.message); }
    finally { setBusy(false); }
  };
  return <>
    <Button onClick={load} style={{ marginBottom: 12 }}>刷新节点</Button>
    <Table scroll={{ x: 'max-content' }} locale={{ emptyText: '暂无 Opencoder 节点' }} rowKey="id" dataSource={rows} columns={[
      { title: '节点', dataIndex: 'name', render: (v, r) => <Space orientation="vertical"><b>{v}</b><small>{r.id}</small></Space> },
      { title: '状态', render: (_, r) => <Tag color={r.online && r.snapshot?.ready ? 'green' : 'red'}>{r.online ? (r.snapshot?.resource_error || '在线') : '离线'}</Tag> },
      { title: '可用 CPU', render: (_, r) => r.snapshot?.cpu_capacity ?? '—' },
      { title: '活跃 agent loops', render: (_, r) => r.snapshot?.active_agent_loops ?? '—' },
      { title: 'loops / CPU', render: (_, r) => r.snapshot ? (r.snapshot.active_agent_loops / r.snapshot.cpu_capacity).toFixed(2) : '—' },
      { title: '维护 agent', dataIndex: 'maintenance_agent_id' },
      { title: '操作', render: (_, r) => <Button disabled={!r.online} onClick={() => { setSelected(r); setResult(null); }}>维护节点</Button> },
    ]} />
    <Modal open={!!selected} title={`节点维护 · ${selected?.name || ''}`} onCancel={() => setSelected(null)} footer={null} width={800}>
      <Space orientation="vertical" style={{ width: '100%' }}>
        <Select value={action} onChange={(v) => { setAction(v); setResult(null); }} style={{ width: 250 }} options={[
          ['status', '查询状态'], ['executions', '查询节点任务'], ['resources', '查询 Agent 资源'], ['config', '查询配置'], ['models', '查询模型'], ['skills', '查询技能'], ['ask', '向维护 agent 下达指令'], ['configure', '更新配置'],
        ].map(([value, label]) => ({ value, label }))} />
        {['ask', 'configure'].includes(action) && <Input.TextArea rows={5} value={input} onChange={(e) => setInput(e.target.value)} placeholder={action === 'ask' ? '明确描述要查询或修改的内容' : '配置更新 JSON'} />}
        <Button type="primary" loading={busy} onClick={perform}>{action === 'configure' ? '应用配置更新' : '执行指令'}</Button>
        {result && <pre style={{ whiteSpace: 'pre-wrap', maxHeight: 400, overflow: 'auto' }}>{JSON.stringify(result, null, 2)}</pre>}
      </Space>
    </Modal>
    {detail && <ExecutionDetail id={detail.id} summary={detail} onClose={() => setDetail(null)} onNotice={onNotice} />}
  </>;
}
