import { memo, useMemo } from 'react';
import { Background, Controls, Handle, MiniMap, Position, ReactFlow } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { Tag } from 'antd';
import { graph, statusOf, STATES, COLORS } from './model.js';

const StepNode = memo(function StepNode({ data, selected }) {
  const status = statusOf(data.group);
  return <div className={`brain-node brain-node-${status} ${selected || data.group?.active_visits?.length ? 'brain-node-selected' : ''}`}>
    <Handle type="target" position={Position.Left} isConnectable={!!data.connectable} />
    <div className="brain-node-kind">{data.kind}{data.step?.foreach ? ' · 批量' : ''}{data.step?.when ? ' · 条件' : ''}</div>
    <strong>{data.label}</strong>
    <div className="brain-node-contract">{data.description}</div>
    {data.step && <div><Tag color={COLORS[status]}>{STATES[status]}</Tag>{data.group?.total > 0 && <small>{data.group.total} 轮</small>}</div>}
    <Handle type="source" position={Position.Right} isConnectable={!!data.connectable} />
  </div>;
});
const nodeTypes = { brainStep: StepNode };
export function PlanCanvas({ plan, groups, capabilities, mode = 'execution', selected, onSelect, positions = {}, viewport, onPositions, onViewport, onConnect }) {
  const projection = useMemo(() => graph(plan, groups, mode === 'ontology', capabilities), [plan, groups, mode, capabilities]);
  const nodes = useMemo(() => projection.nodes.map((node) => ({ ...node, data: { ...node.data, connectable: !!onConnect && !!node.data.step }, position: positions[node.id] || node.position, selected: node.id === selected })), [projection, selected, positions, onConnect]);
  return <div className="brain-canvas" aria-label={mode === 'ontology' ? '本体关系画布' : '步骤执行画布'}>
    <ReactFlow nodes={nodes} edges={projection.edges} nodeTypes={nodeTypes} nodesDraggable={!!onPositions} nodesConnectable={!!onConnect} onConnect={onConnect} isValidConnection={({ source, target }) => (plan.instances || []).some((s) => s.id === source) && (plan.instances || []).some((s) => s.id === target)} onNodesChange={(changes) => { const moved = changes.filter((c) => c.type === 'position' && c.position); if (moved.length) onPositions?.(Object.fromEntries(moved.map((c) => [c.id, c.position]))); }} onMoveEnd={(_, value) => onViewport?.(value)} defaultViewport={viewport || undefined} onNodeClick={(_, node) => onSelect?.(node.id)} fitView={!viewport} minZoom={0.2} maxZoom={2} onlyRenderVisibleElements>
      <Background gap={22} color="#d9e2ed" /><Controls showInteractive={false} /><MiniMap pannable zoomable />
    </ReactFlow>
  </div>;
}
