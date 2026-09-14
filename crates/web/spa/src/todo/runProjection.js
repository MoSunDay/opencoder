// runProjection.js — PURE projection for the TODO run canvas. Mirrors
// dagProjection.js: persisted items + live SSE todo_* frames fold into
// per-todo view states, the spec dependency graph projects those states
// onto read-only React Flow nodes (dagre auto-layout reused from the
// editor), and statuses map to dag-node CSS modifiers. No React, no DOM.
//
// Wire shapes: items are store TodoItemRecord (crates/store/src/todo_types.rs),
// frames are sse.js frames {event, data, seq} from /api/todo/workflows/:id/events
// (kinds recorded by crates/todos/src/batch.rs / runner.rs).

import { layoutTodoNodes } from './editor/canvasLayout.js';

/// crates/todos TodoStatus 词汇（types.rs as_str）。
export const TODO_RUN_STATUSES = [
  'pending', 'running', 'candidate_ready', 'accepting', 'needs_revision',
  'passed', 'interrupted', 'invalidated', 'recovering', 'failed',
];

/// 折叠时视为「执行中」的状态（进度统计 active 计数用）。
const ACTIVE_STATUSES = ['running', 'candidate_ready', 'accepting', 'recovering'];

/// itemsToStates(items) → Map(todo_id → {status, attempt, activeSessionId, lastError}).
/// 每个工作流内 todo_id 唯一，后来者覆盖即可。
export function itemsToStates(items) {
  const map = new Map();
  for (const it of Array.isArray(items) ? items : []) {
    if (!it || typeof it.todo_id !== 'string') {
      continue;
    }
    map.set(it.todo_id, {
      status: TODO_RUN_STATUSES.includes(it.status) ? it.status : 'pending',
      attempt: Number.isFinite(it.attempt) ? it.attempt : 0,
      activeSessionId: it.active_session_id || null,
      lastError: it.last_error || '',
    });
  }
  return map;
}

/// todo_* 事件 kind → 视图状态。todo_execution_failed 单列：interrupted
/// 跟随 payload；非中断时服务端按 attempt/max_attempts 落 needs_revision
/// 或 failed —— 纯函数拿不到 spec 上限，保守折成 needs_revision，上限后的
/// failed 由下一次 items 快照（silent reload）纠正。
const EVENT_STATUS = {
  todo_candidate_ready: 'candidate_ready',
  todo_acceptance_started: 'accepting',
  todo_accepted: 'passed',
  todo_revision_requested: 'needs_revision',
  todo_failed: 'failed',
};

/// foldTodoEvents(states, frames) → NEW Map。帧按 seq/到达序回放是幂等的：
/// 同 kind 重复帧折叠出同一终值。未知 todo_id 首次出现也建档（attempt 未知记 0）。
export function foldTodoEvents(states, frames) {
  const map = new Map(states instanceof Map ? states : []);
  for (const f of Array.isArray(frames) ? frames : []) {
    const kind = (f && f.event) || 'message';
    const d = (f && f.data) || {};
    const todoId = typeof d.todo_id === 'string' ? d.todo_id : null;
    if (!todoId) {
      continue;
    }
    let status = EVENT_STATUS[kind];
    if (kind === 'todo_execution_failed') {
      status = d.interrupted === true ? 'interrupted' : 'needs_revision';
    }
    if (!status) {
      continue;
    }
    const cur = map.get(todoId) || { status: 'pending', attempt: 0, activeSessionId: null, lastError: '' };
    map.set(todoId, { ...cur, status });
  }
  return map;
}

/// runProgress(spec, states) → {total, passed, failed, active, pending}。
export function runProgress(spec, states) {
  const todos = specTodos(spec);
  let passed = 0;
  let failed = 0;
  let active = 0;
  for (const t of todos) {
    const st = (states.get(t.id) || {}).status || 'pending';
    if (st === 'passed') {
      passed += 1;
    } else if (st === 'failed') {
      failed += 1;
    } else if (ACTIVE_STATUSES.includes(st)) {
      active += 1;
    }
  }
  return { total: todos.length, passed, failed, active, pending: todos.length - passed - failed - active };
}

/// todoRunClass(status) → dag-node 修饰类（色板复用 DAG 运行节点）。
export function todoRunClass(status) {
  switch (status) {
    case 'running':
    case 'accepting':
    case 'recovering':
      return 'dag-node--running';
    case 'candidate_ready':
      return 'oc-todo-run--candidate';
    case 'needs_revision':
      return 'oc-todo-run--revise';
    case 'passed':
      return 'dag-node--done';
    case 'failed':
      return 'dag-node--error';
    case 'interrupted':
    case 'invalidated':
      return 'dag-node--skipped';
    default:
      return 'dag-node--pending';
  }
}

/// specTodos(spec) → 有合法 id 的 todo 列表（保持 spec 顺序）。
function specTodos(spec) {
  const todos = spec && Array.isArray(spec.todos) ? spec.todos : [];
  return todos.filter((t) => t && typeof t.id === 'string');
}

/// runGraph(spec, states) → {nodes, edges}。specToCanvas 的运行镜像：节点跟
/// spec 顺序，边来自 depends_on（丢弃 self/unknown/dup，与编辑画布同规则），
/// 坐标交给 dagre 自动布局（运行视图无会话态可保）。
export function runGraph(spec, states) {
  const todos = specTodos(spec);
  const statesMap = states instanceof Map ? states : new Map();
  const defined = new Set(todos.map((t) => t.id));
  const nodes = todos.map((t) => {
    const s = statesMap.get(t.id);
    const deps = (Array.isArray(t.depends_on) ? t.depends_on : [])
      .filter((d) => typeof d === 'string' && d !== t.id && defined.has(d));
    return {
      id: t.id,
      type: 'todoRun',
      position: { x: 0, y: 0 },
      data: {
        todo: { ...t },
        status: s ? s.status : 'pending',
        attempt: s ? s.attempt : 0,
        depNames: Array.from(new Set(deps)),
      },
    };
  });
  const seen = new Set();
  const edges = [];
  for (const t of todos) {
    for (const dep of Array.isArray(t.depends_on) ? t.depends_on : []) {
      if (typeof dep !== 'string' || dep === t.id || !defined.has(dep)) {
        continue;
      }
      const key = dep + '\u0000' + t.id;
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      edges.push({ id: 'e-' + dep + '-' + t.id, source: dep, target: t.id });
    }
  }
  return { nodes: layoutTodoNodes(nodes, edges), edges };
}
