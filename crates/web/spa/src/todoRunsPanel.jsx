// todoRunsPanel.jsx — TODO 工作流「运行」视图：左侧工作流列表（GET
// /api/todo/workflows?limit=50，3s 轮询——仅当仍有 running 时），点击行右侧
// 展开 items 表 + SSE 事件流（openStream 订阅 /events，事件名 = kind）。
// 流在 workflow_completed/workflow_failed 后服务器即关流，这里主动 abort，
// 避免 sse.js 把「干净关闭」当作断线去空重连。

import { Button, Card, Col, Drawer, Empty, Input, Row, Space, Table, Tooltip, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from './api.js';
import { ExecutionDetail } from './fleet/detail.jsx';
import { StatusTag } from './ui/statusTag.jsx';
import { MONO_VAR } from './ui/mono.js';
import { TimeText } from './ui/timeText.jsx';
import { tableLoading, tableRows } from './ui/tableLoading.js';
import { err } from './notice.js';
import { TodoWorkbench } from './todo/review/workbench.jsx';

const { Text } = Typography;

const POLL_MS = 3000;
/// SSE 终帧事件名（服务器随后关流）。
export const TERMINAL_KINDS = ['workflow_completed', 'workflow_failed'];

/// 工作流行上的节点执行状态 Tag（可能缺省 → 不渲染）。
function ExecutionStatusTag({ status }) {
  return status ? <StatusTag status={status} /> : null;
}

export function workflowActions(workflowStatus, executionStatus) {
  const closed = ['done', 'cancelled'].includes(executionStatus)
    || ['completed', 'failed'].includes(workflowStatus);
  return {
    interrupt: executionStatus
      ? ['pending', 'running', 'idle'].includes(executionStatus)
      : ['pending', 'running'].includes(workflowStatus),
    resume: executionStatus
      ? ['interrupted', 'error'].includes(executionStatus)
      : workflowStatus === 'suspended',
    cancel: !closed,
  };
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

function WorkflowDetail({workflowId,summary,onNotice,onMutated}) {
  const [executionOpen,setExecutionOpen]=useState(false);
  return <><Button style={{marginBottom:12}} onClick={()=>setExecutionOpen(true)}>执行详情</Button>
    <TodoWorkbench key={workflowId} id={workflowId} onMutated={onMutated}/>
    {executionOpen&&<ExecutionDetail id={workflowId} summary={{...summary,id:workflowId,kind:'todos'}} onClose={()=>setExecutionOpen(false)} onNotice={onNotice}/>}</>;
}

export function TodoRunsPanel({ onNotice, focusWorkflowId, onFocusConsumed }) {
  const [rows, setRows] = useState([]);
  const [selectedId, setSelectedId] = useState('');
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState('');
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
      if (alive.current && onNotice) {
        onNotice(err('获取工作流列表失败: ' + (e && e.message)));
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
      if (rowsRef.current.length) {
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
      render: (v) => <Tooltip title={v}><span style={{ fontFamily: MONO_VAR }}>{String(v || '').slice(0, 16)}…</span></Tooltip> },
    { title: '状态', key: 'status', width: 170,
      render: (_, row) => <Space size={4}><Tooltip title="节点执行状态"><span><ExecutionStatusTag status={row.execution_status} /></span></Tooltip>{row.detail_error ? null : <Tooltip title="工作流状态"><span><StatusTag status={row.status} /></span></Tooltip>}</Space> },
    { title: '更新时间', dataIndex: 'updated_at', key: 'updated_at', width: 110,
      render: (ts) => <TimeText ts={ts} /> },
  ];

  // 搜索框为受控组件：按工作流 ID/状态（忽略大小写）过滤本地列表，不入服务端。
  const query = search.trim().toLowerCase();
  const visible = query
    ? rows.filter((r) => [r.id, r.status, r.execution_status].some((v) => String(v || '').toLowerCase().includes(query)))
    : rows;

  return (
    <Row gutter={[16, 16]}>
      <Col span={24}>
        <Card size="small" title="工作流" extra={
          <Space size={8}>
            <Input.Search
              allowClear
              style={{ width: 200 }}
              placeholder="搜索工作流 ID / 状态"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              aria-label="todo-run-search"
            />
            <Button size="small" onClick={() => load(false)}>刷新</Button>
          </Space>
        }>
          <Table
            className="oc-todo-runs"
            rowKey="id"
            size="small"
            loading={tableLoading(loading)}
            columns={wfCols}
            dataSource={tableRows(loading, visible)}
            pagination={false}
            scroll={{ x: 'max-content' }}
            onRow={(r) => ({ onClick: () => setSelectedId(r.id), style: { cursor: 'pointer' } })}
            rowClassName={(r) => (r && r.id === selectedId ? 'oc-row-selected' : '')}
          />
        </Card>
      </Col>
      <Drawer open={!!selectedId} title="TODO 运行 Review" placement="right" size="100%" onClose={()=>setSelectedId('')} destroyOnHidden>
        {selectedId&&<WorkflowDetail workflowId={selectedId} summary={rows.find(row=>row.id===selectedId)} onNotice={onNotice} onMutated={()=>load(true)}/>}
      </Drawer>
    </Row>
  );
}
