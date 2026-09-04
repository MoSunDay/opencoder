// todoRunsPanel.jsx — TODO 工作流「运行」视图：左侧工作流列表（GET
// /api/todo/workflows?limit=50，3s 轮询——仅当仍有 running 时），点击行右侧
// 展开 items 表 + SSE 事件流（openStream 订阅 /events，事件名 = kind）。
// 流在 workflow_completed/workflow_failed 后服务器即关流，这里主动 abort，
// 避免 sse.js 把「干净关闭」当作断线去空重连。

import { Button, Card, Col, Empty, Row, Space, Table, Tag, Tooltip, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from './api.js';
import { absTime, relTime } from './format.js';
import { openStream } from './sse.js';

const { Text } = Typography;

const POLL_MS = 3000;
const MAX_EVENTS = 200;
/// SSE 终帧事件名（服务器随后关流）。
export const TERMINAL_KINDS = ['workflow_completed', 'workflow_failed'];

const STATUS_COLOR = {
  pending: 'default',
  running: 'processing',
  suspended: 'warning',
  completed: 'success',
  failed: 'error',
};

export function StatusTag({ status }) {
  const s = String(status || '');
  return <Tag color={STATUS_COLOR[s] || 'default'}>{s || '-'}</Tag>;
}

/// 事件 payload 摘要：todo_id / status 优先，够定位即可。
export function summarizePayload(data) {
  if (!data || typeof data !== 'object') {
    return '';
  }
  const parts = [];
  if (data.todo_id !== undefined) {
    parts.push('todo ' + data.todo_id);
  }
  if (data.status !== undefined) {
    parts.push(String(data.status));
  }
  return parts.join(' · ');
}

function EventsFeed({ workflowId, onNotice, onTerminal }) {
  const [events, setEvents] = useState([]);

  useEffect(() => {
    setEvents([]);
    let stopped = false;
    const handle = openStream({
      path: `/api/todo/workflows/${encodeURIComponent(workflowId)}/events`,
      after: 0,
      onFrame: (f) => {
        if (stopped) {
          return;
        }
        const kind = (f && f.event) || 'message';
        setEvents((list) => list.concat({
          seq: f && f.seq,
          kind,
          summary: summarizePayload(f && f.data),
        }).slice(-MAX_EVENTS));
        if (TERMINAL_KINDS.includes(kind)) {
          handle.abort(); // 服务器关流在即：主动停，防 sse.js 空重连
          if (onTerminal) {
            onTerminal();
          }
        }
      },
      onStatus: (st) => {
        if (st === 'failed' && !stopped && onNotice) {
          onNotice('TODO 事件流连接失败（已重试 5 次）');
        }
      },
    });
    return () => {
      stopped = true;
      handle.abort();
    };
  }, [workflowId, onNotice, onTerminal]);

  if (!events.length) {
    return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无事件" />;
  }
  return (
    <div style={{ maxHeight: 260, overflow: 'auto', fontSize: 12 }}>
      {events.map((e, i) => (
        <div key={(e.seq === undefined || e.seq === null ? 'x' + i : e.seq) + ':' + e.kind} style={{ padding: '1px 0' }}>
          <Text type="secondary">#{e.seq === undefined || e.seq === null ? '-' : e.seq}</Text>
          {' '}
          <Text strong>{e.kind}</Text>
          {e.summary ? <span> — {e.summary}</span> : null}
        </div>
      ))}
    </div>
  );
}

function WorkflowDetail({ workflowId, onNotice, onMutated }) {
  const [detail, setDetail] = useState(null);

  const load = useCallback(async () => {
    try {
      const j = await apiGet(`/api/todo/workflows/${encodeURIComponent(workflowId)}`);
      setDetail(j || null);
    } catch (e) {
      if (onNotice) {
        onNotice('获取工作流详情失败: ' + (e && e.message));
      }
    }
  }, [workflowId, onNotice]);

  useEffect(() => {
    setDetail(null);
    load();
  }, [load]);

  const wf = (detail && detail.workflow) || null;
  const items = (detail && detail.items) || [];
  const status = wf ? String(wf.status || '') : '';
  const terminal = status === 'completed' || status === 'failed';

  const interrupt = async () => {
    try {
      await apiPost(`/api/todo/workflows/${encodeURIComponent(workflowId)}/interrupt`, {});
      load();
      if (onMutated) {
        onMutated();
      }
    } catch (e) {
      if (onNotice) {
        onNotice('中断失败: ' + (e && e.message)); // 500 = 已终态
      }
    }
  };

  const resume = async () => {
    try {
      await apiPost(`/api/todo/workflows/${encodeURIComponent(workflowId)}/resume`, {});
      load();
      if (onMutated) {
        onMutated();
      }
    } catch (e) {
      if (onNotice) {
        onNotice('恢复失败: ' + (e && e.message)); // 409 = 运行中
      }
    }
  };

  const itemCols = [
    { title: 'TODO', dataIndex: 'todo_id', key: 'todo_id', width: 120, ellipsis: true },
    { title: '状态', dataIndex: 'status', key: 'status', width: 110,
      render: (s) => <StatusTag status={s} /> },
    { title: '尝试', dataIndex: 'attempt', key: 'attempt', width: 60 },
    { title: '会话', dataIndex: 'active_session_id', key: 'sid', ellipsis: true,
      render: (v) => (v ? <Text copyable={{ text: v }} style={{ fontSize: 12 }}>{String(v).slice(0, 14) + '…'}</Text> : '-') },
  ];

  return (
    <div>
      <Card size="small" title={<Tooltip title={workflowId}><span>{String(workflowId).slice(0, 18)}…</span></Tooltip>}
        extra={(
          <Space>
            <Button size="small" danger disabled={!wf || terminal} onClick={interrupt}>中断</Button>
            <Button size="small" disabled={!wf || !(status === 'suspended' || status === 'pending')} onClick={resume}>恢复</Button>
          </Space>
        )}
        style={{ marginBottom: 12 }}
      >
        {wf ? (
          <Space wrap size={16}>
            <span><StatusTag status={wf.status} /></span>
            <Text type="secondary">父会话: {wf.parent_session_id || '-'}</Text>
            <Text type="secondary">generation: {wf.generation === undefined ? '-' : wf.generation}</Text>
          </Space>
        ) : <Text type="secondary">加载中…</Text>}
      </Card>
      <Card size="small" title="TODO 项" style={{ marginBottom: 12 }}>
        <Table rowKey={(r) => (r && r.todo_id) || ''} size="small" columns={itemCols}
          dataSource={items} pagination={false} />
      </Card>
      <Card size="small" title="事件流">
        <EventsFeed workflowId={workflowId} onNotice={onNotice} onTerminal={load} />
      </Card>
    </div>
  );
}

export function TodoRunsPanel({ onNotice, focusWorkflowId, onFocusConsumed }) {
  const [rows, setRows] = useState([]);
  const [selectedId, setSelectedId] = useState('');
  const [loading, setLoading] = useState(false);
  const rowsRef = useRef([]);
  const alive = useRef(true);

  const load = useCallback(async (silent) => {
    if (!silent) {
      setLoading(true);
    }
    try {
      const j = await apiGet('/api/todo/workflows?limit=50');
      const list = (j && j.workflows) || [];
      if (!alive.current) {
        return;
      }
      setRows(list);
      rowsRef.current = list;
    } catch (e) {
      if (!silent && alive.current && onNotice) {
        onNotice('获取工作流列表失败: ' + (e && e.message));
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
    // 3s 轮询，但只在仍有 running 工作流时真正拉取。
    const timer = setInterval(() => {
      if (rowsRef.current.some((r) => r && r.status === 'running')) {
        load(true);
      }
    }, POLL_MS);
    return () => {
      alive.current = false;
      clearInterval(timer);
    };
  }, [load]);

  // 外部聚焦（模板 tab 的「运行」成功后跳转）。
  useEffect(() => {
    if (focusWorkflowId) {
      setSelectedId(focusWorkflowId);
      if (onFocusConsumed) {
        onFocusConsumed();
      }
    }
  }, [focusWorkflowId, onFocusConsumed]);

  const wfCols = [
    { title: 'ID', dataIndex: 'id', key: 'id', ellipsis: true,
      render: (v) => <Tooltip title={v}><span style={{ fontFamily: 'monospace' }}>{String(v || '').slice(0, 16)}…</span></Tooltip> },
    { title: '状态', dataIndex: 'status', key: 'status', width: 110,
      render: (s) => <StatusTag status={s} /> },
    { title: '更新时间', dataIndex: 'updated_at', key: 'updated_at', width: 110,
      render: (ts) => <Tooltip title={absTime(ts)}><span>{relTime(ts)}</span></Tooltip> },
  ];

  return (
    <Row gutter={16}>
      <Col span={10}>
        <Card size="small" title="工作流" extra={<Button size="small" onClick={() => load(false)}>刷新</Button>}>
          <Table
            rowKey="id"
            size="small"
            loading={loading}
            columns={wfCols}
            dataSource={rows}
            pagination={false}
            onRow={(r) => ({ onClick: () => setSelectedId(r.id), style: { cursor: 'pointer' } })}
            rowClassName={(r) => (r && r.id === selectedId ? 'ant-table-row-selected' : '')}
          />
        </Card>
      </Col>
      <Col span={14}>
        {selectedId
          ? <WorkflowDetail workflowId={selectedId} onNotice={onNotice} onMutated={() => load(true)} />
          : <Card size="small"><Empty description="点击左侧工作流查看详情" /></Card>}
      </Col>
    </Row>
  );
}
