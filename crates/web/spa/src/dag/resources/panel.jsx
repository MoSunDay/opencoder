import { Alert, Button, Descriptions, Drawer, Input, Modal, Space, Table, Tag, Typography } from 'antd';
import { useState } from 'react';
import { apiDel, apiPost, authFetch } from '../../api.js';
import { useJsonQuery } from '../../ui/requests/query.js';
import { BinaryEditor } from './editor.jsx';
import { readHistory, readPools } from './read.js';

export function BinaryResources() {
  const { data, loading, error: readError, reload } = useJsonQuery('/api/dag/binaries', readPools);
  const pools = data || [];
  const [search, setSearch] = useState('');
  const [error, setError] = useState('');
  const [editor, setEditor] = useState(null);
  const [selected, setSelected] = useState(null);
  const [removing, setRemoving] = useState(null);
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const remove = async () => {
    if (busy) return;
    setBusy(true);
    try { await apiDel(`/api/dag/binaries/${encodeURIComponent(removing.name)}`); setRemoving(null); await reload(); }
    catch (failure) { setError(failure.message); }
    finally { setBusy(false); }
  };
  const query = search.trim().toLowerCase();
  return <Space orientation="vertical" style={{ width: '100%' }}>
    <Alert type="info" showIcon title="一次 DAG 运行共享一个容器和工作区" description="二进制和 Agent 步骤在同一个容器内执行。资源在受理时固定，更新或切换当前版本只影响后续任务。" />
    {(error || readError) && <Alert type="error" showIcon title={error || readError} action={<Button onClick={() => { setError(''); reload(); }}>重试资源列表</Button>} />}
    <Space wrap><Input.Search aria-label="搜索二进制资源" placeholder="搜索二进制资源" value={search} onChange={(event) => setSearch(event.target.value)} allowClear />
      <Button type="primary" onClick={() => setEditor({})}>上传二进制</Button><Button onClick={reload}>刷新资源</Button></Space>
    <Table rowKey="name" loading={loading} dataSource={pools.filter((pool) => `${pool.name} ${pool.description}`.toLowerCase().includes(query))} scroll={{ x: 'max-content' }} locale={{ emptyText: error || readError ? '资源读取失败，请重试' : '暂无二进制资源' }} columns={[
      { title: '名称', dataIndex: 'name', render: (name) => <Button type="link" onClick={() => setSelected(name)}>{name}</Button> },
      { title: '说明', dataIndex: 'description' },
      { title: '当前版本', dataIndex: 'current', render: (version) => `v${version}` },
      { title: '字节数', render: (_, pool) => pool.current_version?.size_bytes ?? '版本不可用' },
      { title: 'SHA-256', render: (_, pool) => <Typography.Text copyable={!!pool.current_version?.sha256}>{pool.current_version?.sha256 || '版本不可用'}</Typography.Text> },
      { title: '操作', render: (_, pool) => <Space><Button onClick={() => setEditor(pool)}>追加版本</Button><Button danger onClick={() => setRemoving(pool)}>删除资源</Button></Space> },
    ]} />
    {editor && <BinaryEditor pool={editor.name ? editor : null} onClose={() => setEditor(null)} onSaved={async () => { await reload(); setRevision((value) => value + 1); }} />}
    {selected && <BinaryHistory key={`${selected}:${revision}`} name={selected} onClose={() => setSelected(null)} onChanged={reload} />}
    <Modal open={!!removing} title={`删除二进制资源 · ${removing?.name || ''}`} okText="确认删除资源" cancelText="取消" okButtonProps={{ danger: true }} confirmLoading={busy} onOk={remove} onCancel={() => { if (!busy) setRemoving(null); }}>
      删除资源及全部版本后，引用它的新任务无法启动。已受理任务的固定副本和执行历史保留。
    </Modal>
  </Space>;
}

function BinaryHistory({ name, onClose, onChanged }) {
  const { data: pool, error: readError, loading, reload: load } = useJsonQuery(`/api/dag/binaries/${encodeURIComponent(name)}`, readHistory);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [target, setTarget] = useState(null);
  const switchVersion = async () => {
    if (busy) return;
    setBusy(true);
    try { await apiPost(`/api/dag/binaries/${encodeURIComponent(name)}/rollback`, { version: target }); setTarget(null); await load(); await onChanged(); }
    catch (failure) { setError(failure.message); }
    finally { setBusy(false); }
  };
  const download = async (version) => {
    try {
      const response = await authFetch('GET', `/api/dag/binaries/${encodeURIComponent(name)}/versions/${version}/binary.bin`);
      if (!response.ok) throw new Error(`下载失败：HTTP ${response.status}`);
      const url = URL.createObjectURL(await response.blob());
      const anchor = document.createElement('a'); anchor.href = url; anchor.download = `${name}-v${version}`; anchor.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (failure) { setError(failure.message); }
  };
  return <Drawer open title={`二进制版本 · ${name}`} onClose={onClose} size="large" styles={{ wrapper: { maxWidth: '100vw' } }}>
    {(error || readError) && <Alert type="error" showIcon title={error || readError} action={<Button onClick={() => { setError(''); load(); }}>重试版本历史</Button>} />}
    {loading && <Typography.Text>加载版本历史…</Typography.Text>}
    {pool && <><Descriptions column={1} items={[{ key: 'name', label: '名称', children: pool.name }, { key: 'current', label: '当前版本', children: `v${pool.current}` }]} />
      <Table rowKey="version" dataSource={pool.history} scroll={{ x: 'max-content' }} columns={[
        { title: '版本', dataIndex: 'version', render: (version) => <Space>v{version}{pool.current === version && <Tag color="success">当前</Tag>}</Space> },
        { title: '说明', dataIndex: 'description' }, { title: '字节数', dataIndex: 'size_bytes' },
        { title: 'SHA-256', dataIndex: 'sha256', render: (digest) => <Typography.Text copyable>{digest}</Typography.Text> },
        { title: '操作', render: (_, version) => <Space><Button disabled={loading || !!readError} onClick={() => download(version.version)}>下载 v{version.version}</Button><Button disabled={busy || loading || !!readError || pool.current === version.version} onClick={() => setTarget(version.version)}>使用 v{version.version}</Button></Space> },
      ]} /></>}
    <Modal open={target !== null} title={`切换当前版本为 v${target}`} okText="确认切换版本" cancelText="取消" confirmLoading={busy} onOk={switchVersion} onCancel={() => { if (!busy) setTarget(null); }}>
      只切换后续任务使用的当前版本，不修改已有版本或已受理任务。
    </Modal>
  </Drawer>;
}
