// rounds.jsx — one v4 layer at a time: the layer decision (reason + evidence)
// and the per-node attempts behind GET .../layered/rounds/:round. Detail is
// fetched lazily when a layer is expanded and dropped whenever the view moves
// on (generation / last_event_seq), so a stale decision can never be shown.
import { Alert, Button, Collapse, Empty, Space, Spin, Tag, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../../api.js';
import { KIND_LABELS } from '../../../fleet/model.js';
import { LAYERED_COLORS, LAYERED_STATUS, activeLayer, barrier, decisionLabel, layerLabel, layerRows, roundDetail } from './model.js';

function NodeAttempt({ node, onExecution }) {
  return <article className="brain-operation-card brain-layer-attempt" data-node={node.nodeId}>
    <Space wrap>
      <Tag color={LAYERED_COLORS[node.status]}>{LAYERED_STATUS[node.status] || node.status}</Tag>
      <Tag>{KIND_LABELS[node.executionKind] || node.executionKind || '执行'}</Tag>
      <Typography.Text strong>{node.title}</Typography.Text>
      <Tag color={node.attempt > 1 ? 'orange' : undefined}>第 {node.attempt} 次尝试</Tag>
      {node.cancelRequested && <Tag color="volcano">已请求取消</Tag>}
    </Space>
    <Typography.Text type="secondary">能力 ID：{node.capabilityId || '未记录'}</Typography.Text>
    <Space wrap><Typography.Text type="secondary">执行 ID：</Typography.Text><Typography.Text code copyable>{node.executionId || '尚未创建'}</Typography.Text></Space>
    <Button disabled={!node.executionId || node.status === 'creating'} onClick={() => onExecution?.(node.executionId)}>查看运行明细</Button>

  </article>;
}

function LayerDetail({ detail, onExecution }) {
  return <>
    <div className="brain-round-reason">
      <Typography.Text strong>本层调度：{decisionLabel(detail.decision) || '调度记录缺失'}</Typography.Text>
      <Typography.Paragraph>{detail.reason || '未记录决策理由'}</Typography.Paragraph>
      {!!detail.evidence.length && <Space wrap><Typography.Text type="secondary">依据执行：</Typography.Text>{detail.evidence.map((id) => <Typography.Text key={id} code>{id}</Typography.Text>)}</Space>}
    </div>
    {detail.nodes.length
      ? <div className="brain-operation-list">{detail.nodes.map((node) => <NodeAttempt key={`${node.nodeId}:${node.attempt}`} node={node} onExecution={onExecution} />)}</div>
      : <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="该层没有节点记录" />}
  </>;
}

export function LayerRounds({ id, view, onExecution }) {
  const rows = layerRows(view); const progress = barrier(view); const active = activeLayer(view);
  const [open, setOpen] = useState(() => (active ? [String(active)] : []));
  const [details, setDetails] = useState({});
  const generation = `${view?.run?.generation ?? 0}:${view?.run?.last_event_seq ?? 0}`;
  const stamp = `${id}:${generation}`; const current = useRef(stamp); current.current = stamp;
  const load = useCallback(async (layer) => {
    setDetails((old) => ({ ...old, [layer]: { loading: true, error: '', detail: old[layer]?.detail || null } }));
    try {
      const raw = await apiGet(`/api/brain/runs/${encodeURIComponent(id)}/layered/rounds/${layer}`);
      if (current.current !== stamp) return;
      setDetails((old) => ({ ...old, [layer]: { loading: false, error: '', detail: roundDetail(raw) } }));
    } catch (error) {
      if (current.current !== stamp) return;
      setDetails((old) => ({ ...old, [layer]: { loading: false, error: error?.status === 404 ? '该层还没有决策明细' : error.message, detail: null } }));
    }
  }, [id, stamp]);
  useEffect(() => { setDetails({}); }, [generation]);
  useEffect(() => { if (active) setOpen((old) => (old.includes(String(active)) ? old : [...old, String(active)])); }, [active]);
  useEffect(() => {
    for (const key of open) {
      const layer = Number(key);
      if (layer > 0 && layer <= progress.dispatched && !details[layer]) load(layer);
    }
  }, [open, progress.dispatched, details, load]);
  if (!rows.length) return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="该计划没有可执行的分层" />;
  return <Collapse activeKey={open} onChange={setOpen} items={rows.map((row) => ({
    key: String(row.layer),
    label: <Space wrap>
      <Typography.Text strong>{layerLabel(row.layer)}</Typography.Text>
      <Tag color={LAYERED_COLORS[row.status]}>{LAYERED_STATUS[row.status] || row.status}</Tag>
      {row.active && <Tag color="blue">当前决策层</Tag>}
      <Typography.Text type="secondary">{row.nodes.length} 个节点 · {row.attempts} 次尝试</Typography.Text>
    </Space>,
    children: row.layer > progress.dispatched
      ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={`${layerLabel(row.layer)} 尚未派发`} />
      : <LayerRoundBody onExecution={onExecution} state={details[row.layer]} onReload={() => load(row.layer)} />,
  }))} />;
}

function LayerRoundBody({ state, onReload, onExecution }) {
  if (!state || state.loading) return <Spin />;
  if (state.error) return <Alert type="warning" showIcon title={state.error} action={<Button size="small" onClick={onReload}>重试</Button>} />;
  return <LayerDetail detail={state.detail} onExecution={onExecution} />;
}
