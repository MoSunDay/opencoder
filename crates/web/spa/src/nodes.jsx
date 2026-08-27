// nodes.jsx — Tab 1 「Opencoder 列表」: 3s polling fleet table.
// Status strings come from crates/web/src/nodes_state.rs::compute_status:
// idle | busy | online | lost (staleness > 20s dominates, garbage → online).

import { Button, Popconfirm, Space, Table, Tag, Tooltip, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet } from './api.js';
import { absTime, relTime } from './format.js';
import { openChatForNode, setNodes } from './store.js';

const POLL_MS = 3000;

const STATUS_COLOR = {
  idle: 'green',
  busy: 'blue',
  online: 'cyan',
  lost: 'red',
};

function StatusTag({ status }) {
  const s = String(status || 'online');
  return <Tag color={STATUS_COLOR[s] || 'cyan'}>{s}</Tag>;
}

export function NodesPanel({ onNotice }) {
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const timer = useRef(null);
  const alive = useRef(true);

  const load = useCallback(async (silent) => {
    if (!silent) {
      setLoading(true);
    }
    try {
      const j = await apiGet('/api/nodes');
      const list = (j && j.nodes) || [];
      if (!alive.current) {
        return;
      }
      setRows(list);
      setNodes(list); // share with Tab 2's node selector
    } catch (e) {
      if (!silent && alive.current && onNotice) {
        onNotice('获取 Opencoder 列表失败: ' + (e && e.message));
      }
    } finally {
      if (alive.current && !silent) {
        setLoading(false);
      }
    }
  }, [onNotice]);

  useEffect(() => {
    alive.current = true;
    load(false);
    timer.current = setInterval(() => load(true), POLL_MS);
    return () => {
      alive.current = false;
      clearInterval(timer.current);
    };
  }, [load]);

  const remove = async (id) => {
    try {
      await apiDel('/api/nodes/' + encodeURIComponent(id));
      await load(true);
    } catch (e) {
      if (onNotice) {
        onNotice('删除失败: ' + (e && e.message));
      }
    }
  };

  const columns = [
    { title: '名称', dataIndex: 'name', key: 'name', render: (v) => v || '-' },
    {
      title: '地址',
      dataIndex: 'addr',
      key: 'addr',
      ellipsis: true,
      render: (v) => v || <Typography.Text type="secondary">-</Typography.Text>,
    },
    {
      title: '心跳',
      dataIndex: 'last_seen_at',
      key: 'last_seen_at',
      width: 110,
      render: (ts) => (
        <Tooltip title={absTime(ts)}>
          <span>{relTime(ts)}</span>
        </Tooltip>
      ),
    },
    {
      title: '状态',
      dataIndex: 'status',
      key: 'status',
      width: 90,
      render: (_, r) => <StatusTag status={r.status} />,
    },
    { title: '版本', dataIndex: 'version', key: 'version', width: 110, render: (v) => v || '-' },
    {
      title: '操作',
      key: 'ops',
      width: 200,
      render: (_, r) => (
        <Space>
          <Button
            size="small"
            type="link"
            onClick={() => openChatForNode(r.id)}
          >
            打开对话
          </Button>
          <Popconfirm
            title="删除该 Opencoder？"
            description="仅从舰队列表移除，不影响远端进程。"
            okText="删除"
            okButtonProps={{ danger: true }}
            cancelText="取消"
            onConfirm={() => remove(r.id)}
          >
            <Button size="small" type="link" danger>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <Table
      rowKey="id"
      size="middle"
      columns={columns}
      dataSource={rows}
      loading={loading}
      pagination={false}
      locale={{ emptyText: '暂无 Opencoder 节点' }}
    />
  );
}
