// runDetail.jsx — single run view: LEFT live graph (React Flow + dagre
// layout, statuses projected from the event fold), RIGHT reverse-chron event
// feed. Live updates come from GET /api/dag/runs/:id/events through the
// shared sse.js openStream (Bearer-authenticated SSE, replay-then-live). On run_finished
// the final status/error is applied to the header row, the runs table is
// refreshed (onFinished) and the stream is closed.

import { DagProcess } from './process.jsx';
import '@xyflow/react/dist/style.css';
import { Alert, Button, Card, Descriptions, Space, Spin, Tag, Typography } from 'antd';
import { useEffect, useMemo, useRef, useState } from 'react';
import { apiGet } from '../api.js';
import { absTime } from '../format.js';
import { useExecutionEvents } from '../ui/executionEvents/useExecutionEvents.js';
import { ExecutionLogs } from '../ui/executionEvents/executionLogs.jsx';
import {
  foldStepStates,
  frameToEvent,
  graphFromSpec,
  outputPreview,
} from '../dagProjection.js';
import { statusLabel } from '../ui/statusTag.jsx';
import { RunStatusTag, NodeBadge } from './runBits.jsx';
import { ExecutionDetail } from '../fleet/detail.jsx';

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
    extra = statusLabel(ev.payload && ev.payload.status) + (ev.payload && ev.payload.error ? ' · ' + ev.payload.error : '');
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
  const [selectedId, setSelected] = useState(null); // clicked step node data
  const [executionOpen, setExecutionOpen] = useState(false);
  const finishedRef = useRef(false);
  const alive = useRef(true);

  // The run's spec snapshot lives on the definition (runs carry dag_id).
  useEffect(() => {
    alive.current = true;
    const id = current && current.id;
    const dagId = current && current.dag_id;
    if (!id || (!dagId && !current.spec)) {
      return undefined;
    }
    (current.spec ? Promise.resolve({ spec: current.spec }) : apiGet('/api/dag/defs/' + encodeURIComponent(dagId)))
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

  const logs = useExecutionEvents({ id: current.id, status: current.status,
    onFrame: (frame) => {
      const ev = frameToEvent(frame);
      if (!ev) return;
      setEvents((prior) => [...prior, ev]);
      if (ev.kind === 'run_finished') {
        setCurrent((prior) => ({ ...prior, status: ev.payload.status || prior.status,
          error: ev.payload.error || '', finished_at: ev.at_ms }));
        if (!finishedRef.current) { finishedRef.current = true; onFinished?.(); }
      }
    },
  });
  const streamStatus = logs.connection;

  const stepStates = useMemo(() => foldStepStates(events), [events]);
  const graph = useMemo(
    () => (spec ? graphFromSpec(spec, stepStates) : { nodes: [], edges: [] }),
    [spec, stepStates],
  );
  const selected = graph.nodes.find((node) => node.id === selectedId)?.data;
  const feed = useMemo(() => [...events].slice(-200).reverse(), [events]);

  return (
    <Space orientation="vertical" size={12} style={{ width: '100%' }}>
      <Space wrap>
        <Button size="small" onClick={onClose}>← 返回运行列表</Button>
        <Button size="small" onClick={() => setExecutionOpen(true)}>执行详情与产物</Button>
        <Text strong>运行 {String(current.id || '').slice(0, 8)}</Text>
        <RunStatusTag status={current.status} />
        <NodeBadge nodeId={current.node_id} status={current.status} />
        <Text type="secondary">创建于 {absTime(current.created_at)}</Text>
        <Tag color={streamStatus === 'live' ? 'green' : streamStatus === 'failed' ? 'red' : 'default'}>
          事件流: {STREAM_LABEL[streamStatus] || streamStatus}
        </Tag>
      </Space>
      {current.error ? <Alert type="error" showIcon title={current.error} /> : null}
      {specError ? <Alert type="warning" showIcon title={specError} /> : null}
      <div className="dag-detail">
        <div className="dag-detail-graph">
          {spec ? (
            <DagProcess spec={spec} events={events} selectedId={selectedId} onSelect={setSelected} showInspector={false} />
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
              <Descriptions size="small" column={1} items={[
                { key: 'status', label: '状态', children: stepStatusLabel(selected.status) },
                { key: 'kind', label: '类型', children: selected.kindType || '-' },
                { key: 'finished', label: '结束时间', children: selected.at_ms ? absTime(selected.at_ms) : '—' },
              ]} />
              {selected.error ? <Alert type="error" style={{ marginTop: 8 }} title={selected.error} /> : null}
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
      <ExecutionLogs id={current.id} {...logs} steps={(spec?.steps || []).map((step) => step.name)} step={selectedId || ''} onStepChange={(name) => setSelected(name || null)} />
      {executionOpen && <ExecutionDetail id={current.id} summary={{ id: current.id, kind: 'dag', node_id: current.node_id, status: current.status, created_at: current.created_at }} onClose={() => setExecutionOpen(false)} onNotice={onNotice} />}
    </Space>
  );
}
