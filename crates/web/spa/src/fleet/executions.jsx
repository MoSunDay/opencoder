import { Button, Select, Space, Table } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../api.js';
import { PageShell } from '../shell/pageShell.jsx';
import { StatusTag } from '../ui/statusTag.jsx';
import { MONO_VAR } from '../ui/mono.js';
import { tableLoading, tableRows } from '../ui/tableLoading.js';
import { TimeText } from '../ui/timeText.jsx';
import { ExecutionDetail } from './detail.jsx';
import { ExecutionLaunchModal } from './executionLaunch.jsx';
import { KIND_LABELS, KINDS, executionPagePath } from './model.js';
import { err } from '../notice.js';

export function ExecutionsPanel({ onNotice }) {
  const [rows, setRows] = useState([]); const [nodes, setNodes] = useState([]);
  const [detail, setDetail] = useState(null); const [filter, setFilter] = useState('');
  const [more, setMore] = useState(false); const [loadingMore, setLoadingMore] = useState(false);
  const [launch, setLaunch] = useState(false);
  /// 索引表拉取态：只有 reset（首屏/换筛选/刷新）会遮罩表格。append（翻页）
  /// 刻意不遮罩——遮罩给表格加 pointer-events: none，ID 链接当场点不动，还会
  /// 和「加载更早的执行」按钮自带的 spinner 撞成两个；3s poll 静默，免得表格
  /// 每 3 秒闪一次 spinner。
  const [loading, setLoading] = useState(true);
  const cursor = useRef(null); const extended = useRef(false);
  const load = useCallback(async (mode = 'reset') => {
    const append = mode === 'append';
    if (append) setLoadingMore(true); else if (mode !== 'poll') setLoading(true);
    try {
      const [a, b] = await Promise.all([apiGet(executionPagePath(filter, append ? cursor.current : null)), apiGet('/api/nodes')]);
      const page = a.executions || [];
      if (append) {
        setRows((old) => [...old, ...page.filter((row) => !old.some((item) => item.id === row.id))]);
        extended.current = true; cursor.current = a.next_cursor || null; setMore(!!a.next_cursor);
      } else if (mode === 'poll' && extended.current) {
        setRows((old) => [...page, ...old.filter((row) => !page.some((fresh) => fresh.id === row.id))]);
      } else {
        setRows(page); extended.current = false; cursor.current = a.next_cursor || null; setMore(!!a.next_cursor);
      }
      setNodes(b.nodes || []);
    }
    catch (e) { onNotice(err(e.message)); }
    finally { if (append) setLoadingMore(false); else if (mode !== 'poll') setLoading(false); }
  }, [filter, onNotice]);
  useEffect(() => {
    let live = true;
    const refresh = () => { if (live) load('poll'); };
    load('reset'); const timer = setInterval(refresh, 3000);
    return () => { live = false; clearInterval(timer); };
  }, [filter, load]);
  return <PageShell page="topics">
    <Space style={{ margin: '12px 0' }}><Select aria-label="执行类型筛选" style={{ width: 180 }} value={filter} onChange={setFilter} options={[{ value: '', label: '全部执行' }, ...KINDS]} /><Button onClick={() => load('reset')}>刷新</Button><Button type="primary" onClick={() => setLaunch(true)}>启动执行</Button></Space>
    <Table scroll={{ x: 'max-content' }} rowKey="id" dataSource={tableRows(loading, rows)} size="small" loading={tableLoading(loading)} columns={[
      { title: 'ID', dataIndex: 'id', render: (id, row) => <Button type="link" style={{ fontFamily: MONO_VAR }} onClick={() => setDetail(row)}>{id}</Button> },
      { title: '类型', dataIndex: 'kind', render: (v) => KIND_LABELS[v] || v },
      { title: '创建时间', dataIndex: 'created_at', render: (v) => <TimeText ts={v} /> },
      { title: '所属节点', dataIndex: 'node_id', render: (id) => <Space size={4}><span style={{ fontFamily: MONO_VAR }}>{id}</span><StatusTag status={nodes.find((node) => node.id === id)?.online ? 'online' : 'offline'} /></Space> },
      { title: '状态', dataIndex: 'status', render: (v) => <StatusTag status={v} /> },
    ]} />
    {more && <Button block loading={loadingMore} onClick={() => load('append')}>加载更早的执行</Button>}
    {detail && <ExecutionDetail id={detail.id} summary={detail} onClose={() => setDetail(null)} onNotice={onNotice} />}
    {launch && <ExecutionLaunchModal open nodes={nodes} onClose={() => setLaunch(false)} onLaunched={(result) => { setLaunch(false); setDetail(result); load('reset'); }} onNotice={onNotice} />}
  </PageShell>;
}
