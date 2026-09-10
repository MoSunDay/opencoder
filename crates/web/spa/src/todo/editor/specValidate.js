// specValidate.js — PURE client-side validation of a WorkflowSpec JSON
// draft (mirror of crates/todos/src/domain.rs validate_spec rules) before
// it is submitted to the server. Unlike dag/specValidate.js, problems are
// STRUCTURED [{path, message}] so the canvas can pin each one on a node:
// path is 'workflow' for spec-level issues or `todos[<id>]` for a todo
// (that is what drives the editor's red dot). [] means the draft may be
// submitted — the server remains authoritative.

export const BUILTIN_AGENTS = ['act', 'plan', 'explore', 'build'];

/// parseSpecDraft(text) → {spec} on success or {error} with a readable
/// Chinese message (JSON.parse's own message is English/noisy).
export function parseSpecDraft(text) {
  const raw = String(text || '').trim();
  if (!raw) {
    return { error: '请输入工作流 JSON' };
  }
  let v;
  try {
    v = JSON.parse(raw);
  } catch (e) {
    return { error: 'JSON 解析失败: ' + (e && e.message ? e.message : String(e)) };
  }
  if (!v || typeof v !== 'object' || Array.isArray(v)) {
    return { error: 'spec 必须是 JSON 对象' };
  }
  return { spec: v };
}

/// todoIdSet(spec) → Set of the raw string ids of every todo (mirror of
/// the ids HashSet in validate_spec: duplicates collapse silently).
function todoIdSet(spec) {
  const ids = new Set();
  for (const t of spec.todos) {
    if (t && typeof t.id === 'string') {
      ids.add(t.id);
    }
  }
  return ids;
}

