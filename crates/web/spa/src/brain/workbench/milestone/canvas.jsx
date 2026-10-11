import { EditButton, useCanEdit } from '../../../ui/permissions.jsx';
import { Background, Controls, Handle, MarkerType, MiniMap, Position, ReactFlow, useUpdateNodeInternals } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { Tag } from 'antd';
import { useEffect, useMemo, useRef, useState } from 'react';
import { capabilityLabel, capabilityName, capabilityTask, groups } from './model.js';
import { changePresentation, committedPositions, displayNodes, flowSelection } from './canvas/presentation.js';
import './style.css';

function LayerCard({ id, data, selected }) {
  const updateNodeInternals = useUpdateNodeInternals();
  const previousHandles = useRef(null);
  useEffect(() => {
    const handles = `${data.inPosition}:${data.outPosition}`;
    if (previousHandles.current && previousHandles.current !== handles) updateNodeInternals(id);
    previousHandles.current = handles;
  }, [id, data.inPosition, data.outPosition, updateNodeInternals]);
  return <article className={`brain-layer-box ${selected ? 'selected' : ''}`}>
    <Handle id="forward-in" type="target" position={data.inPosition || Position.Top} isConnectable={false} />
    <header><Tag>第 {data.index + 1} 层</Tag><strong>{data.layer.title || '新里程碑'}</strong>{data.status && <Tag color={data.status === '已达标' ? 'success' : undefined}>{data.status}</Tag>}{!data.editable && <span className="brain-layer-action">查看执行 →</span>}</header>
    <p title={data.layer.task}>{data.layer.task || '点击配置里程碑要做什么'}</p>
    {data.editable && <EditButton className="nodrag" size="small" disabled={data.disabled} onClick={(event) => { event.stopPropagation(); data.onAddNode(data.layer.layer_id); }}>＋ 并行执行节点</EditButton>}
    <Handle id="forward-out" type="source" position={data.outPosition || Position.Bottom} isConnectable={false} />
  </article>;
}
function ExecutionCard({ data, selected }) {
  return <article className={`brain-execution-node ${selected ? 'selected' : ''}`}>
    <strong title={data.node.title}>{data.node.title || (data.capability ? capabilityName(data.capability) : '选择泛化能力')}</strong>
    <p title={data.node.objective}>{data.node.objective || (data.capability ? capabilityTask(data.capability) : '点击绑定能力')}</p>
    <div className="brain-execution-node-meta"><span title={data.capabilityLabel}>{data.capabilityLabel || '选择泛化能力'}</span>
      {data.status && <Tag color={data.status === '执行结束' ? 'success' : data.status === '执行失败' ? 'error' : undefined}>{data.status}</Tag>}</div>
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
export function MilestoneCanvas({ plan, capabilities = EMPTY_CAPABILITIES, selection, onSelect, onAddLayer, onAddNode, positions = EMPTY, onPositions, onResetLayout, onDelete, disabled = false, statuses = EMPTY, layerStatuses = EMPTY, direction = 'vertical' }) {
  const canEdit = useCanEdit();
  const editable = !!onAddLayer && canEdit;
  const interactive = editable && !disabled;
  const horizontal = direction === 'horizontal';
  const flow = useRef(null);
  const container = useRef(null);
  const firstLayer = useRef(null);
  firstLayer.current = plan.layers[0]?.layer_id;
  const [canvasWidth, setCanvasWidth] = useState(1280);
  const levelGroups = groups(plan);
  const projected = useMemo(() => {
    let y = 40;
    const widest = Math.max(horizontal ? 320 : 420, ...levelGroups.map((group) => Math.min(3, Math.max(1, group.length)) * 228 + 48));
    const availableColumns = Math.max(1, Math.floor(canvasWidth / (widest + 80)));
    const layoutColumns = Math.max(1, Math.ceil(plan.layers.length / Math.max(1, Math.ceil(plan.layers.length / availableColumns))));
    const rowHeight = Math.max(290, ...levelGroups.map((group) => 150 + Math.ceil(group.length / 3) * 140));
    return plan.layers.flatMap((layer, index) => {
      const children = levelGroups[index];
      const columns = Math.min(3, Math.max(1, children.length));
      const rows = Math.ceil(children.length / columns);
      const width = Math.max(horizontal ? 320 : 420, columns * 228 + 48), height = 150 + rows * 140;
      const row = Math.floor(index / layoutColumns), reverse = row % 2 === 1;
      const column = reverse ? layoutColumns - 1 - index % layoutColumns : index % layoutColumns;
      const position = positions[layer.layer_id] || (horizontal ? { x: 40 + column * (widest + 80), y: 40 + row * (rowHeight + 80) } : { x: (widest - width) / 2 + 40, y });
      y += height + 120;
      return [{ id: layerFlowId(layer.layer_id), type: 'layer', position, width, height, selected: selection?.type === 'layer' && selection.id === layer.layer_id,
        ariaLabel: `第 ${index + 1} 层 · ${layer.title || '新里程碑'}`, deletable: false, data: { layer, index, editable, disabled, onAddNode,
          inPosition: horizontal ? (index % layoutColumns === 0 ? Position.Top : reverse ? Position.Right : Position.Left) : Position.Top,
          outPosition: horizontal ? (index % layoutColumns === layoutColumns - 1 ? Position.Bottom : reverse ? Position.Left : Position.Right) : Position.Bottom,
          status: layerStatuses[layer.layer_id] } },
      ...children.map((node, column) => ({ id: executionFlowId(node.node_id), type: 'execution', parentId: layerFlowId(layer.layer_id), extent: 'parent',
        position: { x: 24 + (column % columns) * 228, y: 132 + Math.floor(column / columns) * 140 },
        width: horizontal && columns === 1 ? width - 48 : 210, height: 120, selected: selection?.type === 'node' && selection.id === node.node_id,
        ariaLabel: node.title || node.node_id, deletable: false, data: { node, capability: capabilities.find((item) => (item.capability_id || item.id) === node.capability_id), capabilityLabel: capabilityLabelFor(node, capabilities), status: statuses[node.node_id] }, draggable: false }))];
    });
  }, [plan, levelGroups.length, capabilities, selection, editable, disabled, positions, onAddNode, statuses, layerStatuses, horizontal, canvasWidth]);
  const [presentation, setPresentation] = useState({});
  const nodes = useMemo(() => displayNodes(projected, presentation), [projected, presentation]);
  // An inspector/validation selection can focus a node. Clicking an already
  // selected node must preserve React Flow's multi-selection.
  useEffect(() => {
    if (!selection) return;
    const id = selection.type === 'layer' ? layerFlowId(selection.id) : executionFlowId(selection.id);
    setPresentation((current) => current[id]?.selected ? current : Object.fromEntries([
      ...Object.entries(current).map(([key, value]) => [key, { ...value, selected: key === id }]),
      [id, { ...current[id], selected: true }],
    ]));
  }, [selection?.type, selection?.id]);
  const changeNodes = (changes) => {
    setPresentation((current) => changePresentation(current, changes, projected, interactive));
    if (interactive) {
      const patch = committedPositions(changes, nodes);
      if (Object.keys(patch).length) onPositions?.(patch);
    }
  };
  useEffect(() => {
    const timer = setTimeout(() => flow.current?.fitView({ padding: 0.12, maxZoom: 1, duration: 180,
      ...(!editable && canvasWidth < 600 && plan.layers.length ? { nodes: [{ id: layerFlowId(plan.layers[0].layer_id) }] } : {}),
    }), 100);
    return () => clearTimeout(timer);
  }, [plan.layers.length, plan.nodes.length, canvasWidth, editable]);
  useEffect(() => {
    if (!container.current) return undefined;
    let timer;
    const observer = new ResizeObserver(() => {
      setCanvasWidth(container.current.clientWidth);
      clearTimeout(timer); timer = setTimeout(() => flow.current?.fitView({ padding: 0.12, maxZoom: 1, duration: 180,
        ...(!editable && container.current.clientWidth < 600 && firstLayer.current ? { nodes: [{ id: layerFlowId(firstLayer.current) }] } : {}),
      }), 180);
    });
    observer.observe(container.current);
    return () => { clearTimeout(timer); observer.disconnect(); };
  }, [editable]);
  const edges = plan.layers.slice(1).map((layer, index) => ({
    id: `sequence:${plan.layers[index].layer_id}:${layer.layer_id}`,
    source: layerFlowId(plan.layers[index].layer_id), target: layerFlowId(layer.layer_id),
    sourceHandle: 'forward-out', targetHandle: 'forward-in', type: 'smoothstep',
    selectable: false, deletable: false, ariaLabel: `第 ${index + 1} 层 → 第 ${index + 2} 层`,
    style: { stroke: '#87a3c2', strokeWidth: 2 }, markerEnd: { type: MarkerType.ArrowClosed },
  }));
  return <div ref={container} className={`brain-milestone-canvas ${editable ? 'is-editable' : ''}`} aria-label={editable ? '里程碑编辑画布' : '里程碑执行画布'}
    onKeyDown={(event) => {
      if (interactive && ['Backspace', 'Delete'].includes(event.key) && !event.target.closest('input, textarea, select, button, [contenteditable="true"], [role="combobox"]')) {
        const selected = nodes.filter((node) => node.selected).map((node) => flowSelection(node.id));
        if (selected.length) { event.preventDefault(); event.stopPropagation(); onDelete?.(selected); }
      }
      const node = event.target.closest('.react-flow__node');
      if (!editable && node && event.target === node && ['Enter', ' '].includes(event.key)) {
        event.preventDefault(); onSelect?.({ type: node.dataset.id.startsWith('layer:') ? 'layer' : 'node', id: originalFlowId(node.dataset.id) });
      }
    }}>
    {editable && <div className="brain-milestone-tools"><EditButton disabled={disabled} onClick={onAddLayer}>＋ 里程碑</EditButton><EditButton disabled={disabled} onClick={onResetLayout}>整理布局</EditButton><span>大脑根据执行结果决定前进或回到已执行的里程碑</span></div>}
    <div className="brain-milestone-flow"><ReactFlow onInit={(instance) => { flow.current = instance; }} nodes={nodes} edges={edges} nodeTypes={nodeTypes} nodesDraggable={interactive} nodesConnectable={false} deleteKeyCode={null} fitView minZoom={0.15} maxZoom={2}
      ariaLabelConfig={{ 'controls.zoomIn.ariaLabel': '放大画布', 'controls.zoomOut.ariaLabel': '缩小画布', 'controls.fitView.ariaLabel': '查看全图', 'minimap.ariaLabel': '画布导航' }}
      onNodesChange={changeNodes}
      onNodeClick={(event, node) => { event.currentTarget.focus({ preventScroll: true }); onSelect?.(flowSelection(node.id)); }}>
      <Background /><Controls showInteractive={false} />{(editable || plan.layers.length > 6) && <MiniMap pannable zoomable />}
    </ReactFlow></div>
    {editable && !plan.layers.length && <div className="brain-milestone-empty"><h3>从第一个里程碑开始</h3><p>在画布配置里程碑和并行节点</p><EditButton disabled={disabled} type="primary" onClick={onAddLayer}>添加第一个里程碑</EditButton></div>}
  </div>;
}
