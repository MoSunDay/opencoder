// nodeTable.jsx — Operator 节点表：复用 fleet/useNodes.js 的 5s 轮询快照。
// 「启动 Operator」仅对 在线 + snapshot.ready + kinds 含 operator 的节点
// 可用（与 model.js nodeOptions 的禁用口径一致）。

import { Alert, Button, Space, Table } from 'antd';
import { useNodes } from '../fleet/useNodes.js';
import { StatusTag } from '../ui/statusTag.jsx';

/// 节点当前能否承载 operator 执行（kinds 数组由节点注册时上报）。
export function operable(node) {
  return !!(node.online && node.snapshot?.ready && node.kinds?.includes('operator'));
}

export function NodeTable({ onLaunch }) {
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
    {
      title: '操作',
      render: (_, r) => (
        <Button size="small" type="primary" disabled={!operable(r)} onClick={() => onLaunch(r)}>
          启动 Operator
        </Button>
      ),
    },
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
