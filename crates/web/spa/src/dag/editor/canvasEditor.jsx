// canvasEditor.jsx — the visual DAG editor canvas: React Flow assembly of
// the palette / toolbar / inspector around specToCanvas state. The `spec`
// prop is an INITIAL value only (the parent remounts via key when it wants
// a fresh load); every structural change flows out through onSpecChange.
// CanvasEditor itself only mounts a ReactFlowProvider — useReactFlow must
// be called INSIDE the provider (React Flow's own provider wraps only the
// <ReactFlow> children, not the component that renders it).

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
import { message } from 'antd';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { layoutEditorNodes } from './canvasLayout.js';
import { canConnect, canvasToSpec, newStep, specProblemIndex, specToCanvas } from './canvasModel.js';
import { CanvasToolbar, StepPalette } from './canvasToolbar.jsx';
import { SpecMetaForm, StepInspector } from './stepInspector.jsx';
import { editNodeTypes } from './stepNode.jsx';

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

/// CanvasEditor — uncontrolled-after-mount canvas over a DagSpec draft.
/// props: spec (initial), problems (validateSpec strings for red dots +
/// toolbar badge), positions ({stepName: {x,y}} session-state map),
/// onSpecChange(spec), onPositionsChange(positions).
export function CanvasEditor(props) {
  return (
    <ReactFlowProvider>
      <EditorCanvas {...props} />
    </ReactFlowProvider>
  );
}

/// EditorCanvas — hook body of CanvasEditor (kept inside the provider so
/// useReactFlow / fitView / screenToFlowPosition resolve).
function EditorCanvas({ spec, problems, positions, onSpecChange, onPositionsChange }) {
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const [selectedId, setSelectedId] = useState(null);
  const [meta, setMeta] = useState(spec); // SpecMetaForm base (name/description)
  const specRef = useRef(spec); // name/description carry-through for emit
  const dirtyRef = useRef(false); // structural change → emit on next commit
  const laidRef = useRef(false); // init effect ran; meta effect may touch nodes
  const { fitView, screenToFlowPosition } = useReactFlow();
  const wrapRef = useRef(null);

  // One-time load: spec → nodes/edges, dagre-laid with the session positions.
  useEffect(() => {
    const init = specToCanvas(spec);
    setNodes(layoutEditorNodes(init.nodes, init.edges, { positions: positions || {} }));
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
          kindType: (n.data && n.data.step && n.data.step.kind && n.data.step.kind.type) || '',
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
        message.warning(reason);
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

  const addStep = (kindType, position) => {
    const taken = nodes.map((n) => n.id);
    const step = newStep(kindType, taken);
    const node = {
      id: step.name,
      type: 'stepEdit',
      position: position || { x: 60, y: 60 + taken.length * 24 },
      selected: true, // keep React Flow's selection state in sync with selectedId
      data: { step, kindType, depNames: [], placed: true },
    };
    setNodes(nodes.concat(node));
    setSelectedId(step.name);
    markDirty();
  };

  // HTML5 drop from the palette (dataTransfer carries the kind type).
  const onDrop = (e) => {
    e.preventDefault();
    const kind = e.dataTransfer.getData('application/opencoder-step');
    if (kind !== 'agent' && kind !== 'wasm') {
      return;
    }
    addStep(kind, screenToFlowPosition({ x: e.clientX, y: e.clientY }));
  };
  const onDragOver = (e) => {
    e.preventDefault();
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = 'move';
    }
  };

  const autoLayout = () => {
    setNodes((cur) => layoutEditorNodes(cur, edges, {}));
    onPositionsChange({});
    markDirty();
    setTimeout(() => fitView({ padding: 0.18, duration: 250 }), 50);
  };

  // ---- Inspector wiring -------------------------------------------------
  const selectedNode = nodes.find((n) => n.id === selectedId) || null;
  const problemIdx = useMemo(() => specProblemIndex(problems || [], nodes), [problems, nodes]);
  const problemMapFor = (id) => problemIdx.get(id) || [];

  const updateStep = (nextStep) => {
    setNodes((cur) =>
      cur.map((n) =>
        n.id === selectedId
          ? { ...n, data: { ...n.data, step: nextStep, kindType: (nextStep.kind && nextStep.kind.type) || '' } }
          : n,
      ),
    );
    markDirty();
  };

  const renameNode = (name) => {
    const old = selectedId;
    if (!old || name === old) {
      return;
    }
    setNodes((cur) =>
      cur.map((n) => (n.id === old ? { ...n, id: name, data: { ...n.data, step: { ...n.data.step, name } } } : n)),
    );
    setEdges((cur) =>
      cur.map((e) => {
        const s = e.source === old ? name : e.source;
        const t = e.target === old ? name : e.target;
        return e.source === old || e.target === old ? { ...e, id: 'e-' + s + '-' + t, source: s, target: t } : e;
      }),
    );
    if (positions && positions[old]) {
      const nextPos = { ...positions };
      nextPos[name] = nextPos[old];
      delete nextPos[old];
      onPositionsChange(nextPos);
    }
    setSelectedId(name);
    markDirty();
  };

  const removeNode = () => {
    const id = selectedId;
    setNodes((cur) => cur.filter((n) => n.id !== id));
    setEdges((cur) => cur.filter((e) => e.source !== id && e.target !== id));
    setSelectedId(null);
    markDirty();
  };

  // SpecMetaForm edits name/description; `meta` state (not just the ref)
  // keeps the controlled inputs re-rendering while the canvas emits.
  const applyMeta = (partial) => {
    const next = { ...meta, ...partial };
    setMeta(next);
    specRef.current = next;
    onSpecChange(canvasToSpec({ nodes, edges }, next));
  };

  return (
    <div className="dag-edit-wrap">
      <StepPalette onAdd={(k) => addStep(k)} />
      <div className="dag-edit-stage" ref={wrapRef} onDrop={onDrop} onDragOver={onDragOver}>
        <CanvasToolbar
          problems={problems || []}
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
        <StepInspector
          step={selectedNode.data.step}
          allNames={nodes.filter((n) => n.id !== selectedId).map((n) => n.id)}
          problemList={problemMapFor(selectedId)}
          onChange={updateStep}
          onRename={renameNode}
          onRemove={removeNode}
        />
      ) : (
        <SpecMetaForm spec={meta} onChange={applyMeta} />
      )}
    </div>
  );
}
