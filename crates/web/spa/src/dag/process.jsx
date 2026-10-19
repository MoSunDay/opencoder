import { Background, Controls, Handle, Position, ReactFlow, useNodesState } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useEffect, useMemo } from 'react';
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
  const [nodes, setNodes, onNodesChange] = useNodesState(graph.nodes);
  useEffect(() => {
    setNodes((previous) => {
      const measured = new Map(previous.map((node) => [node.id, node.measured]));
      // Polling changes status, not the measured box. Preserve dimensions even
      // while a tab is hidden; ResizeObserver will update cards that grow.
      return graph.nodes.map((node) => ({ ...node, measured: measured.get(node.id), selected: node.id === selectedId }));
    });
  }, [graph, selectedId, setNodes]);
  // After measurement, let React Flow retain the real handle positions too.
  const measuredNodes = useMemo(() => nodes.map((node) => node.measured?.height == null
    ? node : { ...node, handles: undefined }), [nodes]);
  return <div className="dag-detail-graph"><ReactFlow nodes={measuredNodes} edges={graph.edges} nodeTypes={types}
    onNodesChange={onNodesChange} onNodeClick={(_, node) => onSelect?.(node.id)} fitView nodesDraggable={false} nodesConnectable={false}>
    <Background /><Controls showInteractive={false} />
  </ReactFlow></div>;
}
