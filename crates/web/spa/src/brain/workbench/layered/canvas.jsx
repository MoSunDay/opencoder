// canvas.jsx — the v4 layered canvas: the plan DAG drawn one column per layer,
// the active layer highlighted and every node coloured by its latest attempt.
import { Background, Controls, Handle, MarkerType, MiniMap, Position, ReactFlow } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { Empty, Space, Tag, Typography } from 'antd';
import { memo, useMemo } from 'react';
import { LAYERED_COLORS, LAYERED_STATUS, layerGraph, retryBadge } from './model.js';

const LayerNode = memo(function LayerNode({ data, selected }) {
  const badge = retryBadge(data);
  const classes = ['brain-layer-node', `brain-layer-node--${data.status}`];
  if (data.active) classes.push('brain-layer-node--active');
  if (selected) classes.push('brain-layer-node--selected');
  return <div className={classes.join(' ')}>
    <Handle type="target" position={Position.Top} isConnectable={false} />
    <div className="brain-layer-node-head">
      <span className="brain-layer-node-layer">层 {data.layer}</span>
      <Tag color={LAYERED_COLORS[data.status]}>{LAYERED_STATUS[data.status] || data.status}</Tag>
    </div>
    <strong className="brain-layer-node-title" title={data.title}>{data.title}</strong>
    <div className="brain-layer-node-meta">
      <Typography.Text type="secondary" ellipsis={{ tooltip: data.capabilityId }}>{data.capabilityId || '未绑定能力'}</Typography.Text>
      <span className="brain-layer-node-attempt" title={`尝试 ${badge.label}`}>尝试 {badge.label}</span>
    </div>
    {(badge.retrying || data.cancelRequested || !data.started) && <Space size={4} wrap>
      {badge.retrying && <Tag color="orange">重试 {badge.attempt - 1} 次</Tag>}
      {data.cancelRequested && <Tag color="volcano">已请求取消</Tag>}
      {!data.started && <Tag>未派发</Tag>}
    </Space>}
    {!!data.upstreamTitles?.length && <div className="brain-layer-node-upstream">上游：{data.upstreamTitles.join('、')}</div>}
    <Handle type="source" position={Position.Bottom} isConnectable={false} />
  </div>;
});

const nodeTypes = { layeredNode: LayerNode };

export function LayerCanvas({ view, selected = null, onSelect }) {
  const graph = useMemo(() => layerGraph(view), [view]);
  const nodes = useMemo(() => graph.nodes.map((node) => ({
    ...node, selected: node.id === selected,
    data: { ...node.data, upstreamTitles: node.data.upstreamTitles },
  })), [graph, selected]);
  const edges = useMemo(() => graph.edges.map((edge) => ({ ...edge, markerEnd: { type: MarkerType.ArrowClosed } })), [graph]);
  if (!graph.nodes.length) return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="该 v4 计划还没有节点画布" />;
  return <div className="brain-canvas brain-layer-canvas" aria-label="分层能力画布">
    <ReactFlow
      nodes={nodes} edges={edges} nodeTypes={nodeTypes} nodesDraggable={false} nodesConnectable={false}
      // No onlyRenderVisibleElements: the plan is bounded (<=256 nodes) and
      // viewport culling hides edges entirely while a container has no
      // measured size yet (jsdom, or the first browser frame).
      edgesFocusable={false} fitView minZoom={0.2} maxZoom={2}
      onNodeClick={(_, node) => onSelect?.(node.id, node.data.layer)}
    >
      <Background gap={22} color="#d9e2ed" /><Controls showInteractive={false} /><MiniMap pannable zoomable />
    </ReactFlow>
  </div>;
}
