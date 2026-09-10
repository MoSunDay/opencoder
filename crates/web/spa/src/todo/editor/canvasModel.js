// canvasModel.js — PURE conversions between a WorkflowSpec JSON draft and
// the TODO editor canvas state (React Flow {nodes, edges}), plus the small
// edit-time predicates (rename/cycle/connect/new-todo). Mirrors
// dag/editor/canvasModel.js with todos/id/title in place of steps/name.
// Cycle edges are KEPT here: the editor must render them so validateSpec
// (specValidate.js) can flag them on the canvas.

/// specToCanvas(spec) → {nodes, edges} for the editor canvas. Nodes follow
/// spec todo order (todos without a string id are skipped); every node
/// carries a private copy of its todo in data.todo. Edges are built from
/// depends_on, dropping only self/unknown/duplicate references — cycles
/// survive on purpose (see header).
export function specToCanvas(spec) {
  const todos = spec && Array.isArray(spec.todos) ? spec.todos : [];
  const named = todos.filter((t) => t && typeof t.id === 'string');
  const defined = new Set(named.map((t) => t.id));
  const nodes = named.map((t) => ({
    id: t.id,
    type: 'todoEdit',
    position: { x: 0, y: 0 },
    data: { todo: { ...t } },
  }));
  const seen = new Set();
  const edges = [];
  for (const t of named) {
    const deps = Array.isArray(t.depends_on) ? t.depends_on : [];
    for (const dep of deps) {
      if (typeof dep !== 'string') {
        continue;
      }
      if (dep === t.id || !defined.has(dep)) {
        continue; // self/unknown deps cannot become canvas edges
      }
      const key = dep + '\u0000' + t.id;
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      edges.push({ id: 'e-' + dep + '-' + t.id, source: dep, target: t.id });
    }
  }
  return { nodes, edges };
}

/// canvasToSpec(canvas, baseSpec) → spec draft. Todos follow canvas node
/// order. depends_on is REBUILT: canvas edges into the node (ordered by
/// the source node's position) come first, then the original depends_on
/// entries that reference non-canvas ids (ghost deps preserved for
/// validation — never silently dropped). baseSpec contributes the
/// spec-level fields; schema_version defaults to 1.
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
  const todos = [];
  for (const n of nodes) {
    const raw = n && n.data && n.data.todo;
    if (!raw || typeof raw !== 'object' || typeof raw.id !== 'string') {
      continue;
    }
    const todo = { ...raw };
    const known = (incoming.get(n.id) || []).slice().sort((a, b) => order.get(a) - order.get(b));
    const deps = known.slice();
    const declared = Array.isArray(raw.depends_on) ? raw.depends_on : [];
    for (const d of declared) {
      if (typeof d !== 'string' || d === n.id || idSet.has(d) || deps.includes(d)) {
        continue;
      }
      deps.push(d); // ghost dep: keep it so validation can complain
    }
    todo.depends_on = deps;
    todos.push(todo);
  }
  const base = baseSpec && typeof baseSpec === 'object' ? baseSpec : {};
  const spec = {
    schema_version: base.schema_version === undefined || base.schema_version === null ? 1 : base.schema_version,
    id: typeof base.id === 'string' ? base.id : '',
    name: typeof base.name === 'string' ? base.name : '',
    objective: typeof base.objective === 'string' ? base.objective : '',
    constraints: Array.isArray(base.constraints) ? base.constraints.slice() : [],
    metadata: base.metadata === undefined || base.metadata === null ? {} : base.metadata,
  };
  spec.todos = todos;
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

/// newTodo(takenIds) → fresh todo object with a unique 'todo-N' id and the
/// editor defaults for every field (validateSpec flags the empty texts).
export function newTodo(takenIds) {
  return {
    id: uniqueSlug('todo', takenIds),
    title: '',
    agent: 'act',
    depends_on: [],
    max_attempts: 3,
    requirement_background: '',
    instructions: '',
    acceptance: { criteria: '' },
  };
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

/// renameTodo(canvas, fromId, toId) → {ok, canvas} with new arrays/objects
/// (the input canvas is never mutated) or {error} with a Chinese message
/// when toId trims to empty or collides with another node. A successful
/// rename syncs node ids, node.data.todo.id, edge endpoints AND the
/// depends_on references inside every todo (ghost references included).
export function renameTodo(canvas, fromId, toId) {
  const nodes = canvas && Array.isArray(canvas.nodes) ? canvas.nodes : [];
  const edges = canvas && Array.isArray(canvas.edges) ? canvas.edges : [];
  const id = typeof toId === 'string' ? toId.trim() : '';
  if (!id) {
    return { error: 'TODO id 不能为空' };
  }
  const taken = new Set();
  for (const n of nodes) {
    if (n && typeof n.id === 'string' && n.id !== fromId) {
      taken.add(n.id);
    }
  }
  if (taken.has(id)) {
    return { error: 'TODO id 已存在: ' + id };
  }
  const nextNodes = nodes.map((n) => {
    if (!n || typeof n.id !== 'string') {
      return n;
    }
    const todo = n.data && n.data.todo && typeof n.data.todo === 'object' ? n.data.todo : null;
    const nextTodo = todo
      ? {
          ...todo,
          id: n.id === fromId ? id : todo.id,
          depends_on: Array.isArray(todo.depends_on)
            ? todo.depends_on.map((d) => (d === fromId ? id : d))
            : todo.depends_on,
        }
      : null;
    return {
      ...n,
      id: n.id === fromId ? id : n.id,
      data: nextTodo ? { ...n.data, todo: nextTodo } : n.data,
    };
  });
  const nextEdges = edges.map((e) => {
    if (!e) {
      return e;
    }
    const source = e.source === fromId ? id : e.source;
    const target = e.target === fromId ? id : e.target;
    return { ...e, id: 'e-' + source + '-' + target, source, target };
  });
  return { ok: true, canvas: { nodes: nextNodes, edges: nextEdges } };
}

const TODO_PATH_RE = /^todos\[(.+)\]$/;

/// specProblemIndex(problems) → Map(todo id → problem messages) for
/// validateSpec() output: only `todos[<id>]` problems are attached, ready
/// to drive the per-node red dot.
export function specProblemIndex(problems) {
  const index = new Map();
  for (const p of Array.isArray(problems) ? problems : []) {
    if (!p || typeof p !== 'object') {
      continue;
    }
    const m = TODO_PATH_RE.exec(typeof p.path === 'string' ? p.path : '');
    if (!m) {
      continue;
    }
    const arr = index.get(m[1]) || [];
    arr.push(p.message);
    index.set(m[1], arr);
  }
  return index;
}

/// specLevelProblems(problems) → the messages whose path is 'workflow' or
/// otherwise not attachable to a todo (shape issues, cycles on unknown
/// ids) — rendered in the toolbar popover instead of on a node.
export function specLevelProblems(problems) {
  return (Array.isArray(problems) ? problems : [])
    .filter(
      (p) =>
        !p || typeof p !== 'object' || !TODO_PATH_RE.test(typeof p.path === 'string' ? p.path : ''),
    )
    .map((p) => (p && typeof p.message === 'string' ? p.message : String(p && p.message)));
}
