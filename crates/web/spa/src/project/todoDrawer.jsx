import { Alert, Button, Drawer, Input, Select, Space, Spin, Table, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPatch, apiPost } from '../api.js';
import { ExecutionView } from '../fleet/detail.jsx';
import { err, ok } from '../notice.js';
import { flattenTodos, groupOptions, searchSelect } from './model/relations.js';
import { CapabilityLauncher, CAPABILITIES } from './execute/launcher.jsx';

const { TextArea } = Input;
const STATUS_OPTIONS = [
  { value: 'draft', label: '待处理' },
  { value: 'planned', label: '已规划' },
  { value: 'done', label: '已完成' },
];
const linkPath = (todoId) => `/api/project/todos/${encodeURIComponent(todoId)}/executions`;

function TodoDrawerSession({ todoId, overview, refresh, onClose, onNotice }) {
  const todo = flattenTodos(overview).find((item) => item.id === todoId);
  const [draft, setDraft] = useState(todo?.draft || '');
  const [title, setTitle] = useState(todo?.title || '');
  const [groupId, setGroupId] = useState(todo?.milestone_id || null);
  const [status, setStatus] = useState(todo?.status || 'draft');
  const [mode, setMode] = useState('overview');
  const [kind, setKind] = useState('agent');
  const [linkInput, setLinkInput] = useState('');
  const [pendingId, setPendingId] = useState('');
  const [executionId, setExecutionId] = useState('');
  const [links, setLinks] = useState([]);
  const [indexes, setIndexes] = useState({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  const loadLinks = useCallback(async () => {
    try {
      const result = await apiGet(linkPath(todoId));
      const ids = result.execution_ids || [];
      setLinks(ids);
      const found = await Promise.all(ids.map(async (id) => {
        try { return [id, await apiGet(`/api/executions/${encodeURIComponent(id)}/index`)]; }
        catch { return [id, { id, missing: true }]; }
      }));
      const resolved = Object.fromEntries(found);
      setIndexes(resolved);
      return resolved;
    } catch (failure) { setError(failure.message); return null; }
  }, [todoId]);
  useEffect(() => {
    loadLinks();
    const timer = setInterval(loadLinks, 5000);
    return () => clearInterval(timer);
  }, [loadLinks]);

  const save = async () => {
    if (!title.trim()) { setError('TODO 标题不能为空'); return; }
    setBusy(true);
    try {
      await apiPatch(`/api/project/todos/${encodeURIComponent(todoId)}`, {
        title: title.trim(), draft, milestone_id: groupId, status,
      });
      await refresh();
      setError('');
      onNotice(ok('TODO 已保存'));
    } catch (failure) { setError(failure.message); }
    finally { setBusy(false); }
  };
  const link = async (id, retryIndex = false) => {
    const execution = id.trim();
    if (!execution) return;
    setBusy(true);
    try {
      for (let attempt = 0; ; attempt += 1) {
        try { await apiPost(linkPath(todoId), { execution_id: execution }); break; }
        catch (failure) {
          if (!retryIndex || failure.status !== 404 || attempt >= 7) throw failure;
          await new Promise((resolve) => setTimeout(resolve, 500));
        }
      }
      setPendingId('');
      setLinkInput('');
      const resolved = await loadLinks();
      if (resolved?.[execution]?.kind) {
        setError('');
        setExecutionId(execution);
        setMode('execution');
      } else {
        setMode('overview');
        onNotice(ok('执行已关联；索引可读取后可查看记录'));
      }
    } catch (failure) {
      setPendingId(execution);
      setError(`关联失败：${failure.message}。执行 ID ${execution} 已保留，可重试。`);
    } finally { setBusy(false); }
  };
  const unlink = async (id) => {
    setBusy(true);
    try {
      await apiDel(`${linkPath(todoId)}/${encodeURIComponent(id)}`);
      await loadLinks();
      setError('');
    } catch (failure) { setError(failure.message); }
    finally { setBusy(false); }
  };
  const columns = [
    { title: '类型', render: (_, row) => CAPABILITIES.find((item) => item.value === row.kind)?.label || row.kind || '不可读取' },
    { title: '名称', render: (_, row) => row.name || row.id },
    { title: '执行 ID', dataIndex: 'id', render: (id) => <Typography.Text copyable={{ text: id }}>{id}</Typography.Text> },
    { title: '状态', dataIndex: 'status', render: (value) => value || '—' },
    { title: '操作', render: (_, row) => <Space>
      <Button type="link" disabled={!row.kind} onClick={() => { setExecutionId(row.id); setMode('execution'); }}>查看</Button>
      <Button danger type="link" disabled={busy} onClick={() => unlink(row.id)}>解除关联</Button>
    </Space> },
  ];
  return <Drawer open title={`TODO · ${todo?.title || todoId}`} onClose={onClose} size="min(100vw, 1000px)" destroyOnHidden>
    <Space style={{ marginBottom: 16 }}>
      {mode !== 'overview' && <Button onClick={() => setMode('overview')}>返回 TODO</Button>}
      {mode !== 'launch' && <Button type="primary" onClick={() => setMode('launch')}>从能力发起执行</Button>}
    </Space>
    {error && <Alert type="error" showIcon title={error} style={{ marginBottom: 16 }} />}
    {pendingId && <Button loading={busy} onClick={() => link(pendingId)}>重试关联 {pendingId}</Button>}
    {mode === 'overview' && <Space orientation="vertical" size={16} style={{ width: '100%' }}>
      <Input aria-label="TODO 标题" value={title} onChange={(event) => setTitle(event.target.value)} />
      <Select {...searchSelect} aria-label="所属里程碑或专项" placeholder="未分组" value={groupId} onChange={(value) => setGroupId(value || null)} options={groupOptions(overview)} style={{ width: 320 }} />
      <Select aria-label="TODO 状态" value={status} onChange={setStatus} options={STATUS_OPTIONS} style={{ width: 180 }} />
      <TextArea aria-label="任务说明" value={draft} onChange={(event) => setDraft(event.target.value)} rows={6} />
      <Button type="primary" loading={busy} onClick={save}>保存 TODO</Button>
      <Typography.Title level={5}>关联执行</Typography.Title>
      <Space.Compact style={{ width: '100%' }}>
        <Input aria-label="已有执行 ID" placeholder="粘贴已有执行 ID" value={linkInput} onChange={(event) => setLinkInput(event.target.value)} />
        <Button disabled={!linkInput.trim()} loading={busy} onClick={() => link(linkInput)}>关联</Button>
      </Space.Compact>
      <Table rowKey="id" size="small" pagination={false} dataSource={links.map((id) => indexes[id] || { id })} columns={columns} scroll={{ x: 'max-content' }} />
    </Space>}
    {mode === 'launch' && <CapabilityLauncher key={kind} kind={kind} onKind={setKind} onNotice={onNotice}
      prompt={[title, draft].filter(Boolean).join('\n\n')} onCreated={(id) => link(id, true)} />}
    {mode === 'execution' && (indexes[executionId]
      ? <ExecutionView key={executionId} executionRef={{ id: executionId, kind: indexes[executionId].kind }} summary={indexes[executionId]} onNotice={onNotice} />
      : <Spin />)}
  </Drawer>;
}

export function TodoDrawer({ todoId, overview, refresh, onClose, onNotice }) {
  return todoId ? <TodoDrawerSession key={todoId} {...{ todoId, overview, refresh, onClose, onNotice }} /> : null;
}
