import { Background, Controls, Handle, Position, ReactFlow } from '@xyflow/react';
import { Alert, Descriptions, Typography } from 'antd';
import { useMemo, useState } from 'react';
import { foldStepStates, frameToEvent, graphFromSpec, outputPreview } from '../dagProjection.js';
function Step({ data }) {
  return <div className={`dag-node dag-node--${data.status || 'pending'}`}><Handle type="target" position={Position.Left} /><strong className="dag-node-name">{data.label}</strong><div>{data.kindType} · {data.status}</div><Handle type="source" position={Position.Right} /></div>;
}
const types = { dagStep: Step };
export function DagProcess({ spec, frames = [], events: suppliedEvents, selectedId, onSelect, showInspector = true, snapshot }) {
  const [localId, setLocalId] = useState(null);
  const events = useMemo(() => suppliedEvents || frames.map(frameToEvent).filter(Boolean), [frames, suppliedEvents]);
  const graph = useMemo(() => {
    const states = foldStepStates(events);
    for (const step of snapshot?.steps || []) {
      if (['done', 'error', 'cancelled'].includes(step.status) || !states.has(step.name)) states.set(step.name, { ...states.get(step.name), status: step.status, error: step.error });
    }
    return graphFromSpec(spec, states);
  }, [spec, events, snapshot]);
  const selected = graph.nodes.find((n) => n.id === (selectedId ?? localId))?.data;
  return <><div className="dag-detail-graph" style={{ height: 400 }}><ReactFlow nodes={graph.nodes} edges={graph.edges} nodeTypes={types} onNodeClick={(_, node) => { setLocalId(node.id); onSelect?.(node.id); }} fitView nodesDraggable={false} nodesConnectable={false}><Background /><Controls showInteractive={false} /></ReactFlow></div>
    {showInspector && selected && <div><Typography.Text strong>{selected.label}</Typography.Text><Descriptions column={1} size="small" items={[{ key: 'status', label: '状态', children: selected.status }, { key: 'kind', label: '类型', children: selected.kindType }]} />{selected.error && <Alert type="error" title={selected.error} />}<pre className="brain-json">{outputPreview(selected.output)}</pre></div>}
  </>;
}
