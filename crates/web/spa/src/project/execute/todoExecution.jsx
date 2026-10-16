import { Alert, Button, Form, Popconfirm, Select, Space, Spin, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPost } from '../../api.js';
import { EditButton, useCanEdit } from '../../ui/permissions.jsx';
import { ExecutionView } from '../../fleet/detail.jsx';
import { KIND_LABELS, newId } from '../../fleet/model.js';
import { executionHref } from '../../fleet/navigation/links.js';
import { err, ok } from '../../notice.js';
import { ProjectTable } from '../views/projectTable.jsx';
import { capabilityOptions, useCapabilities } from './catalog.js';
import { CapabilityLauncher } from './launcher.jsx';
import { ExecutionResult } from './result.jsx';

export function TodoExecution({ todoId, active, save, source, onSourceOpened, onBusyChange, onNotice }) {
  const canEdit = useCanEdit();
  const form = Form.useFormInstance();
  const capabilityId = Form.useWatch('capability_id', { form, preserve: true }) ?? form.getFieldValue('capability_id');
  const title = Form.useWatch('title', { form, preserve: true }) ?? form.getFieldValue('title');
  const draft = Form.useWatch('draft', { form, preserve: true }) ?? form.getFieldValue('draft');
  const catalog = useCapabilities({ enabled: active });
  const [mode, setMode] = useState('records');
  const [pendingId, setPendingId] = useState('');
  const [executionId, setExecutionId] = useState('');
  const [links, setLinks] = useState([]);
  const [indexes, setIndexes] = useState({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [revision, setRevision] = useState(0);
  const pendingRead = useRef(null);
  const navigation = useRef(0);
  const visible = useRef(active);
  visible.current = active;
  const sourceRef = useRef(source); sourceRef.current = source;
  const guidanceAttempt = useRef(null);
  const mutation = useRef(false);
  const path = `/api/project/todos/${encodeURIComponent(todoId)}/executions`;
  const navigate = (next) => { navigation.current += 1; setMode(next); };
  const loadLinks = useCallback(async (force = false) => {
    if (!visible.current || pendingRead.current && !force) return null;
    pendingRead.current?.abort();
    const request = new AbortController(); pendingRead.current = request;
    try {
      const result = await apiGet(path, { signal: request.signal });
      if (request.signal.aborted) return null;
      const assignments = result.assignments || [];
      const resolved = Object.fromEntries(await Promise.all(assignments.map(async (record) => {
        try { return [record.execution_id, await apiGet(`/api/executions/${encodeURIComponent(record.execution_id)}/index`, { signal: request.signal })]; }
        catch (failure) { return [record.execution_id, { id: record.execution_id, readError: failure.message }]; }
      })));
      if (request.signal.aborted) return null;
      setLinks(assignments); setIndexes((old) => sourceRef.current && old[sourceRef.current]
        ? { ...resolved, [sourceRef.current]: resolved[sourceRef.current] || old[sourceRef.current] } : resolved); setError('');
      return resolved;
    } catch (failure) { if (!request.signal.aborted) { setLinks([]); setIndexes({}); setError(failure.message); } return null; }
    finally { if (pendingRead.current === request) { pendingRead.current = null; setLoading(false); } }
  }, [path]);
  useEffect(() => {
    if (!active) return;
    setLoading(true); loadLinks();
    const timer = setInterval(() => loadLinks(), 5000);
    return () => { clearInterval(timer); pendingRead.current?.abort(); pendingRead.current = null; navigation.current += 1; };
  }, [active, loadLinks, revision]);
  useEffect(() => {
    if (!active || !source) return;
    const request = new AbortController();
    const attempt = ++navigation.current;
    setMode('detail'); setExecutionId(source); setError('');
    apiGet(`/api/executions/${encodeURIComponent(source)}/index`, { signal: request.signal }).then((index) => {
      if (!request.signal.aborted && navigation.current === attempt) setIndexes((old) => ({ ...old, [source]: index }));
    }).catch((failure) => { if (!request.signal.aborted) setError(failure.message); });
    return () => request.abort();
  }, [active, source]);
  const working = (value) => { mutation.current = value; setBusy(value); onBusyChange(value); };
  const link = async (id, alreadyLinked = false) => {
    if (mutation.current) return;
    const execution = id.trim(); if (!execution) return;
    const attempt = navigation.current; working(true); setError('');
    try {
      if (!alreadyLinked) await apiPost(path, { execution_id: execution });
      setPendingId('');
      const resolved = await loadLinks(true);
      if (navigation.current !== attempt || !visible.current) return;
      if (resolved?.[execution]?.kind) { setExecutionId(execution); setMode('detail'); }
      else { setMode('records'); onNotice(ok('执行已关联；索引可读取后可查看记录')); }
    } catch (failure) { setPendingId(execution); setError(`关联失败：${failure.message}。执行 ID ${execution} 已保留，可重试。`); }
    finally { working(false); }
  };
  const unlink = async (id) => {
    if (mutation.current) return;
    working(true);
    try { await apiDel(`${path}/${encodeURIComponent(id)}`); await loadLinks(true); }
    catch (failure) { setError(failure.message); }
    finally { working(false); }
  };
  const columns = [
    { title: '类型', key: 'kind', width: 110, render: (_, row) => KIND_LABELS[row.kind] || row.kind || '不可读取' },
    { title: '名称', key: 'name', render: (_, row) => row.name || row.id },
    { title: '执行 ID', key: 'id', width: 300, dataIndex: 'id', render: (id, row) => <Typography.Text copyable={{ text: id }}>
      <a className="project-execution-id" href={executionHref(id)} target="_blank" rel="noopener noreferrer" title="在新页面打开执行过程">{id}</a>
    </Typography.Text> },
    { title: '状态', key: 'status', width: 150, render: (_, row) => row.readError || row.status || '等待读取' },
    { title: '操作', key: 'actions', width: 110, render: (_, row) => <Space>
      <Popconfirm title="解除该执行关联？" description="原执行和结果会保留。" onConfirm={() => unlink(row.id)}><EditButton danger type="link" disabled={busy}>解除关联</EditButton></Popconfirm>
    </Space> },
  ];
  const index = indexes[executionId];
  return <div className="project-execution">
    {error && <Alert type="error" showIcon title={error} action={<Button onClick={() => setRevision((v) => v + 1)}>重试读取</Button>} />}
    {pendingId && <Button className="project-todo-retry" loading={busy} onClick={() => link(pendingId)}>重试关联 {pendingId}</Button>}
    {mode !== 'records' && <Button disabled={false} onClick={() => { onSourceOpened(); navigate('records'); }}>返回指派记录</Button>}
    {mode === 'records' && <>
      {catalog.error && <Alert type="error" title={catalog.error} action={<Button onClick={catalog.reload}>重试能力列表</Button>} />}
      <div className="project-execution-launch">
        <Form.Item name="capability_id" label="执行能力"><Select aria-label="执行能力" allowClear loading={catalog.loading} options={capabilityOptions(catalog.capabilities)} showSearch optionFilterProp="label" placeholder="选择能力库中的能力" /></Form.Item>
        <EditButton disabled={!capabilityId || busy || catalog.loading || !!catalog.error} loading={busy} onClick={async () => {
          const attempt = navigation.current;
          if (await save() && visible.current && attempt === navigation.current) navigate('launch');
        }}>指派所选能力</EditButton>
      </div>
      <Typography.Title level={5} style={{ margin: 0 }}>指派记录</Typography.Title>
      <Spin spinning={loading}><ProjectTable label="TODO 指派记录" viewKey={`assignments:${todoId}`} scroll={{ x: 900 }} pagination={false}
        rows={links.map((link) => ({ ...indexes[link.execution_id], ...link, id: link.execution_id, status: indexes[link.execution_id]?.status }))} columns={columns} /></Spin>
      {active && links[0]?.execution_id && <ExecutionResult key={links[0].execution_id} id={links[0].execution_id} />}
    </>}
    {active && mode === 'launch' && <CapabilityLauncher key={capabilityId} capability={catalog.capabilities.find((cap) => cap.id === capabilityId)} todoId={todoId} onNotice={onNotice}
      prompt={[title, draft].filter(Boolean).join('\n\n')} onCreated={(id) => link(id, true)} />}
    {active && mode === 'detail' && (index?.kind ? <ExecutionView key={executionId} executionRef={{ id: executionId, kind: index.kind }} summary={index} onNotice={onNotice}
      mode={['agent', 'operator'].includes(index.kind) ? 'conversation' : 'full'} allowGuidance={canEdit && index.kind === 'team' && index.status === 'running'}
      onGuidance={async (prompt) => {
        const signature = `${executionId}:${prompt}`;
        if (guidanceAttempt.current?.signature !== signature) guidanceAttempt.current = { signature, id: newId('input') };
        try {
          await apiPost(`/api/executions/${encodeURIComponent(executionId)}/commands`, { action: 'steer', input: { prompt, input_id: guidanceAttempt.current.id } });
          guidanceAttempt.current = null; onNotice(ok('引导已提交')); return true;
        } catch (failure) { onNotice(err(failure.message)); return false; }
      }} /> : !error && <Spin />)}
  </div>;
}
