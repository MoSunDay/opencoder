// runsTable.jsx — DAG「运行」tab list: polled run rows (GET /api/dag/runs?
// limit=50, newest first), status Tag, 执行节点 badge, cancel action
// (POST /api/dag/runs/:id/cancel), and the entry into the run detail view
// (runDetail.jsx). When a run id is focused (fresh dispatch / finished live
// run), the detail view opens directly.

import { Button, Popconfirm, Space, Table, Tag, Tooltip, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from '../api.js';
import { absTime, relTime } from '../format.js';
import { CANCELLABLE, NodeBadge, RunStatusTag } from './runBits.jsx';
import { useStore } from '../store.js';
import { RunDetail } from './runDetail.jsx';

const { Text } = Typography;

const POLL_MS = 3000;

export function RunsTable({ onNotice, refreshSignal, focusRunId, onDetailClosed }) {
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [detail, setDetail] = useState(null); // run row shown in RunDetail
  const timer = useRef(null);
  const alive = useRef(true);

  const load = useCallback(
    async (silent) => {
      if (!silent) {
        setLoading(true);
      }
      try {
        const list = await apiGet('/api/dag/runs?limit=50');
        if (!alive.current) {
          return;
        }
        setRows(Array.isArray(list) ? list : []);
      } catch (e) {
        if (!silent && alive.current && onNotice) {
          onNotice('获取运行列表失败: ' + (e && e.message));
        }
      } finally {
        if (alive.current && !silent) {
          setLoading(false);
        }
      }
    },
    [onNotice],
  );

  useEffect(() => {
    alive.current = true;
    load(false);
    timer.current = setInterval(() => load(true), POLL_MS);
    return () => {
      alive.current = false;
      clearInterval(timer.current);
    };
  }, [load]);

  // External refresh trigger (dispatch elsewhere, detail's run_finished).
  useEffect(() => {
    if (refreshSignal) {
      load(true);
    }
  }, [refreshSignal, load]);

  // Focus trigger: open a specific run's detail (fresh dispatch).
  useEffect(() => {
    if (focusRunId) {
      apiGet('/api/dag/runs/' + encodeURIComponent(focusRunId))
        .then((r) => {
          if (alive.current) {
            setDetail(r);
          }
        })
        .catch((e) => {
          if (onNotice) {
            onNotice('打开运行失败: ' + (e && e.message));
          }
        });
    }
  }, [focusRunId, onNotice]);

  const cancel = async (id) => {
    try {
      await apiPost('/api/dag/runs/' + encodeURIComponent(id) + '/cancel', {});
      await load(true);
    } catch (e) {
      if (onNotice) {
        onNotice('取消运行失败: ' + (e && e.message));
      }
    }
  };

  if (detail) {
    return (
      <RunDetail
        run={detail}
        onNotice={onNotice}
        onClose={() => {
          setDetail(null);
          if (onDetailClosed) {
            onDetailClosed();
          }
          load(true);
        }}
      />
    );
  }

  const columns = [
    {
      title: '运行 ID',
      dataIndex: 'id',
      key: 'id',
      width: 110,
      render: (v) => (
        <Tooltip title={v}>
          <Text code>{String(v || '').slice(0, 8)}</Text>
        </Tooltip>
      ),
    },
    { title: '名称', dataIndex: 'name', key: 'name', render: (v) => v || '-' },
    {
      title: '状态',
      dataIndex: 'status',
      key: 'status',
      width: 100,
      render: (v, r) => (
        <Tooltip title={r.error || v}>
          <RunStatusTag status={v} />
        </Tooltip>
      ),
    },
    {
      title: '执行节点',
      dataIndex: 'node_id',
      key: 'node_id',
      width: 190,
      render: (v, r) => <NodeBadge nodeId={v} status={r.status} />,
    },
    {
      title: '创建时间',
      dataIndex: 'created_at',
      key: 'created_at',
      width: 130,
      render: (ts) => (
        <Tooltip title={absTime(ts)}>
          <span>{relTime(ts)}</span>
        </Tooltip>
      ),
    },
    {
      title: '结束时间',
      dataIndex: 'finished_at',
      key: 'finished_at',
      width: 130,
      render: (ts) =>
        ts ? (
          <Tooltip title={absTime(ts)}>
            <span>{relTime(ts)}</span>
          </Tooltip>
        ) : (
          <Text type="secondary">—</Text>
        ),
    },
    {
      title: '操作',
      key: 'ops',
      width: 150,
      render: (_, r) => (
        <Space>
          <Button size="small" type="link" onClick={() => setDetail(r)}>
            查看
          </Button>
          <Popconfirm
            title="取消该运行？"
            description="正在执行的步骤会被中断，运行进入 cancelled。"
            okText="取消运行"
            okButtonProps={{ danger: true }}
            cancelText="返回"
            onConfirm={() => cancel(r.id)}
            disabled={!CANCELLABLE.includes(r.status)}
          >
            <Button size="small" type="link" danger disabled={!CANCELLABLE.includes(r.status)}>
              取消
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <Space direction="vertical" size={12} style={{ width: '100%' }}>
      <Space>
        <Button onClick={() => load(false)}>刷新</Button>
        <Text type="secondary">每 3s 自动刷新，最多展示最近 50 条。</Text>
      </Space>
      <Table
        rowKey="id"
        size="middle"
        columns={columns}
        dataSource={rows}
        loading={loading}
        pagination={false}
        locale={{ emptyText: '暂无运行记录，先在「定义」页派发一个工作流' }}
      />
    </Space>
  );
}
