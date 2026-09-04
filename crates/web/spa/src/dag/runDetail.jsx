// runDetail.jsx — single run view: LEFT live graph (React Flow + dagre
// layout, statuses projected from the event fold), RIGHT reverse-chron event
// feed. Live updates come from GET /api/dag/runs/:id/events through the
// shared sse.js openStream (signed SSE, replay-then-live). On run_finished
// the final status/error is applied to the header row, the runs table is
// refreshed (onFinished) and the stream is closed.

import { Background, Controls, Handle, MarkerType, Position, ReactFlow } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { Alert, Button, Card, Descriptions, Space, Spin, Tag, Typography } from 'antd';
import { useEffect, useMemo, useRef, useState } from 'react';
import { apiGet } from '../api.js';
import { absTime } from '../format.js';
import { openStream } from '../sse.js';
import {
  foldStepStates,
  frameToEvent,
  graphFromSpec,
  outputPreview,
  runStatusLabel,
  STEP_RUNNING,
} from '../dagProjection.js';
import { RunStatusTag, NodeBadge } from './runBits.jsx';

const { Text } = Typography;

const KIND_META = {
  run_started: { label: '运行开始', color: 'blue' },
  step_started: { label: '步骤开始', color: 'processing' },
  step_done: { label: '步骤完成', color: 'green' },
  run_finished: { label: '运行结束', color: 'gold' },
};

const STREAM_LABEL = {
  connecting: '连接中',
  open: '连接中',
  live: '实时',
  reconnecting: '重连中',
  closed: '已结束',
  failed: '连接失败',
};

/// Custom React Flow node — module-level for a stable nodeTypes identity.
function DagStepNode({ data, selected }) {
  const status = (data && data.status) || 'pending';
  return (
    <div className={'dag-node dag-node--' + status + (selected ? ' dag-node--selected' : '')}>
      <Handle type="target" position={Position.Left} isConnectable={false} />
      <div className="dag-node-title">
        <span className="dag-node-name">{data && data.label}</span>
        <span className="dag-node-kind">{data && data.kindType}</span>
      </div>
      <div className="dag-node-status">{stepStatusLabel(status)}</div>
      <Handle type="source" position={Position.Right} isConnectable={false} />
    </div>
  );
}

const nodeTypes = { dagStep: DagStepNode };

const STEP_LABEL = { pending: '待执行', running: '执行中', done: '已完成', error: '失败', skipped: '未执行' };
function stepStatusLabel(status) {
  return STEP_LABEL[status] || status;
}

/// One feed row: kind badge, step, time, payload preview.
function EventRow({ ev }) {
  const meta = KIND_META[ev.kind] || { label: ev.kind, color: 'default' };
  let extra = '';
  if (ev.kind === 'run_started') {
    extra = ev.payload && ev.payload.node_id ? '节点 ' + ev.payload.node_id : '';
  } else if (ev.kind === 'step_done') {
    const ok = !(ev.payload && ev.payload.ok === false);
    extra = ok ? '成功' : '失败' + (ev.payload && ev.payload.error ? ': ' + ev.payload.error : '');
  } else if (ev.kind === 'run_finished') {
    extra = runStatusLabel(ev.payload && ev.payload.status) + (ev.payload && ev.payload.error ? ' · ' + ev.payload.error : '');
  }
  const preview = ev.kind === 'step_done' ? outputPreview(ev.payload && ev.payload.output, 300) : '';
  return (
    <div className="dag-event">
      <div className="dag-event-head">
        <Tag color={meta.color} style={{ marginInlineEnd: 6 }}>{meta.label}</Tag>
        {ev.step ? <Text strong style={{ fontSize: 12 }}>{ev.step}</Text> : null}
        <Text type="secondary" style={{ fontSize: 12, marginLeft: 'auto' }}>
          {ev.at_ms ? absTime(ev.at_ms) : '-'}
        </Text>
      </div>
      {extra ? <div className="dag-event-extra">{extra}</div> : null}
      {preview ? <pre className="dag-event-pre">{preview}</pre> : null}
    </div>
  );
}

