import { Alert, Button, Empty, Space, Table, Tag, Typography } from 'antd';
import { StatusTag } from '../../../ui/statusTag.jsx';
import { TimeText } from '../../../ui/timeText.jsx';
import './style.css';

export function RunHistory({ history, onOpen }) {
  const { rows, page, next, loading, error, load } = history;
  return <section className="brain-history" aria-label="运行历史">
    <div className="brain-history-toolbar"><Typography.Text type="secondary">按创建时间倒序 · 每次执行独立记录</Typography.Text>
      <Button aria-label="刷新运行" loading={loading} onClick={() => load()}>刷新运行</Button></div>
    {error && <Alert type="error" showIcon title={error} action={<Button aria-label="重试" onClick={() => load()}>重试</Button>} />}
    <Table rowKey="id" loading={loading} dataSource={rows} pagination={false} scroll={{ x: 760 }} columns={[
      { title: '执行计划', key: 'plan', width: 380, render: (_, row) => <div className="brain-history-identity">
        <Button className="brain-history-name" type="link" title={row.name || '未记录计划名称'} onClick={() => onOpen(row.id)}>{row.name || '未记录计划名称'}</Button>
        <Typography.Text className="brain-history-id" type="secondary" copyable>{row.id}</Typography.Text>
      </div> },
      { title: '执行状态', dataIndex: 'status', width: 120, render: (status) => status === 'idle' ? <Tag color="blue">等待事件</Tag> : <StatusTag status={status} /> },
      { title: '创建时间', dataIndex: 'created_at', width: 180, render: (time) => <TimeText ts={time} /> },
      { title: '所属节点', dataIndex: 'node_id', width: 160, render: (id) => <span className="brain-history-node">{id || '—'}</span> },
      { title: '操作', key: 'action', width: 110, render: (_, row) => <Button type="link" onClick={() => onOpen(row.id)}>查看运行</Button> },
    ]} locale={{ emptyText: error ? '运行历史读取失败，请重试' : <Empty description="暂无运行记录，可从计划库启动执行" /> }} />
    <Space className="brain-history-paging"><Button disabled={loading || page === 0} onClick={() => load(page - 1)}>上一页</Button>
      <span>第 {page + 1} 页 · 每页 20 条</span><Button disabled={loading || !next} onClick={() => load(page + 1)}>下一页</Button></Space>
  </section>;
}
