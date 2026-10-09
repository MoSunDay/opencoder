import { EditButton } from '../ui/permissions.jsx';
import { useCanEdit } from '../ui/permissions.jsx';
import { Alert, Button, ConfigProvider, Drawer, Input, Select, Space, Spin, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPatch, apiPost } from '../api.js';
import { ExecutionView } from '../fleet/detail.jsx';
import { KIND_LABELS, newId } from '../fleet/model.js';
import { err, ok } from '../notice.js';
import { flattenTodos, groupOptions, searchSelect } from './model/relations.js';
import { CapabilityLauncher } from './execute/launcher.jsx';
import { useCapabilities, capabilityOptions } from './execute/catalog.js';
import { ExecutionResult } from './execute/result.jsx';
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
  const canEdit = useCanEdit();
  const todo = flattenTodos(overview).find((item) => item.id === todoId);
  const [draft, setDraft] = useState(todo?.draft || '');
  const [title, setTitle] = useState(todo?.title || '');
  const [groupId, setGroupId] = useState(todo?.initiative_id || null);
  const [status, setStatus] = useState(todo?.board_status || 'backlog');
  const [tagIds, setTagIds] = useState(todo?.tag_ids || []);
  const availableTags = effectiveTags(overview, groupId);
  const selectedTags = remapTagIds(overview, tagIds, groupId);
  const [mode, setMode] = useState('overview');
  const catalog = useCapabilities();
  const [capabilityId, setCapabilityId] = useState(todo?.capability_id || null);
  const [linkInput, setLinkInput] = useState('');
  const [pendingId, setPendingId] = useState('');
  const [executionId, setExecutionId] = useState('');
  const [links, setLinks] = useState([]);
  const [indexes, setIndexes] = useState({});
  const [busy, setBusy] = useState(false);
  const guidanceAttempt = useRef(null);
  const navigation = useRef(0);
  const initiallyOpened = useRef(false);
  const pendingRead = useRef(null);
  const mounted = useRef(true);
  const [error, setError] = useState('');

  const navigate = (next) => { navigation.current += 1; setMode(next); };
  const loadLinks = useCallback(async (force = false) => {
    if (!mounted.current) return null;
    if (pendingRead.current && !force) return null;
    pendingRead.current?.abort();
    const request = new AbortController();
    pendingRead.current = request;
    try {
      const result = await apiGet(linkPath(todoId), { signal: request.signal });
      if (request.signal.aborted) return null;
      const assignments = result.assignments || [];
      setLinks(assignments);
      const found = await Promise.all(assignments.map(async (assignment) => {
        const id = assignment.execution_id;
        try { return [id, await apiGet(`/api/executions/${encodeURIComponent(id)}/index`, { signal: request.signal })]; }
        catch (failure) { return [id, { id, missing: true, readError: failure.message }]; }
      }));
      const resolved = Object.fromEntries(found);
      if (request.signal.aborted) return null;
      setIndexes(resolved);
      if (!initiallyOpened.current && navigation.current === 0) {
        initiallyOpened.current = true;
        const currentId = result.current_execution_id;
        if (currentId && resolved[currentId]?.kind) {
          setExecutionId(currentId); setMode('execution');
        } else if (todo?.capability_id && !currentId && canEdit) {
          setMode('launch');
        }
      }
      return resolved;
    } catch (failure) { if (!request.signal.aborted) setError(failure.message); return null; }
    finally { if (pendingRead.current === request) pendingRead.current = null; }
  }, [todoId, todo?.capability_id, canEdit]);
  useEffect(() => {
    mounted.current = true;
    loadLinks();
    const timer = setInterval(loadLinks, 5000);
    return () => {
      clearInterval(timer); pendingRead.current?.abort(); pendingRead.current = null;
      mounted.current = false; navigation.current += 1;
    };
  }, [loadLinks]);

  const save = async () => {
    navigation.current += 1;
    if (!title.trim()) { setError('TODO 标题不能为空'); return; }
    setBusy(true);
    try {
      await apiPatch(`/api/project/todos/${encodeURIComponent(todoId)}`, {
        title: title.trim(), draft, initiative_id: groupId, board_status: status, capability_id: capabilityId, tag_ids: selectedTags,
      });
      await refresh();
      setError('');
      onNotice(ok('TODO 已保存'));
      return true;
    } catch (failure) { setError(failure.message); return false; }
    finally { setBusy(false); }
  };
  const link = async (id, alreadyLinked = false) => {
    const execution = id.trim();
    if (!execution) return;
    const attempt = navigation.current;
    setBusy(true);
    try {
      if (!alreadyLinked) await apiPost(linkPath(todoId), { execution_id: execution });
      setPendingId((current) => current === execution ? '' : current);
      setLinkInput((current) => current.trim() === execution ? '' : current);
      const resolved = await loadLinks(true);
      if (navigation.current !== attempt) return;
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
      await loadLinks(true);
      setError('');
    } catch (failure) { setError(failure.message); }
    finally { setBusy(false); }
  };
  const columns = [
    { title: '类型', key: 'kind', width: '13%', kind: 'enum', searchValue: (r) => KIND_LABELS[r.kind] || r.kind || '不可读取', render: (_, row) => KIND_LABELS[row.kind] || row.kind || '不可读取' },
    { title: '名称', key: 'name', searchValue: (r) => r.name || r.id, render: (_, row) => row.name || row.id },
    { title: '执行 ID', key: 'id', width: '25%', searchValue: (r) => r.id, dataIndex: 'id', render: (id) => <Typography.Text copyable={{ text: id }}><TableText>{id}</TableText></Typography.Text> },
    { title: '状态', key: 'status', width: '18%', kind: 'enum', searchValue: (r) => r.status || r.readError || '等待读取', render: (_, row) => row.readError || row.status || '等待读取' },
    { title: '操作', key: 'actions', width: '17%', render: (_, row) => <Space>
      <Button type="link" disabled={!row.kind} onClick={() => { setExecutionId(row.id); navigate('execution'); }}>查看</Button>
      <EditButton danger type="link" disabled={!canEdit || busy} onClick={() => unlink(row.id)}>解除关联</EditButton>
    </Space> },
  ];
  return <Drawer open title={`TODO · ${todo?.title || todoId}`} onClose={onClose} placement="right" size="100vw" destroyOnHidden>
    <Space style={{ marginBottom: 16 }}>
      {mode !== 'overview' && <Button onClick={() => navigate('overview')}>返回 TODO</Button>}
      {mode === 'overview' && <EditButton type="primary" disabled={!canEdit || !capabilityId || catalog.loading || !!catalog.error} loading={busy} onClick={async () => { if (await save()) navigate('launch'); }}>指派所选能力</EditButton>}
    </Space>
    {error && <Alert type="error" showIcon title={error} style={{ marginBottom: 16 }} />}
    {pendingId && <Button loading={busy} onClick={() => link(pendingId)}>重试关联 {pendingId}</Button>}
    {mode === 'overview' && <Space orientation="vertical" size={16} style={{ width: '100%' }}>
      <ConfigProvider componentDisabled={!canEdit}>
      <Input aria-label="TODO 标题" value={title} onChange={(event) => setTitle(event.target.value)} />
      <Select {...searchSelect} aria-label="所属专项" placeholder="未归属专项" value={groupId} onChange={(value) => { setTagIds(remapTagIds(overview, tagIds, value || null)); setGroupId(value || null); }} options={groupOptions(overview)} style={{ width: '100%' }} />
      <Select mode="multiple" aria-label="TODO Tag" value={selectedTags} onChange={setTagIds} disabled={!groupId} showSearch optionFilterProp="label" placeholder={groupId ? "选择 Tag" : "关联专项后可选择 Tag"} options={availableTags.map((tag) => ({ value: tag.id, label: tag.name }))} style={{ width: '100%' }} />
      <Select aria-label="TODO 看板列" value={status} onChange={setStatus} options={STATUS_OPTIONS} style={{ width: 180 }} />
      {catalog.error && <Alert type="error" title={catalog.error} action={<Button onClick={catalog.reload}>重试能力列表</Button>} />}
      <Select aria-label="执行能力" placeholder="选择能力库中的能力" value={capabilityId} onChange={(value) => setCapabilityId(value || null)} allowClear loading={catalog.loading} options={capabilityOptions(catalog.capabilities)} style={{ width: '100%' }} showSearch optionFilterProp="label" />
      <Input.TextArea aria-label="任务说明" value={draft} onChange={(event) => setDraft(event.target.value)} rows={6} placeholder="写下任务要求，指派时会带入执行界面" />
      <EditButton type="primary" loading={busy} onClick={save}>保存 TODO</EditButton>
      </ConfigProvider>
      <Typography.Title level={5}>指派记录</Typography.Title>
      {links[0]?.execution_id && <ExecutionResult key={links[0].execution_id} id={links[0].execution_id} />}
      <Space.Compact style={{ width: '100%' }}>
        <Input aria-label="已有执行 ID" placeholder="粘贴已有执行 ID" value={linkInput} onChange={(event) => setLinkInput(event.target.value)} />
        <EditButton disabled={!canEdit || !linkInput.trim()} loading={busy} onClick={() => link(linkInput)}>关联</EditButton>
      </Space.Compact>
      <ProjectTable label="TODO 指派记录" viewKey={`assignments:${todoId}`} pagination={false} rows={links.map((record) => ({ ...indexes[record.execution_id], ...record, id: record.execution_id, kind: record.kind || indexes[record.execution_id]?.kind, name: record.name || indexes[record.execution_id]?.name, status: indexes[record.execution_id]?.status, missing: indexes[record.execution_id]?.missing, readError: indexes[record.execution_id]?.readError }))} columns={columns} />
    </Space>}
    {mode === 'launch' && catalog.loading && <Spin />}
    {mode === 'launch' && !catalog.loading && <CapabilityLauncher key={capabilityId} capability={catalog.capabilities.find((cap) => cap.id === capabilityId)} todoId={todoId} onNotice={onNotice}
      prompt={[title, draft].filter(Boolean).join('\n\n')} onCreated={(id) => link(id, true)} />}
    {mode === 'execution' && (indexes[executionId]
      ? <ExecutionView key={executionId} executionRef={{ id: executionId, kind: indexes[executionId].kind || links.find((link) => link.execution_id === executionId)?.kind }} summary={indexes[executionId]} onNotice={onNotice} mode={["agent", "operator"].includes(indexes[executionId].kind) ? "conversation" : "full"}
        allowGuidance={canEdit && indexes[executionId].kind === 'team' && indexes[executionId].status === 'running'}
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
