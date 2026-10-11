import { EditButton } from '../ui/permissions.jsx';
import { useCanEdit } from '../ui/permissions.jsx';
import { Alert, Button, Drawer, Form, Input, Space, Spin, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPatch, apiPost } from '../api.js';
import { ExecutionView } from '../fleet/detail.jsx';
import { KIND_LABELS, newId } from '../fleet/model.js';
import { err, ok } from '../notice.js';
import { flattenTodos } from './model/relations.js';
import { CapabilityLauncher } from './execute/launcher.jsx';
import { useCapabilities } from './execute/catalog.js';
import { ExecutionResult } from './execute/result.jsx';
import { remapTagIds } from './model/catalog.js';
import { ProjectTable, TableText } from './views/projectTable.jsx';
import { TodoEditor } from './forms/editor.jsx';
import { TodoActions, TodoSection } from './forms/section.jsx';

const linkPath = (todoId) => `/api/project/todos/${encodeURIComponent(todoId)}/executions`;

function TodoDrawerSession({ todoId, todo, overview, refresh, onClose, onNotice }) {
  const canEdit = useCanEdit();
  const [draft, setDraft] = useState(todo?.draft || '');
  const [title, setTitle] = useState(todo?.title || '');
  const [groupId, setGroupId] = useState(todo?.initiative_id || null);
  const [status, setStatus] = useState(todo?.board_status || 'backlog');
  const [tagIds, setTagIds] = useState(todo?.tag_ids || []);
  const [owner, setOwner] = useState(todo?.owner_user_name ?? null);
  const [followers, setFollowers] = useState(todo?.follower_user_names || []);
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
    if (!canEdit || busy) return false;
    navigation.current += 1;
    if (!title.trim()) { setError('TODO 标题不能为空'); return; }
    setBusy(true);
    try {
      await apiPatch(`/api/project/todos/${encodeURIComponent(todoId)}`, {
        title: title.trim(), draft, initiative_id: groupId, board_status: status, capability_id: capabilityId, tag_ids: selectedTags,
        owner_user_name: owner, follower_user_names: followers,
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
  const changeField = (key, value) => {
    if (key === 'title' && value.trim() && error === 'TODO 标题不能为空') setError('');
    if (key === 'groupId') {
      setTagIds(remapTagIds(overview, tagIds, value)); setGroupId(value);
    } else ({ title: setTitle, draft: setDraft, status: setStatus, tagIds: setTagIds,
      owner: setOwner, followers: setFollowers, capabilityId: setCapabilityId })[key](value);
  };
  return <Drawer open title={`TODO · ${todo?.title || todoId}`} onClose={onClose} placement="right" size="100vw" destroyOnHidden
    footer={mode === 'overview' && <TodoActions>
      {error && error !== 'TODO 标题不能为空' && <Alert type="error" showIcon title={error} />}
      <EditButton type="primary" loading={busy} onClick={save}>保存 TODO</EditButton>
      <EditButton disabled={!canEdit || !capabilityId || catalog.loading || !!catalog.error} loading={busy} onClick={async () => { if (await save()) navigate('launch'); }}>指派所选能力</EditButton>
    </TodoActions>}>
    {mode !== 'overview' && <Button style={{ marginBottom: 16 }} onClick={() => navigate('overview')}>返回 TODO</Button>}
    {mode !== 'overview' && error && <Alert type="error" showIcon title={error} style={{ marginBottom: 16 }} />}
    {pendingId && <Button className="project-todo-retry" loading={busy} onClick={() => link(pendingId)}>重试关联 {pendingId}</Button>}
    {mode === 'overview' && <div className="project-todo-workspace">
      <TodoEditor value={{ title, draft, groupId, status, tagIds: selectedTags, owner, followers, capabilityId }}
        onChange={changeField} overview={overview} catalog={catalog} disabled={!canEdit || busy} onSave={save} titleError={error === 'TODO 标题不能为空' ? error : ''} />
      <TodoSection title="指派记录">
        {links[0]?.execution_id && <div className="project-todo-field-full"><ExecutionResult key={links[0].execution_id} id={links[0].execution_id} /></div>}
        <Form layout="vertical" component="div" className="project-todo-field-full">
          <Form.Item label="已有执行 ID" htmlFor={`todo-link-${todoId}`}>
            <div className="project-todo-link">
              <Input id={`todo-link-${todoId}`} aria-label="已有执行 ID" placeholder="粘贴已有执行 ID" value={linkInput} onChange={(event) => setLinkInput(event.target.value)} disabled={!canEdit || busy} />
              <EditButton disabled={!canEdit || !linkInput.trim()} loading={busy} onClick={() => link(linkInput)}>关联</EditButton>
            </div>
          </Form.Item>
        </Form>
        <div className="project-todo-field-full">
          <ProjectTable scroll={{ x: 900 }} label="TODO 指派记录" viewKey={`assignments:${todoId}`} pagination={false} rows={links.map((record) => ({ ...indexes[record.execution_id], ...record, id: record.execution_id, kind: record.kind || indexes[record.execution_id]?.kind, name: record.name || indexes[record.execution_id]?.name, status: indexes[record.execution_id]?.status, missing: indexes[record.execution_id]?.missing, readError: indexes[record.execution_id]?.readError }))} columns={columns} />
        </div>
      </TodoSection>
    </div>}
    {mode === 'launch' && catalog.loading && <Spin />}
    {mode === 'launch' && !catalog.loading && <div className="project-todo-workspace"><CapabilityLauncher key={capabilityId} capability={catalog.capabilities.find((cap) => cap.id === capabilityId)} todoId={todoId} onNotice={onNotice}
      prompt={[title, draft].filter(Boolean).join('\n\n')} onCreated={(id) => link(id, true)} /></div>}
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
  if (!todoId) return null;
  const todo = flattenTodos(overview).find((item) => item.id === todoId);
  if (!todo) return <Drawer open title="TODO" onClose={onClose} placement="right" size="100vw"><Spin description="正在读取 TODO"><div style={{ minHeight: 80 }} /></Spin></Drawer>;
  return <TodoDrawerSession key={todoId} {...{ todoId, todo, overview, refresh, onClose, onNotice }} />;
}
