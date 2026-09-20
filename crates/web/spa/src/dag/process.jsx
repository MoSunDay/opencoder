import { Background, Controls, Handle, Position, ReactFlow } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useMemo } from 'react';
import { graphFromSpec } from '../dagProjection.js';
import { progressLabel } from './dynamic/model.js';
import { statusLabel } from '../ui/statusTag.jsx';

function Step({ data }) {
  const label = data.status === 'pending' ? '待执行' : data.status === 'skipped' ? '未执行' : statusLabel(data.status);
  return <div className={`dag-node dag-node--${data.status}`}>
    <Handle type="target" position={Position.Left} /><strong className="dag-node-name">{data.label}</strong>
    <div>{data.kindType} · {label}</div>
    {data.kindType === 'dynamic' && <div>{progressLabel(data.instances)}</div>}
    {data.error && <div className="dag-node-error" title={data.error}>{data.error}</div>}
    <Handle type="source" position={Position.Right} />
  </div>;
}
const types = { dagStep: Step };

export function DagProcess({ spec, snapshot, selectedId, onSelect }) {
  const graph = useMemo(() => {
    const ended = ['done', 'error', 'cancelled'].includes(snapshot?.execution_status);
    const states = new Map((snapshot?.steps || []).map((step) => [step.name,
      ended && step.status === 'pending' ? { ...step, status: 'skipped' } : step]));
    return graphFromSpec(spec, states);
  }, [spec, snapshot]);
  const nodes = useMemo(() => graph.nodes.map((node) => ({ ...node, selected: node.id === selectedId })), [graph, selectedId]);
  return <div className="dag-detail-graph"><ReactFlow nodes={nodes} edges={graph.edges} nodeTypes={types}
    onNodeClick={(_, node) => onSelect?.(node.id)} fitView nodesDraggable={false} nodesConnectable={false}>
    <Background /><Controls showInteractive={false} />
  </ReactFlow></div>;
}
