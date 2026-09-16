// nodeTable.jsx — 「节点总览」节点表：复用 fleet/useNodes.js 的 5s 轮询快照。
// 只读节点总览：Agent 页（chat）的会话即以 operator 执行在节点宿主机进程
// 内运行，本表不再提供启动入口，仅展示支持 operator 的在线节点与负载。

import { Alert, Space, Table } from 'antd';
import { useNodes } from '../fleet/useNodes.js';
import { StatusTag } from '../ui/statusTag.jsx';

export function NodeTable() {
  const { nodes, error } = useNodes();
  const columns = [
    {
      title: '节点',
      dataIndex: 'name',
      render: (v, r) => (
        <Space orientation="vertical" size={2}>
          <b>{v}</b>
          <small style={{ fontFamily: 'var(--oc-mono, monospace)' }}>{r.id}</small>
        </Space>
      ),
    },
    {
      title: '状态',
      render: (_, r) => (
        <StatusTag
          status={r.online ? 'online' : 'offline'}
          label={r.online ? (r.snapshot?.resource_error || '在线') : '离线'}
          color={r.online && r.snapshot?.ready ? 'success' : 'error'}
        />
      ),
    },
    { title: 'CPU', render: (_, r) => r.snapshot?.cpu_capacity ?? '—' },
    { title: '运行中', render: (_, r) => r.snapshot?.active_agent_loops ?? '—' },
    { title: '待处理', render: (_, r) => r.snapshot?.pending_runs ?? '—' },
  ];

  return (
    <div>
      {error ? <Alert type="error" showIcon title={error} style={{ marginBottom: 12 }} /> : null}
      <Table
        rowKey="id"
        size="small"
        columns={columns}
        dataSource={nodes}
        pagination={false}
        scroll={{ x: 'max-content' }}
        locale={{ emptyText: '暂无节点' }}
      />
    </div>
  );
}
