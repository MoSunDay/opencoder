import { memo, useMemo } from 'react';
import { Background, Controls, Handle, MiniMap, Position, ReactFlow } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { Tag } from 'antd';
import { graph, statusOf, STATES, COLORS } from './model.js';

const StepNode = memo(function StepNode({ data, selected }) {
  const status = statusOf(data.group);
  return <div className={`brain-node brain-node-${status} ${selected ? 'brain-node-selected' : ''}`}>
    <Handle type="target" position={Position.Left} isConnectable={false} />
    <div className="brain-node-kind">{data.kind}{data.step?.foreach ? ' · 批量' : ''}{data.step?.when ? ' · 条件' : ''}</div>
    <strong>{data.label}</strong>
    {data.ontology ? <div className="brain-node-contract">{data.step ? <><span>输入 {Object.keys(data.step.inputs || {}).join(' · ') || '无'}</span><span>输出 {data.step.output?.semantic || data.step.output?.type}</span></> : data.port?.schema?.semantic || data.port?.schema?.type}</div>
      : <div><Tag color={COLORS[status]}>{STATES[status]}</Tag>{data.group?.total > 1 && <small>{data.group.counts.succeeded || 0} / {data.group.total} 通过</small>}</div>}
    <Handle type="source" position={Position.Right} isConnectable={false} />
  </div>;
});
const nodeTypes = { brainStep: StepNode };
export function PlanCanvas({ plan, groups, mode = 'execution', selected, onSelect }) {
  const projection = useMemo(() => graph(plan, groups, mode === 'ontology'), [plan, groups, mode]);
  const nodes = useMemo(() => projection.nodes.map((node) => ({ ...node, selected: node.id === selected })), [projection, selected]);
  return <div className="brain-canvas" aria-label={mode === 'ontology' ? '本体关系画布' : '步骤执行画布'}>
    <ReactFlow nodes={nodes} edges={projection.edges} nodeTypes={nodeTypes} nodesDraggable={false} nodesConnectable={false} onNodeClick={(_, node) => onSelect?.(node.id)} fitView minZoom={0.2} maxZoom={2} onlyRenderVisibleElements>
      <Background gap={22} color="#d9e2ed" /><Controls showInteractive={false} /><MiniMap pannable zoomable />
    </ReactFlow>
  </div>;
}
