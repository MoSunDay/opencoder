// Normalize DAG events and project step snapshots onto the run graph.

import { NODE_H, NODE_W, layoutGraph } from './dag/dagLayout.js';

export const STEP_PENDING = 'pending';
export const STEP_ERROR = 'error';
export const STEP_SKIPPED = 'skipped';

/// Event kind vocabulary (server validates uploads against the same set).
export const RUN_EVENT_KINDS = ['run_started', 'step_started', 'step_done', 'step_progress', 'run_finished'];
/// frameToEvent(frame) — normalize one sse.js frame ({event, data, seq}) into
/// an event record {seq, kind, step, payload, at_ms}. Returns null when the
/// frame is not a well-formed DAG event (keep-alives, foreign frames).
export function frameToEvent(frame) {
  const d = (frame && frame.data) || {};
  const kind = String(d.kind || (frame && frame.event) || '');
  if (!RUN_EVENT_KINDS.includes(kind)) {
    return null;
  }
  const stepKinds = ['step_started', 'step_done', 'step_progress'];
  const step = d.step === undefined || d.step === null || d.step === '' ? null : String(d.step);
  if (stepKinds.includes(kind) && !step) {
    return null; // a step event without a step name cannot fold anywhere
  }
  const dataSeq = Number(d.seq);
  const idSeq = frame && Number(frame.seq);
  const seq = Number.isFinite(dataSeq) ? dataSeq : Number.isFinite(idSeq) ? idSeq : null;
  const at = Number(d.at_ms);
  return {
    seq,
    kind,
    step,
    payload: d.payload === undefined || d.payload === null ? {} : d.payload,
    at_ms: Number.isFinite(at) ? at : null,
  };
}

/// specSteps(spec) → the step list, degrading to [] for malformed specs.
export function specSteps(spec) {
  const steps = spec && Array.isArray(spec.steps) ? spec.steps : [];
  return steps.filter((s) => s && typeof s.name === 'string');
}

/// dependsOn(step) → the step's declared deps (strings only).
export function dependsOn(step) {
  return Array.isArray(step && step.depends_on) ? step.depends_on.filter((d) => typeof d === 'string') : [];
}

/// projectStepStatuses(spec, stepStates) → Map(step → terminal projection).
/// Adds STEP_SKIPPED for steps still pending whose (transitive) deps ended in
/// error/skipped — the runtime never starts them, so grey is the honest
/// projection. Fixpoint pass, bounded by step count; cycles cannot loop it.
export function projectStepStatuses(spec, stepStates) {
  const steps = specSteps(spec);
  const known = new Set(steps.map((s) => s.name));
  const states = stepStates instanceof Map ? stepStates : new Map();
  const out = new Map();
  for (const s of steps) {
    const st = states.get(s.name);
    out.set(s.name, st && st.status ? st.status : STEP_PENDING);
  }
  for (let pass = 0; pass < steps.length; pass += 1) {
    let changed = false;
    for (const s of steps) {
      if (out.get(s.name) !== STEP_PENDING) {
        continue;
      }
      const blocked = dependsOn(s)
        .filter((d) => known.has(d))
        .some((d) => out.get(d) === STEP_ERROR || out.get(d) === 'cancelled' || out.get(d) === STEP_SKIPPED);
      if (blocked) {
        out.set(s.name, STEP_SKIPPED);
        changed = true;
      }
    }
    if (!changed) {
      break;
    }
  }
  return out;
}

/// dropCycleEdges(edges) → acyclic edge list. Server-side specs are validated
/// acyclic, but the graph must never wedge on hostile/stale snapshots: edges
/// are kept in order, and any edge that would close a cycle against the ones
/// already kept (a target→source path exists) is dropped — deterministic and
/// order-preserving.
export function dropCycleEdges(edges) {
  const kept = [];
  const adj = new Map(); // id → Set of reachable ids, grown as edges are kept
  const reach = (from, to) => {
    if (from === to) {
      return true;
    }
    const seen = new Set([from]);
    const queue = [from];
    while (queue.length) {
      const cur = queue.pop();
      for (const next of adj.get(cur) || []) {
        if (next === to) {
          return true;
        }
        if (!seen.has(next)) {
          seen.add(next);
          queue.push(next);
        }
      }
    }
    return false;
  };
  for (const e of edges) {
    // e.source→e.target closes a cycle iff target already reaches source.
    if (e.source !== e.target && !reach(e.target, e.source)) {
      kept.push(e);
      if (!adj.has(e.source)) {
        adj.set(e.source, new Set());
      }
      adj.get(e.source).add(e.target);
    }
  }
  return kept;
}

