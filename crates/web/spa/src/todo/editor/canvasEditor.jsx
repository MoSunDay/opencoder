// canvasEditor.jsx — the visual TODO editor canvas: React Flow assembly of
// the palette / toolbar / inspector around specToCanvas state. Mirrors
// dag/editor/canvasEditor.jsx with todos/id in place of steps/name. The
// `spec` prop is an INITIAL value only (the parent remounts via key when
// it wants a fresh load); every structural change flows out through
// onSpecChange. TodoCanvasEditor itself only mounts a ReactFlowProvider —
// useReactFlow must be called INSIDE the provider (React Flow's own
// provider wraps only the <ReactFlow> children, not the component that
// renders it).

import {
  addEdge,
  Background,
  BackgroundVariant,
  Controls,
  MarkerType,
  ReactFlow,
  ReactFlowProvider,
  useEdgesState,
  useNodesState,
  useReactFlow,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useMessage } from '../../ui/appMessage.js';
import { layoutTodoNodes } from './canvasLayout.js';
import {
  canConnect,
  canvasToSpec,
  newTodo,
  renameTodo,
  specLevelProblems,
  specProblemIndex,
  specToCanvas,
} from './canvasModel.js';
import { TodoCanvasToolbar, TodoPalette } from './canvasToolbar.jsx';
import { SpecMetaForm, TodoInspector } from './todoInspector.jsx';
import { editNodeTypes } from './todoNode.jsx';

/// orderDepNames(nodes, edges, id) → deduped incoming edge sources for node
/// `id`, ordered by the source's position in the node array — the exact
/// order canvasToSpec rebuilds depends_on with, so the node card's 依赖
/// summary and the emitted spec can never disagree.
function orderDepNames(nodes, edges, id) {
  const pos = new Map();
  (Array.isArray(nodes) ? nodes : []).forEach((n, i) => {
    if (n && typeof n.id === 'string') {
      pos.set(n.id, i);
    }
  });
  const seen = new Set();
  const names = [];
  for (const e of Array.isArray(edges) ? edges : []) {
    if (!e || e.target !== id || typeof e.source !== 'string' || seen.has(e.source) || !pos.has(e.source)) {
      continue;
    }
    seen.add(e.source);
    names.push(e.source);
  }
  return names.sort((a, b) => pos.get(a) - pos.get(b));
}

/// TodoCanvasEditor — uncontrolled-after-mount canvas over a WorkflowSpec
/// draft. props: spec (initial), problems (validateSpec [{path, message}]
/// for red dots + toolbar messages), positions ({todoId: {x, y}}
/// session-state map), onSpecChange(spec), onPositionsChange(positions).
export function TodoCanvasEditor(props) {
  return (
    <ReactFlowProvider>
      <EditorCanvas {...props} />
    </ReactFlowProvider>
  );
}

