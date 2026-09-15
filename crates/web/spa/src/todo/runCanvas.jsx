// runCanvas.jsx — TODO 运行态画布：只读 React Flow，把 items + SSE 折叠出的
// 每 TODO 状态投影到 spec 依赖图上。与 editor/canvasEditor.jsx 的分工镜像
// dag 的 defEditor（编辑）/ process.jsx（运行）：这里不可拖拽/连线，节点
// 卡片渲染运行语义（状态 Tag + 尝试次数 + 依赖），点击节点由父级联动
// TodoRunInspector 展示该 TODO 的需求/验收/会话细节。坐标每次 dagre 自动
// 布局（runProjection.runGraph），运行视图没有会话态可保。

import { Background, Controls, Handle, Position, ReactFlow, ReactFlowProvider, useReactFlow } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useEffect, useMemo } from 'react';
import { Alert, Descriptions, Empty, Typography } from 'antd';
import { StatusTag } from '../ui/statusTag.jsx';
import { MONO_VAR } from '../ui/mono.js';
import { runGraph, todoRunClass } from './runProjection.js';

const { Text } = Typography;

/// TodoRunNode — 只读运行卡片。data: { todo, status, attempt, depNames }。
/// data-status 落在根节点上，DOM 测试按它断言，不依赖 antd Tag 的内部结构。
export function TodoRunNode({ data, selected }) {
  const todo = (data && data.todo) || {};
  const status = (data && data.status) || 'pending';
  const attempt = Number((data && data.attempt) || 0);
  const depNames = Array.isArray(data && data.depNames) ? data.depNames : [];
  const title = typeof todo.title === 'string' && todo.title.trim() ? todo.title : todo.id;
  const cls = 'dag-node oc-todo-run-node ' + todoRunClass(status) + (selected ? ' dag-node--selected' : '');
  return (
    <div className={cls} data-todo-id={todo.id} data-status={status}>
      <Handle type="target" position={Position.Left} isConnectable={false} />
      <div className="dag-node-title">
        <span className="dag-node-name">{title}</span>
        <span className="dag-node-kind">{todo.agent || '-'}</span>
      </div>
      <div className="dag-node-status">
        <StatusTag status={status} />
        {attempt > 0 ? <span className="oc-todo-run-attempt">尝试 {attempt}</span> : null}
      </div>
      <div className="oc-todo-run-deps">
        {depNames.length ? '依赖: ' + depNames.join(', ') : '无依赖'}
      </div>
      <Handle type="source" position={Position.Right} isConnectable={false} />
    </div>
  );
}

/// runNodeTypes — 模块级稳定 map，避免 React Flow 在父级重渲染时重挂节点。
const runNodeTypes = { todoRun: TodoRunNode };

function RunFlow({ spec, states, selectedId, onSelect, height }) {
  const topology = JSON.stringify((spec?.todos||[]).map(t=>[t.id,t.depends_on]));
  const graph = useMemo(() => runGraph(spec, new Map()), [topology]);
  const {fitView}=useReactFlow();
  useEffect(()=>{if(selectedId)fitView({nodes:[{id:selectedId}],duration:180,padding:0.5,maxZoom:1});},[selectedId,fitView]);
  if (!graph.nodes.length) {
    return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="spec 未加载或无 TODO" />;
  }
  const nodes = graph.nodes.map((n) => ({ ...n, selected: n.id === selectedId,
    data:{...n.data,status:states.get(n.id)?.status||'pending',attempt:states.get(n.id)?.attempt||0} }));
  return (
    <div className="dag-detail-graph" style={{ height }}>
      <ReactFlow
        nodes={nodes}
        edges={graph.edges}
        nodeTypes={runNodeTypes}
        onNodeClick={(_, node) => { if (onSelect) { onSelect(node.id); } }}
        fitView
        nodesDraggable={false}
        nodesConnectable={false}
      >
        <Background />
        <Controls showInteractive={false} />
      </ReactFlow>
    </div>
  );
}

/// TodoRunCanvas — 运行画布入口。spec 是 workflow.spec_json（裸 WorkflowSpec）；
/// states 是 runProjection 折叠出的 Map(todo_id → view state)。
export function TodoRunCanvas({ spec, states, selectedId, onSelect, height = 320 }) {
  return (
    <ReactFlowProvider>
      <RunFlow spec={spec} states={states} selectedId={selectedId} onSelect={onSelect} height={height} />
    </ReactFlowProvider>
  );
}

/// TodoRunInspector — 选中 TODO 的运行细节：需求/验收/当前会话/最近错误。
export function TodoRunInspector({ todo, state }) {
  if (!todo) {
    return null;
  }
  const st = state || {};
  const calls = (todo.acceptance && Array.isArray(todo.acceptance.required_tool_calls))
    ? todo.acceptance.required_tool_calls.map((c) => (c && c.name) || '').filter(Boolean)
    : [];
  const deps = Array.isArray(todo.depends_on) ? todo.depends_on : [];
  return (
    <div className="oc-todo-run-inspector">
      <Text strong>{todo.title || todo.id}</Text>
      <Descriptions
        column={2}
        size="small"
        items={[
          { key: 'status', label: '状态', children: <StatusTag status={st.status || 'pending'} /> },
          { key: 'attempt', label: '尝试', children: st.attempt || 0 },
          { key: 'agent', label: 'Agent', children: todo.agent || '-' },
          { key: 'deps', label: '依赖', children: deps.join(', ') || '无' },
          { key: 'session', label: '当前会话', children: st.activeSessionId
            ? <Text copyable={{ text: st.activeSessionId }} style={{ fontFamily: MONO_VAR, fontSize: 12 }}>{String(st.activeSessionId).slice(0, 14)}…</Text>
            : '-' },
          { key: 'criteria', label: '验收标准', children: (todo.acceptance && todo.acceptance.criteria) || '-' },
          ...(calls.length ? [{ key: 'calls', label: '必需工具调用', children: calls.join(', ') }] : []),
        ]}
      />
      {todo.instructions ? <pre className="oc-todo-run-pre">{todo.instructions}</pre> : null}
      {st.lastError ? <Alert style={{ marginTop: 8 }} type="error" showIcon title={st.lastError} /> : null}
    </div>
  );
}
