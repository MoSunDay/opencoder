// canvasModel.js — PURE conversions between a DagSpec JSON draft and the
// editor canvas state (React Flow {nodes, edges}), plus the small edit-time
// predicates (rename/cycle/connect/new-step). Unlike dagProjection.js's
// graphFromSpec, cycle edges are KEPT here: the editor must render them so
// validateSpec (specValidate.js) can flag them on the canvas.

import { SLUG_RE } from '../specValidate.js';

/// specToCanvas(spec) → {nodes, edges} for the editor canvas. Nodes follow
/// spec step order (steps without a string name are skipped); every node
/// carries a private copy of its step in data.step. Edges are built from
/// depends_on, dropping only self/unknown/duplicate references — cycles
/// survive on purpose (see header).
export function specToCanvas(spec) {
  const steps = spec && Array.isArray(spec.steps) ? spec.steps : [];
  const named = steps.filter((s) => s && typeof s.name === 'string');
  const defined = new Set(named.map((s) => s.name));
  const nodes = named.map((s) => ({
    id: s.name,
    type: 'stepEdit',
    position: { x: 0, y: 0 },
    data: { step: { ...s }, kindType: (s.kind && s.kind.type) || '' },
  }));
  const seen = new Set();
  const edges = [];
  for (const s of named) {
    const deps = Array.isArray(s.depends_on) ? s.depends_on : [];
    for (const dep of deps) {
      if (typeof dep !== 'string') {
        continue;
      }
      if (dep === s.name || !defined.has(dep)) {
        continue; // self/unknown deps cannot become canvas edges
      }
      const key = dep + '\u0000' + s.name;
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      edges.push({ id: 'e-' + dep + '-' + s.name, source: dep, target: s.name });
    }
  }
  return { nodes, edges };
}

/// canvasToSpec(canvas, baseSpec) → spec draft. Steps follow canvas node
/// order. depends_on is REBUILT: canvas edges into the node (ordered by the
/// source node's position) come first, then the original depends_on entries
/// that reference non-canvas ids (ghost deps preserved for validation) —
/// the key is omitted entirely when the result is empty. baseSpec only
/// contributes name/description metadata.
export function canvasToSpec(canvas, baseSpec) {
  const nodes = canvas && Array.isArray(canvas.nodes) ? canvas.nodes : [];
  const edges = canvas && Array.isArray(canvas.edges) ? canvas.edges : [];
  const order = new Map();
  const idSet = new Set();
  nodes.forEach((n, i) => {
    if (n && typeof n.id === 'string') {
      order.set(n.id, i);
      idSet.add(n.id);
    }
  });
  const incoming = new Map(); // target id → deduped source ids
  for (const e of edges) {
    if (!e || !idSet.has(e.source) || !idSet.has(e.target) || e.source === e.target) {
      continue;
    }
    const list = incoming.get(e.target) || [];
    if (!list.includes(e.source)) {
      list.push(e.source);
      incoming.set(e.target, list);
    }
  }
  const steps = [];
  for (const n of nodes) {
    const raw = n && n.data && n.data.step;
    if (!raw || typeof raw !== 'object' || typeof raw.name !== 'string') {
      continue;
    }
    const step = { ...raw };
    const known = (incoming.get(n.id) || []).slice().sort((a, b) => order.get(a) - order.get(b));
    const deps = known.slice();
    const declared = Array.isArray(raw.depends_on) ? raw.depends_on : [];
    for (const d of declared) {
      if (typeof d !== 'string' || d === n.id || idSet.has(d) || deps.includes(d)) {
        continue;
      }
      deps.push(d); // ghost dep: keep it so validation can complain
    }
    if (deps.length) {
      step.depends_on = deps;
    } else {
      delete step.depends_on;
    }
    steps.push(step);
  }
  const spec = { name: baseSpec && typeof baseSpec.name === 'string' ? baseSpec.name : '' };
  if (baseSpec && typeof baseSpec.description === 'string') {
    spec.description = baseSpec.description;
  }
  spec.steps = steps;
  return spec;
}