/// validateSpec(spec) → structured problem list [{path, message}], in
/// server order: schema_version / id / name / objective shape, non-empty
/// todos, id uniqueness, per-todo required fields, max_attempts, path-safe
/// ids, depends_on references, required_tool_calls payloads, agent hints
/// and dependency cycles. Unknown agent names are NOT rejected here —
/// resolve_agent on the server is authoritative, BUILTIN_AGENTS is only
/// advisory for the editor's dropdown.
export function validateSpec(spec) {
  const problems = [];
  if (!spec || typeof spec !== 'object' || Array.isArray(spec)) {
    return [{ path: 'workflow', message: 'spec 必须是 JSON 对象' }];
  }
  if (spec.schema_version !== 1) {
    problems.push({ path: 'workflow', message: 'schema_version 必须为 1' });
  }
  for (const field of ['id', 'name', 'objective']) {
    if (typeof spec[field] !== 'string' || !spec[field].trim()) {
      problems.push({ path: 'workflow', message: 'workflow ' + field + ' 不能为空' });
    }
  }
  if (!Array.isArray(spec.todos) || spec.todos.length === 0) {
    problems.push({ path: 'workflow', message: 'todos 必须是非空数组' });
    return problems;
  }
  const seen = new Set();
  for (const t of spec.todos) {
    if (t && typeof t.id === 'string') {
      if (seen.has(t.id)) {
        problems.push({ path: 'workflow', message: 'TODO id 重复: ' + t.id });
      }
      seen.add(t.id);
    }
  }
  const ids = todoIdSet(spec);
  for (const t of spec.todos) {
    if (!t || typeof t !== 'object') {
      problems.push({ path: 'workflow', message: 'todos 条目必须是对象' });
      continue;
    }
    // Only a non-empty string id can be pinned on the canvas; every
    // problem of an unpinnable todo falls back to the workflow path.
    const hasId = typeof t.id === 'string' && !!t.id.trim();
    const where = hasId ? 'todos[' + t.id + ']' : 'workflow';
    const label = hasId ? t.id : '(空 id)';
    if (!hasId) {
      problems.push({ path: where, message: 'TODO id 不能为空' });
    }
    for (const field of ['title', 'requirement_background', 'instructions']) {
      if (typeof t[field] !== 'string' || !t[field].trim()) {
        problems.push({ path: where, message: 'TODO ' + label + ' ' + field + ' 不能为空' });
      }
    }
    const acc =
      t.acceptance && typeof t.acceptance === 'object' && !Array.isArray(t.acceptance)
        ? t.acceptance
        : null;
    if (!acc || typeof acc.criteria !== 'string' || !acc.criteria.trim()) {
      problems.push({ path: where, message: 'TODO ' + label + ' acceptance.criteria 不能为空' });
    }
    if (!(Number.isInteger(t.max_attempts) && t.max_attempts > 0)) {
      problems.push({ path: where, message: 'TODO ' + label + ' max_attempts 必须为正整数' });
    }
    // Path safety (mirror of domain.rs): todo ids feed debug file paths
    // (`sessions/todos/<id>/...`), so traversal-shaped ids are rejected up
    // front — note '..' is a plain substring test, not a segment check.
    if (
      typeof t.id === 'string' &&
      (t.id.includes('/') || t.id.includes('\\') || t.id.includes('\0') || t.id.includes('..'))
    ) {
      problems.push({
        path: where,
        message: 'TODO ' + label + ' id 不安全（不能含 / \\ .. 或空字节）',
      });
    }
    if (t.depends_on !== undefined && !Array.isArray(t.depends_on)) {
      problems.push({ path: where, message: 'TODO ' + label + ' depends_on 必须是字符串数组' });
    } else if (Array.isArray(t.depends_on)) {
      for (const dep of t.depends_on) {
        if (dep === t.id) {
          problems.push({ path: where, message: 'TODO ' + label + ' 依赖不能指向自身' });
        } else if (!ids.has(dep)) {
          problems.push({
            path: where,
            message: 'TODO ' + label + ' 依赖了不存在的 TODO: ' + dep,
          });
        }
      }
    }
    if (acc && acc.required_tool_calls !== undefined) {
      if (!Array.isArray(acc.required_tool_calls)) {
        problems.push({ path: where, message: 'TODO ' + label + ' required_tool_calls 必须是数组' });
      } else {
        for (const call of acc.required_tool_calls) {
          const bad =
            !call ||
            typeof call !== 'object' ||
            Array.isArray(call) ||
            typeof call.name !== 'string' ||
            !call.name.trim() ||
            !call.arguments_contains ||
            typeof call.arguments_contains !== 'object' ||
            Array.isArray(call.arguments_contains);
          if (bad) {
            problems.push({
              path: where,
              message:
                'TODO ' + label + ' required_tool_calls 条目非法（name 须非空，arguments_contains 须为对象）',
            });
          }
        }
      }
    }
    if (typeof t.agent !== 'string' || !t.agent.trim()) {
      problems.push({ path: where, message: 'TODO ' + label + ' agent 不能为空' });
    } else if (t.agent === 'workflow') {
      problems.push({ path: where, message: 'TODO ' + label + ' 不能使用 workflow agent' });
    }
  }
  const cycle = findCycle(spec);
  if (cycle) {
    const message = '依赖图存在环（涉及 ' + cycle + '）';
    problems.push({ path: ids.has(cycle) ? 'todos[' + cycle + ']' : 'workflow', message });
  }
  return problems;
}

/// findCycle(spec) → the id of the first todo where the walk detects a
/// back edge, or null. Iterative tri-color DFS with an EXPLICIT stack —
/// a line-by-line mirror of domain.rs reject_cycles: the spec is
/// user-supplied JSON and a long dependency chain must not overflow the
/// recursion stack. Unknown dep ids cannot sit on a cycle (they are
/// reported by validateSpec instead).
export function findCycle(spec) {
  const todos = spec && Array.isArray(spec.todos) ? spec.todos : [];
  const deps = new Map();
  for (const t of todos) {
    if (!t || typeof t.id !== 'string' || deps.has(t.id)) {
      continue;
    }
    deps.set(t.id, (Array.isArray(t.depends_on) ? t.depends_on : []).filter((d) => typeof d === 'string'));
  }
  const visiting = new Set(); // on the current DFS stack
  const done = new Set(); // fully explored, provably acyclic
  for (const root of deps.keys()) {
    if (done.has(root)) {
      continue;
    }
    visiting.add(root);
    const stack = [[root, 0]]; // [id, next child index]
    while (stack.length) {
      const top = stack[stack.length - 1];
      const children = deps.get(top[0]) || [];
      if (top[1] < children.length) {
        const child = children[top[1]];
        top[1] += 1;
        if (done.has(child) || !deps.has(child)) {
          continue;
        }
        if (visiting.has(child)) {
          return child; // back edge: child is already on the stack
        }
        visiting.add(child);
        stack.push([child, 0]);
      } else {
        stack.pop();
        visiting.delete(top[0]);
        done.add(top[0]);
      }
    }
  }
  return null;
}
