import { Background, Controls, Handle, MarkerType, MiniMap, Position, ReactFlow, applyNodeChanges } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { Button, Tag } from 'antd';
import { useEffect, useMemo, useRef, useState } from 'react';
import { groups } from './model.js';
import './style.css';
function MilestoneNode({ data, selected }) {
  return <article className={`brain-milestone-node ${selected ? 'selected' : ''}`}>
    <Handle id="forward-in" type="target" position={Position.Top} isConnectable={data.editable} />
    <Tag>第 {data.node.layer} 层</Tag><strong>{data.node.title || '新里程碑'}</strong>
    <p>{data.node.objective || '点击配置目标、达成标准和能力'}</p>
    <span>{data.node.capability_ids.length} 个挂载能力</span>
    {data.status && <Tag>{data.status}</Tag>}
    {data.editable && <Button className="nodrag" size="small" onClick={(event) => { event.stopPropagation(); data.onParallel(data.node.layer); }}>＋ 同层里程碑</Button>}
    <Handle id="forward-out" type="source" position={Position.Bottom} isConnectable={data.editable} />
  </article>;
}
const EMPTY = {};
function BarrierNode() {
  return <div className="brain-milestone-barrier"><Handle id="in" type="target" position={Position.Top} isConnectable={false} />全层结束 · 大脑判断<Handle id="out" type="source" position={Position.Bottom} isConnectable={false} /></div>;
}
const nodeTypes = { milestone: MilestoneNode, barrier: BarrierNode };
export function MilestoneCanvas({ plan, selected, onSelect, onMove, onParallel, onAddLayer, positions = EMPTY, onPositions, statuses = EMPTY }) {
  const editable = !!onAddLayer;
  const flow = useRef(null);
  const container = useRef(null);
  useEffect(() => { const timer = setTimeout(() => flow.current?.fitView({ padding: 0.2, maxZoom: 1, duration: 180 }), 60); return () => clearTimeout(timer); }, [plan.nodes.length]);
  useEffect(() => {
    const element = container.current;
    if (!element) return;
    let timer;
    const observer = new ResizeObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(() => flow.current?.fitView({ padding: 0.2, maxZoom: 1, duration: 180 }), 80);
    });
    observer.observe(element);
    return () => { clearTimeout(timer); observer.disconnect(); };
  }, []);
  const projected = useMemo(() => groups(plan).flatMap((layer, index) => layer.map((node, column) => ({
    id: node.node_id, type: 'milestone', selected: node.node_id === selected,
    position: positions[node.node_id] || { x: column * 310, y: index * 280 },
    data: { node, editable, onParallel, status: statuses[node.node_id] },
  }))), [plan, selected, positions, onParallel, statuses, editable]);
  const [nodes, setNodes] = useState(projected);
  useEffect(() => setNodes(projected), [projected]);
  const edges = [];
  const levels = groups(plan);
  const barriers = [];
  const forward = (id, source, target, sourceHandle = 'forward-out', targetHandle = 'forward-in') => ({
    id, source, target, sourceHandle, targetHandle, type: 'smoothstep', selectable: false,
    style: { stroke: '#91a9c4', strokeDasharray: '5 4' }, markerEnd: { type: MarkerType.ArrowClosed },
  });
  for (let i = 1; i < levels.length; i += 1) {
    if (levels[i - 1].length === 1 && levels[i].length === 1) {
      edges.push(forward(`forward:${i}`, levels[i - 1][0].node_id, levels[i][0].node_id));
      continue;
    }
    const id = `barrier:${i}:${plan.nodes.map((n) => n.node_id).join(':')}`;
    const previous = nodes.filter((n) => n.data.node.layer === i);
    const x = previous.reduce((sum, n) => sum + n.position.x, 0) / previous.length + 71;
    barriers.push({ id, type: 'barrier', position: { x, y: (i - 1) * 280 + 230 }, data: {}, draggable: false, selectable: false });
    for (const node of levels[i - 1]) edges.push(forward(`to:${node.node_id}`, node.node_id, id, 'forward-out', 'in'));
    for (const node of levels[i]) edges.push(forward(`from:${node.node_id}`, id, node.node_id, 'out', 'forward-in'));
  }
  return <div ref={container} className={`brain-milestone-canvas ${editable ? "is-editable" : ""}`} aria-label="里程碑编辑画布">
    {editable && <div className="brain-milestone-tools"><Button onClick={onAddLayer}>＋ 新增层级</Button><Button onClick={() => onPositions?.({})}>整理布局</Button><span>同层并行 · 大脑可回退到已执行的层</span></div>}
    <div className="brain-milestone-flow"><ReactFlow onInit={(instance) => { flow.current = instance; }} nodes={[...nodes, ...barriers]} edges={edges} nodeTypes={nodeTypes} nodesDraggable={editable} nodesConnectable={false} fitView minZoom={0.15} maxZoom={2}
      onNodesChange={(changes) => setNodes((old) => applyNodeChanges(changes, old))}
      onNodeClick={(_, node) => node.type === 'milestone' && onSelect?.(node.id)}
      onNodeDragStop={(_, node) => { onMove?.(node.id, Math.max(1, Math.min(levels.length + 1, Math.round(node.position.y / 280) + 1)), node.position); }}>
      <Background /><Controls showInteractive={false} /><MiniMap pannable zoomable />
    </ReactFlow></div>
    {!nodes.length && <div className="brain-milestone-empty"><h3>从第一个里程碑开始</h3><p>在画布配置里程碑目标与能力</p><Button type="primary" onClick={onAddLayer}>添加第一个里程碑</Button></div>}
  </div>;
}