/// uniqueSlug(base, names) → base when unused, else 'base-2', 'base-3', ...
/// names may be an array or a Set.
export function uniqueSlug(base, names) {
  const taken = names instanceof Set ? names : new Set(Array.isArray(names) ? names : []);
  if (!taken.has(base)) {
    return base;
  }
  let i = 2;
  while (taken.has(base + '-' + i)) {
    i += 1;
  }
  return base + '-' + i;
}

/// newStep(kindType, takenNames) → fresh step object with a unique 'step-N'
/// name and an empty payload of the requested kind (unknown kinds fall back
/// to agent).
export function newStep(kindType, takenNames) {
  const name = uniqueSlug('step', takenNames);
  const kind = kindType === 'wasm' ? { type: 'wasm', command: '' } : { type: 'agent', prompt: '' };
  return { name, kind };
}

/// canConnect(edges, source, target) → null when the connection may be
/// added, else a Chinese reason (self / duplicate / would close a cycle via
/// an iterative DFS from target over the existing adjacency).
export function canConnect(edges, source, target) {
  if (source === target) {
    return '不能连接到自身';
  }
  const list = Array.isArray(edges) ? edges : [];
  const adj = new Map();
  for (const e of list) {
    if (!e || typeof e.source !== 'string' || typeof e.target !== 'string') {
      continue;
    }
    if (e.source === source && e.target === target) {
      return '依赖已存在';
    }
    const arr = adj.get(e.source) || [];
    arr.push(e.target);
    adj.set(e.source, arr);
  }
  const seen = new Set([target]);
  const stack = [target];
  while (stack.length) {
    const cur = stack.pop();
    if (cur === source) {
      return '不能形成循环依赖';
    }
    for (const next of adj.get(cur) || []) {
      if (!seen.has(next)) {
        seen.add(next);
        stack.push(next);
      }
    }
  }
  return null;
}

/// renameStep(name, allNames) → null when the candidate is a valid new
/// name, else a Chinese error. allNames must EXCLUDE the step being renamed.
export function renameStep(name, allNames) {
  if (!SLUG_RE.test(typeof name === 'string' ? name : '')) {
    return '步骤名须为小写 slug（a-z0-9-，≤64 字符）';
  }
  const taken = allNames instanceof Set ? allNames : new Set(Array.isArray(allNames) ? allNames : []);
  if (taken.has(name)) {
    return '步骤名已存在';
  }
  return null;
}

/// changeStepKind(step, nextType) → new step keeping name/timeout_secs with
/// a RESET kind payload (agent → empty prompt, wasm → empty command). Deps
/// live on the canvas edges, so they are intentionally not carried over.
export function changeStepKind(step, nextType) {
  const next = {
    name: step && step.name,
    kind: nextType === 'wasm' ? { type: 'wasm', command: '' } : { type: 'agent', prompt: '' },
  };
  if (step && step.timeout_secs !== undefined && step.timeout_secs !== null) {
    next.timeout_secs = step.timeout_secs;
  }
  return next;
}

const STEP_PREFIX_RE = /^steps\[(\d+)\]/;

/// specProblemIndex(problems, nodes) → Map(node id → problem strings) for
/// validateSpec() output. Only 'steps[N] ...' problems are attached; N
/// indexes into the canvas node array (spec order for specToCanvas nodes).
export function specProblemIndex(problems, nodes) {
  const list = Array.isArray(nodes) ? nodes : [];
  const index = new Map();
  for (const p of Array.isArray(problems) ? problems : []) {
    const text = typeof p === 'string' ? p : String(p);
    const m = STEP_PREFIX_RE.exec(text);
    if (!m) {
      continue;
    }
    const node = list[Number(m[1])];
    if (!node || typeof node.id !== 'string') {
      continue;
    }
    const arr = index.get(node.id) || [];
    arr.push(text);
    index.set(node.id, arr);
  }
  return index;
}

/// specLevelProblems(problems) → the problems WITHOUT a steps[N] prefix
/// (spec-level shape issues, ghost-dep lines, cycles — rendered in the
/// toolbar popover instead of on a node).
export function specLevelProblems(problems) {
  return (Array.isArray(problems) ? problems : [])
    .filter((p) => !STEP_PREFIX_RE.test(typeof p === 'string' ? p : String(p)))
    .map((p) => (typeof p === 'string' ? p : String(p)));
}
