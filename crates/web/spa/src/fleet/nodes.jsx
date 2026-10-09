import { Button, Input, Modal, Select, Space, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPost } from '../api.js';
import { setState } from '../store.js';
import { PageShell } from '../shell/pageShell.jsx';
import { StatusTag } from '../ui/statusTag.jsx';
import { MONO_VAR } from '../ui/mono.js';
import { tableLoading, tableRows } from '../ui/tableLoading.js';
import { ExecutionDetail } from './detail.jsx';
import { newId } from './model.js';
import { err, ok } from '../notice.js';
import { NodeSchedulingModal } from './settings/scheduling.jsx';
import { ReleasesModal } from './settings/releases.jsx';
import { AdminHint, AdminTable, AdminToolbar } from '../admin/layout.jsx';
import { matchesSearch } from '../admin/model.js';

export function FleetNodesPanel({ onNotice }) {
  const [rows, setRows] = useState([]); const [selected, setSelected] = useState(null);
  const [releases, setReleases] = useState(false);
  const [scheduling, setScheduling] = useState(null);
  const [action, setAction] = useState('status'); const [input, setInput] = useState('');
  const [result, setResult] = useState(null); const [busy, setBusy] = useState(false); const [detail, setDetail] = useState(null);
  const [deleting, setDeleting] = useState(false);
  const [removeTarget, setRemoveTarget] = useState(null);
  const [search, setSearch] = useState('');
  const [status, setStatus] = useState();
  /// 列表拉取态：silent = 3s 轮询（静默，不闪 spinner），非 silent = 首屏 / 手动刷新。
  const [loading, setLoading] = useState(true);
  const attempt = useRef(null);
  const load = useCallback(async (silent) => {
    if (!silent) setLoading(true);
    try { const j = await apiGet('/api/nodes'); setRows(j.nodes); setState({ nodes: j.nodes }); }
    catch (e) { onNotice(err(e.message)); }
    finally { if (!silent) setLoading(false); }
  }, [onNotice]);
  useEffect(() => { load(false); const timer = setInterval(() => load(true), 3000); return () => clearInterval(timer); }, [load]);
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
      onNotice(err('')); setResult(reply); if (action === 'ask') { setDetail(reply); attempt.current = null; } await load();
    } catch (e) { onNotice(err(e.message)); }
    finally { setBusy(false); }
  };
  const removeNode = async () => {
    setDeleting(true);
    try {
      await apiDel(`/api/nodes/${encodeURIComponent(removeTarget.id)}`);
      setRemoveTarget(null);
      onNotice(ok('节点注册已删除，任务记录保留'));
      await load(false);
    } catch (e) { onNotice(err(e.message)); }
    finally { setDeleting(false); }
  };
  return <PageShell page="nodes">
    <AdminToolbar search={search} onSearch={setSearch} placeholder="搜索节点名称或 ID" loading={loading}
      onRefresh={() => load(false)} refreshLabel="刷新节点"
      filters={<Select aria-label="筛选节点状态" placeholder="全部状态" allowClear value={status} onChange={setStatus}
        options={[{ value: 'online', label: '在线' }, { value: 'offline', label: '离线' }]} />}
      actions={<Button onClick={() => setReleases(true)}>发布状态</Button>} />
    <AdminTable scroll={{ x: 1250 }} locale={{ emptyText: search || status ? '没有匹配的节点' : '暂无 Opencoder 节点' }} rowKey="id"
      dataSource={tableRows(loading, rows.filter((row) => (!status || (row.online ? 'online' : 'offline') === status) && matchesSearch(search, row.name, row.id)))} loading={tableLoading(loading)} columns={[
      { title: '节点', dataIndex: 'name', width: 270, ellipsis: true, render: (v, r) => <>
        <Typography.Text strong>{v}</Typography.Text>
        <span className="oc-admin-cell-secondary" style={{ fontFamily: MONO_VAR }} title={r.id}>{r.id}</span>
        <span className="oc-admin-cell-secondary" title={`维护 Agent：${r.maintenance_agent_id || '—'}`}>维护 Agent：<span>{r.maintenance_agent_id || '—'}</span></span>
      </> },
      { title: '状态', width: 130, ellipsis: true, render: (_, r) => <StatusTag status={r.online ? 'online' : 'offline'} label={r.online ? (r.snapshot?.resource_error || '在线') : '离线'} color={r.online && r.snapshot?.ready ? 'success' : 'error'} /> },
      { title: '可用 CPU', width: 100, align: 'right', render: (_, r) => r.snapshot?.cpu_capacity ?? '—' },
      { title: '运行 / 最大并发', width: 150, align: 'right', render: (_, r) => r.snapshot ? `${r.snapshot.active_runs ?? '—'} / ${r.snapshot.max_runs ?? '—'}` : '—' },
      { title: '排队任务', width: 140, render: (_, r) => <>{r.snapshot?.pending_runs ?? '—'}<span className="oc-admin-cell-secondary">{r.snapshot ? (r.snapshot.queue_order === 'lifo' ? '后入先出 LIFO' : '先入先出 FIFO') : '—'}</span></> },
      { title: '活跃 agent loops', width: 170, align: 'right', render: (_, r) => <>{r.snapshot?.active_agent_loops ?? '—'}<span className="oc-admin-cell-secondary">loops / CPU：<span>{r.snapshot?.cpu_capacity > 0 && r.snapshot.active_agent_loops != null ? (r.snapshot.active_agent_loops / r.snapshot.cpu_capacity).toFixed(2) : '—'}</span></span></> },
      { title: '操作', key: 'actions', width: 290, fixed: 'right', render: (_, r) => <Space size={8}><Button type="link" disabled={!r.online} onClick={() => setScheduling(r)}>调度配置</Button><Button type="link" disabled={!r.online} onClick={() => { setSelected(r); setResult(null); }}>维护节点</Button><Button type="link" danger disabled={r.online} title={r.online ? '请先停止节点服务，离线后可删除注册' : undefined} onClick={() => setRemoveTarget(r)}>删除节点</Button></Space> },
    ]} />
    <AdminHint>节点状态每 3 秒自动更新。选择节点行中的按钮查看调度配置或打开维护对话框。</AdminHint>
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
    <Modal open={!!removeTarget} title={`删除节点 · ${removeTarget?.name || ''}`} okText="确认删除" cancelText="取消"
      okButtonProps={{ danger: true }} confirmLoading={deleting} onOk={removeNode}
      onCancel={() => { if (!deleting) setRemoveTarget(null); }}>
      只删除节点注册信息，任务记录和执行文件保留。节点服务重新连接后会再次注册。
    </Modal>
    {detail && <ExecutionDetail id={detail.id} summary={detail} onClose={() => setDetail(null)} onNotice={onNotice} />}
    {scheduling && <NodeSchedulingModal node={scheduling} onClose={() => setScheduling(null)} onSaved={() => load(false)} onNotice={onNotice} />}
    {releases && <ReleasesModal onClose={() => setReleases(false)} onInspect={(id) => setDetail({ id })} />}
  </PageShell>;
}
