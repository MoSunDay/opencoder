import { Alert, Button, Drawer, Input, Select, Space, Spin, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPatch, apiPost } from '../api.js';
import { ExecutionView } from '../fleet/detail.jsx';
import { KIND_LABELS, newId } from '../fleet/model.js';
import { err, ok } from '../notice.js';
import { flattenTodos, groupOptions, searchSelect } from './model/relations.js';
import { CapabilityLauncher, CAPABILITIES } from './execute/launcher.jsx';
import { Markdown } from './markdown.jsx';
import { effectiveTags, remapTagIds } from './model/catalog.js';
import { ProjectTable, TableText } from './views/projectTable.jsx';

const STATUS_OPTIONS = [
  { value: 'backlog', label: '待整理' },
  { value: 'todo', label: '待办' },
  { value: 'in_progress', label: '进行中' },
  { value: 'done', label: '已完成' },
];
const linkPath = (todoId) => `/api/project/todos/${encodeURIComponent(todoId)}/executions`;

function TodoDrawerSession({ todoId, overview, refresh, onClose, onNotice }) {
  const todo = flattenTodos(overview).find((item) => item.id === todoId);
  const [draft, setDraft] = useState(todo?.draft || '');
  const [title, setTitle] = useState(todo?.title || '');
  const [groupId, setGroupId] = useState(todo?.initiative_id || null);
  const [status, setStatus] = useState(todo?.board_status || 'backlog');
  const [tagIds, setTagIds] = useState(todo?.tag_ids || []);
  const availableTags = effectiveTags(overview, groupId);
  const selectedTags = remapTagIds(overview, tagIds, groupId);
  const [mode, setMode] = useState('overview');
  const [kind, setKind] = useState(todo?.capability_id || null);
  const [linkInput, setLinkInput] = useState('');
  const [pendingId, setPendingId] = useState('');
  const [executionId, setExecutionId] = useState('');
  const [links, setLinks] = useState([]);
  const [indexes, setIndexes] = useState({});
  const [busy, setBusy] = useState(false);
  const guidanceAttempt = useRef(null);
  const [error, setError] = useState('');

  const loadLinks = useCallback(async () => {
    try {
      const result = await apiGet(linkPath(todoId));
      const assignments = result.assignments || (result.execution_ids || []).map((execution_id) => ({ execution_id, kind: '', name: '', sync_state: 'pending' }));
      setLinks(assignments);
      const found = await Promise.all(assignments.map(async (assignment) => {
        const id = assignment.execution_id;
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
        title: title.trim(), draft, initiative_id: groupId, board_status: status, capability_id: kind, tag_ids: selectedTags,
      });
      await refresh();
      setError('');
      onNotice(ok('TODO 已保存'));
      return true;
    } catch (failure) { setError(failure.message); return false; }
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
    { title: '类型', key: 'kind', width: '13%', kind: 'enum', searchValue: (r) => KIND_LABELS[r.kind] || r.kind || '不可读取', render: (_, row) => KIND_LABELS[row.kind] || row.kind || '不可读取' },
    { title: '名称', key: 'name', searchValue: (r) => r.name || r.id, render: (_, row) => row.name || row.id },
    { title: '执行 ID', key: 'id', width: '25%', searchValue: (r) => r.id, dataIndex: 'id', render: (id) => <Typography.Text copyable={{ text: id }}><TableText>{id}</TableText></Typography.Text> },
    { title: '状态', key: 'status', width: '18%', kind: 'enum', searchValue: (r) => r.result_md ? '结论已回写' : ({ error: '失败', cancelled: '已取消', empty: '已结束，无结论' }[r.sync_state] || r.status || '等待执行'), render: (_, row) => row.result_md ? '结论已回写' : ({ error: '失败', cancelled: '已取消', empty: '已结束，无结论' }[row.sync_state] || row.status || '等待执行') },
    { title: '操作', key: 'actions', width: '17%', render: (_, row) => <Space>
      <Button type="link" disabled={!row.kind || row.missing} onClick={() => { setExecutionId(row.id); setMode('execution'); }}>查看</Button>
      <Button danger type="link" disabled={busy} onClick={() => unlink(row.id)}>解除关联</Button>
    </Space> },
  ];
  return <Drawer open title={`TODO · ${todo?.title || todoId}`} onClose={onClose} placement="right" size="100vw" styles={{ wrapper: { maxWidth: 1000 } }} destroyOnHidden>
    <Space style={{ marginBottom: 16 }}>
      {mode !== 'overview' && <Button onClick={() => setMode('overview')}>返回 TODO</Button>}
      {mode === 'overview' && <Button type="primary" disabled={!kind} loading={busy} onClick={async () => { if (await save()) setMode('launch'); }}>指派所选能力</Button>}
    </Space>
    {error && <Alert type="error" showIcon title={error} style={{ marginBottom: 16 }} />}
    {pendingId && <Button loading={busy} onClick={() => link(pendingId)}>重试关联 {pendingId}</Button>}
    {mode === 'overview' && <Space orientation="vertical" size={16} style={{ width: '100%' }}>
      <Input aria-label="TODO 标题" value={title} onChange={(event) => setTitle(event.target.value)} />
      <Select {...searchSelect} aria-label="所属专项" placeholder="未归属专项" value={groupId} onChange={(value) => { setTagIds(remapTagIds(overview, tagIds, value || null)); setGroupId(value || null); }} options={groupOptions(overview)} style={{ width: '100%' }} />
      <Select mode="multiple" aria-label="TODO Tag" value={selectedTags} onChange={setTagIds} disabled={!groupId} showSearch optionFilterProp="label" placeholder={groupId ? "选择 Tag" : "关联专项后可选择 Tag"} options={availableTags.map((tag) => ({ value: tag.id, label: tag.name }))} style={{ width: '100%' }} />
      <Select aria-label="TODO 看板列" value={status} onChange={setStatus} options={STATUS_OPTIONS} style={{ width: 180 }} />
      <Select aria-label="执行能力" placeholder="选择能力后可指派" value={kind} onChange={(value) => setKind(value || null)} allowClear options={CAPABILITIES} style={{ width: 210 }} />
      <Input.TextArea aria-label="任务说明" value={draft} onChange={(event) => setDraft(event.target.value)} rows={6} placeholder="写下任务要求，指派时会带入执行界面" />
      <Button type="primary" loading={busy} onClick={save}>保存 TODO</Button>
      <Typography.Title level={5}>指派记录</Typography.Title>
      {links[0]?.result_md && <div><Typography.Text strong>最新结论</Typography.Text><Markdown text={links[0].result_md} /></div>}
      <Space.Compact style={{ width: '100%' }}>
        <Input aria-label="已有执行 ID" placeholder="粘贴已有执行 ID" value={linkInput} onChange={(event) => setLinkInput(event.target.value)} />
        <Button disabled={!linkInput.trim()} loading={busy} onClick={() => link(linkInput)}>关联</Button>
      </Space.Compact>
      <ProjectTable label="TODO 指派记录" viewKey={`assignments:${todoId}`} pagination={false} rows={links.map((record) => ({ ...indexes[record.execution_id], ...record, id: record.execution_id, kind: record.kind || indexes[record.execution_id]?.kind, name: record.name || indexes[record.execution_id]?.name, status: indexes[record.execution_id]?.status, missing: indexes[record.execution_id]?.missing }))} columns={columns} />
    </Space>}
    {mode === 'launch' && <CapabilityLauncher key={kind} kind={kind} onNotice={onNotice}
      prompt={[title, draft].filter(Boolean).join('\n\n')} onCreated={(id) => link(id, true)} />}
    {mode === 'execution' && (indexes[executionId]
      ? <ExecutionView key={executionId} executionRef={{ id: executionId, kind: indexes[executionId].kind }} summary={indexes[executionId]} onNotice={onNotice}
        allowGuidance={indexes[executionId].kind === 'team' && indexes[executionId].status === 'running'}
        onGuidance={async (prompt) => {
          try {
            const signature = `${executionId}:${prompt}`;
            if (guidanceAttempt.current?.signature !== signature) guidanceAttempt.current = { signature, id: newId('input') };
            await apiPost(`/api/executions/${encodeURIComponent(executionId)}/commands`, { action: 'steer', input: { prompt, input_id: guidanceAttempt.current.id } });
            guidanceAttempt.current = null;
            onNotice(ok('引导已提交'));
            return true;
          } catch (failure) { onNotice(err(failure.message)); return false; }
        }} />
      : <Spin />)}
  </Drawer>;
}

export function TodoDrawer({ todoId, overview, refresh, onClose, onNotice }) {
  return todoId ? <TodoDrawerSession key={todoId} {...{ todoId, overview, refresh, onClose, onNotice }} /> : null;
}