/// RUN_HANDLE — default React Flow handle size (6px, see
/// @xyflow/react style.css); handles straddle the node border, hence the
/// -size/2 x offsets. runNodeBox() returns a FRESH object per call: React
/// Flow's toHandleBounds() mutates handle entries, so sharing one frozen
/// array across nodes would leak writes.
const RUN_HANDLE = 6;

/// runNodeBox() → {width, handles} declaring the fixed run-view card box
/// (mirrors .dag-node CSS and the dagre constants). React Flow can place
/// edges using these initial handle bounds before measuring.
/// Node visibility still requires measured width AND height; DagProcess
/// retains those measurements when snapshots replace the node data.
/// Height is deliberately
/// NOT declared: getNodeInlineStyleDimensions() would bake a permanent
/// inline height:52px onto the wrapper while .dag-node has no
/// position:relative — error text grows cards past 52px, yet handles
/// would stay pinned to the stale box and fitView would read the clamped
/// measured.height. ResizeObserver reports the true DOM height.
function runNodeBox() {
  return {
    width: NODE_W,
    handles: [
      { type: 'target', position: 'left', x: -RUN_HANDLE / 2, y: (NODE_H - RUN_HANDLE) / 2, width: RUN_HANDLE, height: RUN_HANDLE },
      { type: 'source', position: 'right', x: NODE_W - RUN_HANDLE / 2, y: (NODE_H - RUN_HANDLE) / 2, width: RUN_HANDLE, height: RUN_HANDLE },
    ],
  };
}

/// graphFromSpec(spec, stepStates, opts) → {nodes, edges} shaped for React
/// Flow: node ids are step slugs, edges follow depends_on, positions come
/// from the dagre layout, node.data carries the projected status plus the
/// step_done snapshot fields for the click card.
export function graphFromSpec(spec, stepStates, opts = {}) {
  const steps = specSteps(spec);
  const statuses = projectStepStatuses(spec, stepStates);
  const states = stepStates instanceof Map ? stepStates : new Map();
  const defined = new Set(steps.map((s) => s.name));
  const seen = new Set();
  let edges = [];
  for (const s of steps) {
    for (const dep of dependsOn(s)) {
      const id = dep + '\u0000' + s.name;
      if (!defined.has(dep) || seen.has(id)) {
        continue; // unknown dep (server rejects the spec) / duplicate edge
      }
      seen.add(id);
      // '>' never occurs in a slug, so 'a'→'b-c' and 'a-b'→'c' can't collide
      edges.push({ id: 'e-' + dep + '>' + s.name, source: dep, target: s.name });
    }
  }
  edges = dropCycleEdges(edges);
  const nodes = steps.map((s) => {
    const st = states.get(s.name) || {};
    return {
      id: s.name,
      type: 'dagStep',
      position: { x: 0, y: 0 },
      ...runNodeBox(),
      draggable: false,
      data: {
        label: s.name,
        kindType: (s.kind && s.kind.type) || '',
        instances: st.instances || null,
        status: statuses.get(s.name) || STEP_PENDING,
        output: typeof st.output === 'string' ? st.output : '',
        error: typeof st.error === 'string' ? st.error : '',
        at_ms: Number.isFinite(st.at_ms) ? st.at_ms : null,
      },
    };
  });
  return { nodes: layoutGraph(nodes, edges, opts), edges };
}

/// runStatusTag/runStatusLabel were removed in iteration 3: the status →
/// (color, 中文) mapping now lives in ONE place — src/ui/statusTag.jsx — and
/// runBits/runDetail consume it directly.

/// nodeBadgeText(nodeId, emptyText) — 执行节点 badge text: the claiming
/// node id, or the unclaimed hint while the run waits in the queue.
export function nodeBadgeText(nodeId, emptyText = '任意节点排队中') {
  return nodeId === undefined || nodeId === null || nodeId === '' ? emptyText : String(nodeId);
}
