import { Background, Controls, Handle, MarkerType, MiniMap, Position, ReactFlow, applyNodeChanges } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { Button, Tag } from 'antd';
import { useEffect, useMemo, useRef, useState } from 'react';
import { groups } from './model.js';
import './style.css';

function LayerCard({ data, selected }) {
  return <article className={`brain-layer-box ${selected ? 'selected' : ''}`}>
    <Handle id="forward-in" type="target" position={Position.Top} isConnectable={data.editable} />
    <Handle id="return-in" type="target" position={Position.Right} isConnectable={data.editable} />
    <header><Tag>第 {data.index + 1} 层</Tag><strong>{data.layer.title || '新里程碑'}</strong>{data.status && <Tag>{data.status}</Tag>}</header>
    <p>{data.layer.objective || '点击配置里程碑目标与达成标准'}</p>
    {data.editable && <Button className="nodrag" size="small" onClick={(event) => { event.stopPropagation(); data.onAddNode(data.layer.layer_id); }}>＋ 并行执行节点</Button>}
    <Handle id="forward-out" type="source" position={Position.Bottom} isConnectable={data.editable} />
    <Handle id="return-out" type="source" position={Position.Left} isConnectable={data.editable} />
  </article>;
}
function ExecutionCard({ data, selected }) {
  return <article className={`brain-execution-node ${selected ? 'selected' : ''}`}>
    <strong>{data.node.title || '新执行节点'}</strong>
    <p>{data.node.objective || '点击配置执行任务'}</p>
    <span>{data.node.capability_id || '选择泛化能力'}</span>
    {data.status && <Tag>{data.status}</Tag>}
  </article>;
}
const nodeTypes = { layer: LayerCard, execution: ExecutionCard };
const EMPTY = {};
export function MilestoneCanvas({ plan, selection, onSelect, onConnect, onAddLayer, onAddNode, positions = EMPTY, onPositions, statuses = EMPTY, layerStatuses = EMPTY }) {
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
      return [{ id: layer.layer_id, type: 'layer', position, style: { width, height }, selected: selection?.type === 'layer' && selection.id === layer.layer_id,
        data: { layer, index, editable, onAddNode, status: layerStatuses[layer.layer_id] } },
      ...children.map((node, column) => ({ id: node.node_id, type: 'execution', parentId: layer.layer_id, extent: 'parent',
        position: { x: 24 + (column % columns) * 228, y: 132 + Math.floor(column / columns) * 140 },
        style: { width: 210, height: 120 }, selected: selection?.type === 'node' && selection.id === node.node_id,
        data: { node, status: statuses[node.node_id] }, draggable: false }))];
    });
  }, [plan, levelGroups.length, selection, editable, positions, onAddNode, statuses, layerStatuses]);
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
  const edges = plan.transitions.map((edge) => {
    const forward = plan.layers.findIndex((item) => item.layer_id === edge.to) > plan.layers.findIndex((item) => item.layer_id === edge.from);
    return { id: `transition:${edge.from}:${edge.to}`, source: edge.from, target: edge.to,
      sourceHandle: forward ? 'forward-out' : 'return-out', targetHandle: forward ? 'forward-in' : 'return-in',
      type: 'smoothstep', label: edge.condition || '填写扭转条件', selectable: true,
      style: { stroke: forward ? '#87a3c2' : '#d77948', strokeWidth: 2 }, markerEnd: { type: MarkerType.ArrowClosed } };
  });
  return <div ref={container} className={`brain-milestone-canvas ${editable ? 'is-editable' : ''}`} aria-label="里程碑编辑画布">
    {editable && <div className="brain-milestone-tools"><Button onClick={onAddLayer}>＋ 里程碑</Button><Button onClick={() => onPositions?.({})}>整理布局</Button><span>连线描述可选流转；每层节点并行执行</span></div>}
    <div className="brain-milestone-flow"><ReactFlow onInit={(instance) => { flow.current = instance; }} nodes={nodes} edges={edges} nodeTypes={nodeTypes} nodesDraggable={editable} nodesConnectable={editable} fitView minZoom={0.15} maxZoom={2}
      onNodesChange={(changes) => setNodes((old) => applyNodeChanges(changes, old))}
      onNodeClick={(_, node) => onSelect?.({ type: node.type === 'layer' ? 'layer' : 'node', id: node.id })}
      onEdgeClick={(_, edge) => onSelect?.({ type: 'transition', id: `${edge.source}:${edge.target}` })}
      onConnect={(edge) => onConnect?.(edge.source, edge.target)}
      onNodeDragStop={(_, node) => { if (node.type === 'layer') onPositions?.({ ...positions, [node.id]: node.position }); }}>
      <Background /><Controls showInteractive={false} /><MiniMap pannable zoomable />
    </ReactFlow></div>
    {!plan.layers.length && <div className="brain-milestone-empty"><h3>从第一个里程碑开始</h3><p>在画布配置里程碑、并行节点和扭转关系</p><Button type="primary" onClick={onAddLayer}>添加第一个里程碑</Button></div>}
  </div>;
}
