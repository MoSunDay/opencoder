// dagProjection.js — PURE fold/projection helpers for the DAG run view.
// Everything here is a plain function over wire data (crates/dag/src/protocol.rs
// shapes): SSE event frames fold into per-step states, step states project
// onto the spec graph (pending/running/done/error/skipped incl. transitive
// dep-failure skipping), and small display tokens map run/step statuses to
// antd colors. No React, no DOM — fully unit-tested in dagProjection.test.js.

import { layoutGraph } from './dag/dagLayout.js';

export const STEP_PENDING = 'pending';
export const STEP_RUNNING = 'running';
export const STEP_DONE = 'done';
export const STEP_ERROR = 'error';
export const STEP_SKIPPED = 'skipped';

/// Event kind vocabulary (server validates uploads against the same set).
export const RUN_EVENT_KINDS = ['run_started', 'step_started', 'step_done', 'run_finished'];
const STEP_KINDS = ['step_started', 'step_done'];

/// frameToEvent(frame) — normalize one sse.js frame ({event, data, seq}) into
/// an event record {seq, kind, step, payload, at_ms}. Returns null when the
/// frame is not a well-formed DAG event (keep-alives, foreign frames).
export function frameToEvent(frame) {
  const d = (frame && frame.data) || {};
  const kind = String(d.kind || (frame && frame.event) || '');
  if (!RUN_EVENT_KINDS.includes(kind)) {
    return null;
  }
  const stepKinds = ['step_started', 'step_done'];
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

/// foldStepStates(events) → Map(step → {status, output, error, at_ms}).
/// Projection rules (per step, later events win):
///   step_started → running; step_done {ok} → done|error (+output snapshot,
///   +error text). Events are ordered by seq (arrival order for unpersisted
///   frames), replay duplicates (same seq / same instant) collapse — folding
///   the SSE replay plus the live tail is idempotent.
export function foldStepStates(events) {
  const list = (Array.isArray(events) ? events : []).filter(
    (e) => e && STEP_KINDS.includes(e.kind) && e.step,
  );
  const rank = (e) => (Number.isFinite(e.seq) ? e.seq : Number.MAX_SAFE_INTEGER);
  const sorted = [...list].sort((a, b) => rank(a) - rank(b)); // stable: arrival order wins on ties
  const seen = new Set();
  const map = new Map();
  for (const ev of sorted) {
    const key = Number.isFinite(ev.seq)
      ? 'seq:' + ev.seq
      : 'inst:' + ev.kind + ':' + ev.step + ':' + ev.at_ms;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    const cur = map.get(ev.step) || { status: STEP_PENDING, output: '', error: '', at_ms: null };
    if (ev.kind === 'step_started') {
      map.set(ev.step, { ...cur, status: STEP_RUNNING, at_ms: ev.at_ms });
    } else {
      const p = ev.payload || {};
      const ok = p.ok !== false;
      map.set(ev.step, {
        status: ok ? STEP_DONE : STEP_ERROR,
        output: typeof p.output === 'string' ? p.output : '',
        error: ok ? '' : String(p.error || ''),
        at_ms: ev.at_ms,
      });
    }
  }
  return map;
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
        .some((d) => out.get(d) === STEP_ERROR || out.get(d) === STEP_SKIPPED);
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
      edges.push({ id: 'e-' + dep + '-' + s.name, source: dep, target: s.name });
    }
  }
  edges = dropCycleEdges(edges);
  const nodes = steps.map((s) => {
    const st = states.get(s.name) || {};
    return {
      id: s.name,
      type: 'dagStep',
      position: { x: 0, y: 0 },
      draggable: false,
      data: {
        label: s.name,
        kindType: (s.kind && s.kind.type) || '',
        status: statuses.get(s.name) || STEP_PENDING,
        output: typeof st.output === 'string' ? st.output : '',
        error: typeof st.error === 'string' ? st.error : '',
        at_ms: Number.isFinite(st.at_ms) ? st.at_ms : null,
      },
    };
  });
  return { nodes: layoutGraph(nodes, edges, opts), edges };
}

/// runStatusTag(status) → antd Tag color token for a DagRunView.status.
export function runStatusTag(status) {
  const map = {
    pending: 'default',
    running: 'processing',
    cancelling: 'orange',
    done: 'success',
    error: 'red',
    cancelled: 'grey',
  };
  return map[String(status || '')] || 'default';
}

/// runStatusLabel(status) → Chinese label for the same status vocabulary.
export function runStatusLabel(status) {
  const map = {
    pending: '排队中',
    running: '运行中',
    cancelling: '取消中',
    done: '已完成',
    error: '失败',
    cancelled: '已取消',
  };
  return map[String(status || '')] || String(status || '-');
}

/// nodeBadgeText(nodeId, emptyText) — 执行节点 badge text: the claiming
/// node id, or the unclaimed hint while the run waits in the queue.
export function nodeBadgeText(nodeId, emptyText = '任意节点排队中') {
  return nodeId === undefined || nodeId === null || nodeId === '' ? emptyText : String(nodeId);
}

/// outputPreview(output, max) — pre-wrap feed/step-card snapshot text. The
/// server snapshot may already carry a truncation marker; long bodies are
/// additionally clipped client-side with an explicit 截断 marker.
export function outputPreview(output, max = 600) {
  if (typeof output !== 'string' || !output) {
    return '';
  }
  if (output.length <= max) {
    return output;
  }
  return output.slice(0, max) + '…(已截断)';
}