/// EditorCanvas — hook body of TodoCanvasEditor (kept inside the provider
/// so useReactFlow / fitView / screenToFlowPosition resolve).
function EditorCanvas({ spec, problems, positions, onSpecChange, onPositionsChange }) {
  const msg = useMessage();
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const [selectedId, setSelectedId] = useState(null);
  const [meta, setMeta] = useState(spec); // SpecMetaForm base (name/objective/constraints)
  const specRef = useRef(spec); // spec-level fields carry-through for emit
  const dirtyRef = useRef(false); // structural change → emit on next commit
  const laidRef = useRef(false); // init effect ran; meta effect may touch nodes
  const { fitView, screenToFlowPosition } = useReactFlow();
  const wrapRef = useRef(null);

  // One-time load: spec → nodes/edges, dagre-laid with the session positions.
  useEffect(() => {
    const init = specToCanvas(spec);
    setNodes(layoutTodoNodes(init.nodes, init.edges, { positions: positions || {} }));
    setEdges(init.edges);
    laidRef.current = true;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Structural changes (add/rename/remove/connect) emit the rebuilt spec.
  useEffect(() => {
    if (!dirtyRef.current) {
      return;
    }
    dirtyRef.current = false;
    onSpecChange(canvasToSpec({ nodes, edges }, specRef.current));
  }, [nodes, edges, onSpecChange]);

  // Node card meta (dep summary + red invalid dot) follows edges/problems.
  useEffect(() => {
    if (!laidRef.current) {
      return;
    }
    setNodes((cur) => {
      const idx = specProblemIndex(problems || [], cur);
      return cur.map((n) => ({
        ...n,
        data: {
          ...n.data,
          depNames: orderDepNames(cur, edges, n.id),
          invalid: (idx.get(n.id) || []).length > 0,
        },
      }));
    });
  }, [edges, problems]);

  // Fit once the first layout has committed (next macrotask keeps it calm).
  useEffect(() => {
    const t = setTimeout(() => fitView({ padding: 0.18, duration: 200 }), 60);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Drawer resize → refit so nodes never drift off-screen.
  useEffect(() => {
    if (typeof ResizeObserver === 'undefined' || !wrapRef.current) {
      return undefined;
    }
    const ro = new ResizeObserver(() => {
      if (laidRef.current) {
        fitView({ padding: 0.18, duration: 150 });
      }
    });
    ro.observe(wrapRef.current);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const markDirty = () => {
    dirtyRef.current = true;
  };

  // Position/selection changes flow through silently; deletions emit.
  const onNodesChangeWrapped = useCallback(
    (changes) => {
      onNodesChange(changes);
      if (changes.some((c) => c.type === 'remove')) {
        markDirty();
      }
    },
    [onNodesChange],
  );
  const onEdgesChangeWrapped = useCallback(
    (changes) => {
      onEdgesChange(changes);
      if (changes.some((c) => c.type === 'remove')) {
        markDirty();
      }
    },
    [onEdgesChange],
  );

  const onConnect = useCallback(
    (params) => {
      const reason = canConnect(edges, params.source, params.target);
      if (reason) {
        msg.warning(reason);
        return;
      }
      setEdges(addEdge({ ...params, type: 'smoothstep', markerEnd: { type: MarkerType.ArrowClosed } }, edges));
      markDirty();
    },
    [edges, setEdges],
  );

  // Drag stops persist the pin into the session-state positions map only.
  const onNodeDragStop = useCallback(
    (_e, node) => {
      onPositionsChange({ ...(positions || {}), [node.id]: node.position });
    },
    [positions, onPositionsChange],
  );

  const addTodo = (position) => {
    const taken = nodes.map((n) => n.id);
    const todo = newTodo(taken);
    const node = {
      id: todo.id,
      type: 'todoEdit',
      position: position || { x: 60, y: 60 + taken.length * 24 },
      selected: true, // keep React Flow's selection state in sync with selectedId
      data: { todo, depNames: [], placed: true },
    };
    setNodes(nodes.concat(node));
    setSelectedId(todo.id);
    markDirty();
  };

  // HTML5 drop from the palette (dataTransfer carries the 'todo' kind).
  const onDrop = (e) => {
    e.preventDefault();
    const kind = e.dataTransfer.getData('application/opencoder-todo');
    if (kind !== 'todo') {
      return;
    }
    addTodo(screenToFlowPosition({ x: e.clientX, y: e.clientY }));
  };
  const onDragOver = (e) => {
    e.preventDefault();
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = 'move';
    }
  };

  const autoLayout = () => {
    setNodes((cur) => layoutTodoNodes(cur, edges, {}));
    onPositionsChange({});
    markDirty();
    setTimeout(() => fitView({ padding: 0.18, duration: 250 }), 50);
  };

  // ---- Inspector wiring -------------------------------------------------
  const selectedNode = nodes.find((n) => n.id === selectedId) || null;
  const problemIdx = useMemo(() => specProblemIndex(problems || [], nodes), [problems, nodes]);
  const problemMapFor = (id) => problemIdx.get(id) || [];

  const updateTodo = (nextTodo) => {
    setNodes((cur) =>
      cur.map((n) => (n.id === selectedId ? { ...n, data: { ...n.data, todo: nextTodo } } : n)),
    );
    markDirty();
  };

  // renameTodo owns every rename rule (empty/collision + id/depends_on
  // sync inside every todo); failures surface here as a warning toast.
  const renameNode = (nextId) => {
    const old = selectedId;
    if (!old || nextId === old) {
      return;
    }
    const res = renameTodo({ nodes, edges }, old, nextId);
    if (res.error) {
      msg.warning(res.error);
      return;
    }
    const next = nextId.trim(); // renameTodo commits the trimmed id
    setNodes(res.canvas.nodes);
    setEdges(res.canvas.edges);
    if (positions && positions[old]) {
      const nextPos = { ...positions };
      nextPos[next] = nextPos[old];
      delete nextPos[old];
      onPositionsChange(nextPos);
    }
    setSelectedId(next);
    markDirty();
  };

  const removeNode = () => {
    const id = selectedId;
    setNodes((cur) => cur.filter((n) => n.id !== id));
    setEdges((cur) => cur.filter((e) => e.source !== id && e.target !== id));
    setSelectedId(null);
    markDirty();
  };

  // SpecMetaForm edits name/objective/constraints; `meta` state (not just
  // the ref) keeps the controlled inputs re-rendering while the canvas
  // emits. Its onChange already carries all three merged fields.
  const applyMeta = (partial) => {
    const next = { ...meta, ...partial };
    setMeta(next);
    specRef.current = next;
    onSpecChange(canvasToSpec({ nodes, edges }, next));
  };

  return (
    <div className="dag-edit-wrap">
      <TodoPalette onAdd={() => addTodo()} />
      <div className="dag-edit-stage" ref={wrapRef} onDrop={onDrop} onDragOver={onDragOver}>
        <TodoCanvasToolbar
          problems={specLevelProblems(problems || [])}
          onAutoLayout={autoLayout}
          onFitView={() => fitView({ padding: 0.18, duration: 200 })}
        />
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={editNodeTypes}
          onNodesChange={onNodesChangeWrapped}
          onEdgesChange={onEdgesChangeWrapped}
          onConnect={onConnect}
          onSelectionChange={({ nodes: sel }) => setSelectedId(sel.length ? sel[0].id : null)}
          onNodeDragStop={onNodeDragStop}
          isValidConnection={(c) => canConnect(edges, c.source, c.target) === null}
          deleteKeyCode={['Delete', 'Backspace']}
          defaultEdgeOptions={{ type: 'smoothstep', markerEnd: { type: MarkerType.ArrowClosed } }}
          fitView
          fitViewOptions={{ padding: 0.18 }}
          proOptions={{ hideAttribution: false }}
        >
          <Background variant={BackgroundVariant.Dots} gap={20} size={1} />
          <Controls showInteractive={false} />
        </ReactFlow>
      </div>
      {selectedNode ? (
        <TodoInspector
          todo={selectedNode.data.todo}
          allIds={nodes.filter((n) => n.id !== selectedId).map((n) => n.id)}
          problemList={problemMapFor(selectedId)}
          onChange={updateTodo}
          onRename={renameNode}
          onRemove={removeNode}
        />
      ) : (
        <SpecMetaForm spec={meta} onChange={applyMeta} />
      )}
    </div>
  );
}
