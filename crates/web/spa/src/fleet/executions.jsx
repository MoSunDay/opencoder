import { Button, Select, Space, Table } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet } from '../api.js';
import { PageShell } from '../shell/pageShell.jsx';
import { StatusTag } from '../ui/statusTag.jsx';
import { MONO_VAR } from '../ui/mono.js';
import { tableLoading, tableRows } from '../ui/tableLoading.js';
import { TimeText } from '../ui/timeText.jsx';
import { ExecutionDetail } from './detail.jsx';
import { KIND_LABELS, KINDS, executionPagePath } from './model.js';
import { err } from '../notice.js';

export function ExecutionsPanel({ onNotice }) {
  const [rows, setRows] = useState([]); const [nodes, setNodes] = useState([]);
  const [detail, setDetail] = useState(null); const [filter, setFilter] = useState('');
  /// 首屏、筛选和手动刷新遮罩表格；3s poll 静默，免得表格每 3 秒闪一次 spinner。
  const [loading, setLoading] = useState(true);
  const load = useCallback(async (mode = 'reset') => {
    if (mode !== 'poll') setLoading(true);
    try {
      const [a, b] = await Promise.all([apiGet(executionPagePath(filter)), apiGet('/api/nodes')]);
      const page = a.executions || [];
      setRows(page);
      setNodes(b.nodes || []);
    }
    catch (e) { onNotice(err(e.message)); }
    finally { if (mode !== 'poll') setLoading(false); }
  }, [filter, onNotice]);
  useEffect(() => {
    let live = true;
    const refresh = () => { if (live) load('poll'); };
    load('reset'); const timer = setInterval(refresh, 3000);
    return () => { live = false; clearInterval(timer); };
  }, [filter, load]);
  return <PageShell page="topics">
    <Space style={{ margin: '12px 0' }}><Select aria-label="执行类型筛选" style={{ width: 180 }} value={filter} onChange={setFilter} options={[{ value: '', label: '全部执行' }, ...KINDS]} /><Button onClick={() => load('reset')}>刷新</Button></Space>
    <Table scroll={{ x: 'max-content' }} rowKey="id" dataSource={tableRows(loading, rows)} size="small" loading={tableLoading(loading)} columns={[
      { title: 'ID', dataIndex: 'id', render: (id, row) => <Button type="link" style={{ fontFamily: MONO_VAR }} onClick={() => setDetail(row)}>{id}</Button> },
      { title: '类型', dataIndex: 'kind', render: (v) => KIND_LABELS[v] || v },
      { title: '名称', dataIndex: 'name', render: (v) => v || '-' },
      { title: '创建时间', dataIndex: 'created_at', render: (v) => <TimeText ts={v} /> },
      { title: '所属节点', dataIndex: 'node_id', render: (id) => <Space size={4}><span style={{ fontFamily: MONO_VAR }}>{id}</span><StatusTag status={nodes.find((node) => node.id === id)?.online ? 'online' : 'offline'} /></Space> },
      { title: '状态', dataIndex: 'status', render: (v) => <StatusTag status={v} /> },
    ]} />
    {detail && <ExecutionDetail id={detail.id} summary={detail} onClose={() => setDetail(null)} onNotice={onNotice} />}
  </PageShell>;
}
