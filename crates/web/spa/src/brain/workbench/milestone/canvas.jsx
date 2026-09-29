import { Background, Controls, Handle, MarkerType, MiniMap, Position, ReactFlow, applyNodeChanges } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { Button, Tag } from 'antd';
import { useEffect, useMemo, useRef, useState } from 'react';
import { capabilityLabel, capabilityName, capabilityTask, groups } from './model.js';
import './style.css';

function LayerCard({ data, selected }) {
  return <article className={`brain-layer-box ${selected ? 'selected' : ''}`}>
    <Handle id="forward-in" type="target" position={Position.Top} isConnectable={false} />
    <header><Tag>第 {data.index + 1} 层</Tag><strong>{data.layer.title || '新里程碑'}</strong>{data.status && <Tag>{data.status}</Tag>}</header>
    <p>{data.layer.task || '点击配置里程碑要做什么'}</p>
    {data.editable && <Button className="nodrag" size="small" onClick={(event) => { event.stopPropagation(); data.onAddNode(data.layer.layer_id); }}>＋ 并行执行节点</Button>}
    <Handle id="forward-out" type="source" position={Position.Bottom} isConnectable={false} />
  </article>;
}
function ExecutionCard({ data, selected }) {
  return <article className={`brain-execution-node ${selected ? 'selected' : ''}`}>
    <strong>{data.capability ? capabilityName(data.capability) : '选择泛化能力'}</strong>
    <p>{data.capability ? capabilityTask(data.capability) : '点击绑定能力'}</p>
    <span title={data.capabilityLabel}>{data.capabilityLabel || '选择泛化能力'}</span>
    {data.status && <Tag>{data.status}</Tag>}
  </article>;
}
const nodeTypes = { layer: LayerCard, execution: ExecutionCard };
const EMPTY = {};
const EMPTY_CAPABILITIES = [];
const layerFlowId = (id) => `layer:${id}`;
const executionFlowId = (id) => `execution:${id}`;
const originalFlowId = (id) => id.slice(id.indexOf(':') + 1);
const capabilityLabelFor = (node, capabilities) => {
  const capability = capabilities.find((item) => (item.capability_id || item.id) === node.capability_id);
  return capability ? capabilityLabel(capability) : node.capability_id;
};
export function MilestoneCanvas({ plan, capabilities = EMPTY_CAPABILITIES, selection, onSelect, onAddLayer, onAddNode, positions = EMPTY, onPositions, statuses = EMPTY, layerStatuses = EMPTY }) {
  const editable = !!onAddLayer;
  const flow = useRef(null);
  const container = useRef(null);
  const levelGroups = groups(plan);
  const projected = useMemo(() => {
    let y = 40;
    const widest = Math.max(420, ...levelGroups.map((group) => Math.min(3, Math.max(1, group.length)) * 228 + 48));
    return plan.layers.flatMap((layer, index) => {
      const children = levelGroups[index];
      const columns = Math.min(3, Math.max(1, children.length));
      const rows = Math.ceil(children.length / columns);
      const width = Math.max(420, columns * 228 + 48), height = 150 + rows * 140;
      const position = positions[layer.layer_id] || { x: (widest - width) / 2 + 40, y };
      y += height + 120;
      return [{ id: layerFlowId(layer.layer_id), type: 'layer', position, style: { width, height }, selected: selection?.type === 'layer' && selection.id === layer.layer_id,
        data: { layer, index, editable, onAddNode, status: layerStatuses[layer.layer_id] } },
      ...children.map((node, column) => ({ id: executionFlowId(node.node_id), type: 'execution', parentId: layerFlowId(layer.layer_id), extent: 'parent',
        position: { x: 24 + (column % columns) * 228, y: 132 + Math.floor(column / columns) * 140 },
        style: { width: 210, height: 120 }, selected: selection?.type === 'node' && selection.id === node.node_id,
        data: { node, capability: capabilities.find((item) => (item.capability_id || item.id) === node.capability_id), capabilityLabel: capabilityLabelFor(node, capabilities), status: statuses[node.node_id] }, draggable: false }))];
    });
  }, [plan, levelGroups.length, capabilities, selection, editable, positions, onAddNode, statuses, layerStatuses]);
  const [nodes, setNodes] = useState(projected);
  useEffect(() => setNodes(projected), [projected]);
  useEffect(() => { const timer = setTimeout(() => flow.current?.fitView({ padding: 0.18, maxZoom: 1, duration: 180 }), 80); return () => clearTimeout(timer); }, [plan.layers.length, plan.nodes.length]);
  useEffect(() => {
    if (!container.current) return undefined;
    let timer;
    const observer = new ResizeObserver(() => { clearTimeout(timer); timer = setTimeout(() => flow.current?.fitView({ padding: 0.18, maxZoom: 1, duration: 180 }), 80); });
    observer.observe(container.current);
    return () => { clearTimeout(timer); observer.disconnect(); };
  }, []);
  const edges = plan.layers.slice(1).map((layer, index) => ({
    id: `sequence:${plan.layers[index].layer_id}:${layer.layer_id}`,
    source: layerFlowId(plan.layers[index].layer_id), target: layerFlowId(layer.layer_id),
    sourceHandle: 'forward-out', targetHandle: 'forward-in', type: 'smoothstep',
    selectable: false, ariaLabel: `第 ${index + 1} 层 → 第 ${index + 2} 层`,
    style: { stroke: '#87a3c2', strokeWidth: 2 }, markerEnd: { type: MarkerType.ArrowClosed },
  }));
  return <div ref={container} className={`brain-milestone-canvas ${editable ? 'is-editable' : ''}`} aria-label="里程碑编辑画布">
    {editable && <div className="brain-milestone-tools"><Button onClick={onAddLayer}>＋ 里程碑</Button><Button onClick={() => onPositions?.({})}>整理布局</Button><span>大脑根据执行结果决定前进或回到已执行的里程碑</span></div>}
    <div className="brain-milestone-flow"><ReactFlow onInit={(instance) => { flow.current = instance; }} nodes={nodes} edges={edges} nodeTypes={nodeTypes} nodesDraggable={editable} nodesConnectable={false} fitView minZoom={0.15} maxZoom={2}
      onNodesChange={(changes) => setNodes((old) => applyNodeChanges(changes, old))}
      onNodeClick={(_, node) => onSelect?.({ type: node.type === 'layer' ? 'layer' : 'node', id: originalFlowId(node.id) })}
      onNodeDragStop={(_, node) => { if (node.type === 'layer') onPositions?.({ ...positions, [originalFlowId(node.id)]: node.position }); }}>
      <Background /><Controls showInteractive={false} /><MiniMap pannable zoomable />
    </ReactFlow></div>
    {!plan.layers.length && <div className="brain-milestone-empty"><h3>从第一个里程碑开始</h3><p>在画布配置里程碑和并行节点</p><Button type="primary" onClick={onAddLayer}>添加第一个里程碑</Button></div>}
  </div>;
}
