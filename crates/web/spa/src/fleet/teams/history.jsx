import { Alert, Button, Empty, Space, Table, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../api.js';
import { StatusTag } from '../../ui/statusTag.jsx';
import { TimeText } from '../../ui/timeText.jsx';
import { executionPagePath } from '../model.js';

// Embeddable history: the host owns the selected execution and its presentation.
export function TeamExecutionHistory({ onOpen, nodes = [] }) {
  const [cursors, setCursors] = useState([null]), [index, setIndex] = useState(0);
  const [page, setPage] = useState(null), [error, setError] = useState('');
  const [busy, setBusy] = useState(true), [revision, setRevision] = useState(0);
  const path = executionPagePath('team', cursors[index]);
  useEffect(() => {
    const controller = new AbortController();
    let timer;
    setBusy(true); setError('');
    const load = async () => {
      try {
        const result = await apiGet(path, { signal: controller.signal });
        if (!Array.isArray(result.executions)) throw new Error('执行记录格式无效');
        if (!controller.signal.aborted) { setPage({ ...result, path }); setError(''); }
      } catch (failure) { if (!controller.signal.aborted) setError(failure.message || '读取执行记录失败'); }
      finally {
        if (!controller.signal.aborted) { setBusy(false); timer = setTimeout(load, 3000); }
      }
    };
    load();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [path, revision]);
  const current = page?.path === path ? page : null;
  const pending = busy || (!current && !error);
  return <section aria-label="Team 执行记录">
    <Space wrap style={{ marginBottom: 12 }}>
      <Typography.Text type="secondary">记录自动更新，点击执行编号可查看讨论过程。</Typography.Text>
      <Button onClick={() => setRevision((v) => v + 1)}>刷新记录</Button>
    </Space>
    {error && <Alert type="error" showIcon title="执行记录读取失败" description={error}
      action={<Button onClick={() => setRevision((v) => v + 1)}>重试</Button>} />}
    <Table rowKey="id" size="small" tableLayout="fixed" scroll={{ x: 860 }} pagination={false}
      dataSource={current?.executions || []} loading={pending}
      locale={{ emptyText: error || pending ? ' ' : <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无 Team 执行记录" /> }} columns={[
        { title: '执行编号', dataIndex: 'id', width: 250, ellipsis: true, render: (id, row) => <a href="#" title={id} onClick={(event) => { event.preventDefault(); onOpen(row); }}>{id}</a> },
        { title: 'Team', dataIndex: 'name', width: 160, ellipsis: true, render: (v) => v || '—' },
        { title: '状态', dataIndex: 'status', width: 100, render: (v) => <StatusTag status={v} /> },
        { title: '创建时间', dataIndex: 'created_at', width: 140, render: (v) => <TimeText ts={v} /> },
        { title: '执行节点', dataIndex: 'node_id', width: 210, ellipsis: true, render: (id) => nodes.find((node) => node.id === id)?.name || id },
      ]} />
    {(index > 0 || current?.next_cursor) && <Space wrap style={{ marginTop: 12 }}>
      <Button disabled={index === 0 || pending} onClick={() => setIndex((v) => v - 1)}>上一页记录</Button>
      <span>第 {index + 1} 页</span>
      <Button disabled={!current?.next_cursor || pending} onClick={() => { setCursors(cursors.slice(0, index + 1).concat(current.next_cursor)); setIndex((v) => v + 1); }}>下一页记录</Button>
    </Space>}
  </section>;
}