export function RunDetail({ run, onNotice, onClose, onFinished }) {
  const [current, setCurrent] = useState(run); // local copy, finalized on run_finished
  const [spec, setSpec] = useState(null);
  const [specError, setSpecError] = useState('');
  const [events, setEvents] = useState([]); // ascending by seq (arrival for unpersisted)
  const [streamStatus, setStreamStatus] = useState('connecting');
  const [selected, setSelected] = useState(null); // clicked step node data
  const streamRef = useRef(null);
  const finishedRef = useRef(false);
  const alive = useRef(true);

  // The run's spec snapshot lives on the definition (runs carry dag_id).
  useEffect(() => {
    alive.current = true;
    const id = current && current.id;
    const dagId = current && current.dag_id;
    if (!id || !dagId) {
      return undefined;
    }
    apiGet('/api/dag/defs/' + encodeURIComponent(dagId))
      .then((def) => {
        if (alive.current) {
          setSpec(def && def.spec ? def.spec : null);
        }
      })
      .catch((e) => {
        if (alive.current) {
          setSpecError('加载工作流定义失败: ' + (e && e.message));
        }
      });
    return () => {
      alive.current = false;
    };
  }, [current && current.id, current && current.dag_id]);

  // Live event stream: replay first, then live frames. Fold is a pure
  // reducer (dagProjection.foldStepStates) over the appended event list.
  useEffect(() => {
    const id = current && current.id;
    if (!id) {
      return undefined;
    }
    const applyFinal = (payload, atMs) => {
      if (finishedRef.current) {
        return;
      }
      finishedRef.current = true;
      setCurrent((prev) => ({
        ...prev,
        status: (payload && payload.status) || prev.status,
        error: payload && payload.error ? payload.error : prev && prev.error,
        finished_at: Number.isFinite(atMs) ? atMs : prev && prev.finished_at,
      }));
      if (onFinished) {
        onFinished();
      }
      // Close after applying: the run is terminal, the stream has no more news.
      if (streamRef.current) {
        streamRef.current.abort();
      }
    };
    streamRef.current = openStream({
      path: '/api/dag/runs/' + encodeURIComponent(id) + '/events',
      after: 0,
      onStatus: (st) => {
        if (alive.current) {
          setStreamStatus(st);
        }
      },
      onFrame: (frame) => {
        const ev = frameToEvent(frame);
        if (!ev) {
          return;
        }
        if (alive.current) {
          setEvents((prev) => {
            if (Number.isFinite(ev.seq)) {
              const lastSeq = prev.length ? prev[prev.length - 1].seq : null;
              if (Number.isFinite(lastSeq) && ev.seq <= lastSeq) {
                return prev; // replay repeat (transport already dedups; belt+braces)
              }
            }
            return [...prev, ev];
          });
        }
        if (ev.kind === 'run_finished') {
          applyFinal(ev.payload, ev.at_ms);
        }
      },
    });
    return () => {
      if (streamRef.current) {
        streamRef.current.abort();
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current && current.id]);

  const stepStates = useMemo(() => foldStepStates(events), [events]);
  const graph = useMemo(
    () => (spec ? graphFromSpec(spec, stepStates) : { nodes: [], edges: [] }),
    [spec, stepStates],
  );
  const rfEdges = useMemo(
    () =>
      graph.edges.map((e) => {
        const targetNode = graph.nodes.find((n) => n.id === e.target);
        const running = targetNode && targetNode.data && targetNode.data.status === STEP_RUNNING;
        return { ...e, animated: !!running, markerEnd: { type: MarkerType.ArrowClosed } };
      }),
    [graph],
  );
  const feed = useMemo(() => [...events].slice(-200).reverse(), [events]);

  return (
    <Space direction="vertical" size={12} style={{ width: '100%' }}>
      <Space wrap>
        <Button size="small" onClick={onClose}>← 返回运行列表</Button>
        <Text strong>运行 {String(current.id || '').slice(0, 8)}</Text>
        <RunStatusTag status={current.status} />
        <NodeBadge nodeId={current.node_id} status={current.status} />
        <Text type="secondary">创建于 {absTime(current.created_at)}</Text>
        <Tag color={streamStatus === 'live' ? 'green' : streamStatus === 'failed' ? 'red' : 'default'}>
          事件流: {STREAM_LABEL[streamStatus] || streamStatus}
        </Tag>
      </Space>
      {current.error ? <Alert type="error" showIcon message={current.error} /> : null}
      {specError ? <Alert type="warning" showIcon message={specError} /> : null}
      <div className="dag-detail">
        <div className="dag-detail-graph">
          {spec ? (
            <ReactFlow
              nodes={graph.nodes}
              edges={rfEdges}
              nodeTypes={nodeTypes}
              onNodeClick={(_, node) => setSelected(node.data)}
              fitView
              minZoom={0.2}
              nodesDraggable={false}
              nodesConnectable={false}
              proOptions={{ hideAttribution: false }}
            >
              <Background gap={18} size={1} />
              <Controls showInteractive={false} />
            </ReactFlow>
          ) : (
            <div className="dag-detail-empty">
              <Spin />
              <Text type="secondary" style={{ marginTop: 8 }}>加载工作流图中…</Text>
            </div>
          )}
        </div>
        <div className="dag-detail-side">
          {selected ? (
            <Card
              size="small"
              title={'步骤 · ' + selected.label}
              extra={<Button size="small" type="text" onClick={() => setSelected(null)}>关闭</Button>}
            >
              <Descriptions size="small" column={1}>
                <Descriptions.Item label="状态">{stepStatusLabel(selected.status)}</Descriptions.Item>
                <Descriptions.Item label="类型">{selected.kindType || '-'}</Descriptions.Item>
                <Descriptions.Item label="结束时间">{selected.at_ms ? absTime(selected.at_ms) : '—'}</Descriptions.Item>
              </Descriptions>
              {selected.error ? <Alert type="error" style={{ marginTop: 8 }} message={selected.error} /> : null}
              {selected.output ? (
                <pre className="dag-event-pre">{outputPreview(selected.output)}</pre>
              ) : (
                <Text type="secondary" style={{ fontSize: 12 }}>暂无输出快照</Text>
              )}
            </Card>
          ) : null}
          <div className="dag-feed">
            {feed.length ? (
              feed.map((ev, i) => <EventRow key={(Number.isFinite(ev.seq) ? 's' + ev.seq : 'i' + i) + ':' + ev.kind} ev={ev} />)
            ) : (
              <Text type="secondary" style={{ padding: 12 }}>暂无事件，等待节点上报…</Text>
            )}
          </div>
        </div>
      </div>
    </Space>
  );
}
