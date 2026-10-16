import { Alert, Button, Input, Popconfirm, Space, Table, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet, apiDel } from '../../api.js';
import { useCanEdit } from '../../ui/permissions.jsx';
import { downloadFile, fileUrl, uploadFile } from './api.js';

export function TodoFiles({ todoId, active, onBusyChange, onDraftChange }) {
  const canEdit = useCanEdit();
  const [data, setData] = useState(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const [revision, setRevision] = useState(0);
  const [file, setFile] = useState(null);
  const [path, setPath] = useState('');
  const input = useRef(null);
  const operation = useRef(null);
  const mounted = useRef(true);
  const listUrl = `/api/project/todos/${encodeURIComponent(todoId)}/files`;
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; operation.current?.abort(); };
  }, []);
  useEffect(() => { onDraftChange?.(Boolean(file)); }, [file, onDraftChange]);
  useEffect(() => {
    if (!active) return undefined;
    const controller = new AbortController();
    setLoading(true); setError('');
    apiGet(listUrl, { signal: controller.signal }).then((result) => {
      if (controller.signal.aborted) return;
      if (!Array.isArray(result.files)) throw new Error('文件列表响应不完整');
      setData(result);
    }).catch((failure) => { if (!controller.signal.aborted) { setData(null); setError(failure.message); } })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [active, listUrl, revision]);
  async function run(action) {
    if (operation.current) return;
    const controller = new AbortController(); operation.current = controller;
    setBusy(true); onBusyChange?.(true); setError('');
    try { await action(controller.signal); }
    catch (failure) { if (mounted.current) setError(failure.message); }
    finally {
      operation.current = null;
      if (mounted.current) { setBusy(false); onBusyChange?.(false); }
    }
  }
  function upload() {
    if (!file || !path.trim() || !data || !canEdit) return;
    if (file.size > data.max_file_bytes) { setError(`文件超过大小上限 ${data.max_file_bytes} 字节`); return; }
    run(async (signal) => {
      await uploadFile(todoId, path.trim(), file, signal);
      setFile(null); setPath(''); input.current.value = ''; setRevision((n) => n + 1);
    });
  }
  return <section aria-label="任务文件" style={{ minWidth: 0 }}>
    <Space wrap style={{ marginBottom: 16 }}>
      <Typography.Text>保存交接文档、日志和任务产物。文件单独提交，不影响关键信息草稿。</Typography.Text>
      <Button loading={loading} disabled={busy} onClick={() => setRevision((n) => n + 1)}>刷新文件</Button>
    </Space>
    {error && <Alert type="error" showIcon title={error} style={{ marginBottom: 12 }} />}
    {data && <Typography.Paragraph type="secondary" style={{ overflowWrap: 'anywhere' }}>
      存储目录：{data.directory} · 单文件上限 {Math.round(data.max_file_bytes / 1024 / 1024)} MiB
    </Typography.Paragraph>}
    {canEdit && <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, marginBottom: 16 }}>
      <input ref={input} type="file" aria-label="选择任务文件" disabled={busy} style={{ maxWidth: '100%' }} onChange={(event) => {
        const selected = event.target.files?.[0] || null; setFile(selected); setPath(selected?.name || '');
      }} />
      <Input aria-label="文件存储路径" value={path} disabled={busy} placeholder="文件名或目录/文件名" style={{ flex: '1 1 200px', minWidth: 0 }} onChange={(e) => setPath(e.target.value)} />
      <Button type="primary" loading={busy} disabled={!file || !path.trim() || !data} onClick={upload}>上传文件</Button>
      {file && <Button disabled={busy} onClick={() => { setFile(null); setPath(''); input.current.value = ''; }}>取消选择</Button>}
    </div>}
    {data && <Table size="small" rowKey="path" dataSource={data.files} pagination={{ pageSize: 20 }} scroll={{ x: 600 }} columns={[
      { title: '文件', dataIndex: 'path', render: (value) => <span style={{ overflowWrap: 'anywhere' }}>{value}</span> },
      { title: '大小', dataIndex: 'bytes', width: 110, render: (value) => `${value.toLocaleString()} B` },
      { title: '修改时间', dataIndex: 'modified_at', width: 180, render: (value) => new Date(value).toLocaleString() },
      { title: '操作', key: 'actions', width: 145, render: (_, entry) => <Space>
        <Button size="small" disabled={busy} onClick={() => run((signal) => downloadFile(todoId, entry.path, signal))}>下载</Button>
        {canEdit && <Popconfirm title={`删除文件 ${entry.path}？`} description="删除后无法恢复。" okText="删除" cancelText="取消" onConfirm={() => run(async (signal) => {
          await apiDel(fileUrl(todoId, entry.path), { signal }); setRevision((n) => n + 1);
        })}><Button size="small" danger disabled={busy}>删除</Button></Popconfirm>}
      </Space> },
    ]} />}
  </section>;
}
