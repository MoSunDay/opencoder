// runProjection.test.js — pure projection tests for the TODO run canvas
// (items/SSE → view states → spec graph → progress). No React, no jsdom —
// same node-side style as editor/canvasModel.test.js.
import { describe, expect, it } from 'vitest';
import {
  foldTodoEvents,
  itemsToStates,
  runGraph,
  runProgress,
  todoRunClass,
} from './runProjection.js';

const SPEC = {
  schema_version: 1,
  id: 'wf-demo',
  name: 'demo',
  objective: 'ship it',
  todos: [
    { id: 't1', title: '调研', agent: 'explore', depends_on: [], acceptance: { criteria: 'a' } },
    { id: 't2', title: '实现', agent: 'act', depends_on: ['t1'], acceptance: { criteria: 'b' } },
    { id: 't3', title: '验收', agent: 'act', depends_on: ['t2', 't2', 't1', 'ghost', 't3'] },
  ],
};

const ITEMS = [
  { todo_id: 't1', status: 'passed', attempt: 1, active_session_id: 's1', last_error: '' },
  { todo_id: 't2', status: 'running', attempt: 2, active_session_id: 's2', last_error: '' },
];

describe('itemsToStates', () => {
  it('折叠 TodoItemRecord 字段并忽略垃圾行', () => {
    const m = itemsToStates([...ITEMS, null, { status: 'running' }]);
    expect(m.size).toBe(2);
    expect(m.get('t1')).toEqual({ status: 'passed', attempt: 1, activeSessionId: 's1', lastError: '' });
    expect(m.get('t2').attempt).toBe(2);
    expect(m.get('t2').activeSessionId).toBe('s2');
  });

  it('未知状态词兜底为 pending，缺字段兜底为 0/null', () => {
    const m = itemsToStates([{ todo_id: 't9', status: 'nope', attempt: 'x' }]);
    expect(m.get('t9')).toEqual({ status: 'pending', attempt: 0, activeSessionId: null, lastError: '' });
  });

  it('非数组入参返回空 Map', () => {
    expect(itemsToStates(null).size).toBe(0);
  });
});

describe('foldTodoEvents', () => {
  it('todo_* kind 按词表折状态，未知 todo_id 建档', () => {
    const base = itemsToStates(ITEMS);
    const folded = foldTodoEvents(base, [
      { event: 'todo_acceptance_started', data: { todo_id: 't2' } },
      { event: 'todo_accepted', data: { todo_id: 't2', reason: 'ok' } },
      { event: 'todo_candidate_ready', data: { todo_id: 't3' } },
    ]);
    expect(folded.get('t2').status).toBe('passed');
    expect(folded.get('t3').status).toBe('candidate_ready');
    expect(folded.get('t3').attempt).toBe(0); // 新建档案，attempt 未知记 0
    expect(folded.get('t1').status).toBe('passed');
  });

  it('execution_failed：interrupted → interrupted，否则保守 needs_revision', () => {
    const base = itemsToStates([{ todo_id: 't1', status: 'running', attempt: 1 }]);
    expect(foldTodoEvents(base, [{ event: 'todo_execution_failed', data: { todo_id: 't1', interrupted: true } }]).get('t1').status)
      .toBe('interrupted');
    expect(foldTodoEvents(base, [{ event: 'todo_execution_failed', data: { todo_id: 't1' } }]).get('t1').status)
      .toBe('needs_revision');
  });

  it('工作流级 kind 与缺 todo_id 的帧不动状态；回放幂等', () => {
    const base = itemsToStates(ITEMS);
    const frames = [
      { event: 'workflow_rewound', data: { milestone_todo_id: 't1' } },
      { event: 'todo_failed', data: {} },
      { event: 'todo_revision_requested', data: { todo_id: 't2', reason: 'r' } },
    ];
    const once = foldTodoEvents(base, frames);
    expect(once.get('t2').status).toBe('needs_revision');
    expect(foldTodoEvents(once, frames).get('t2').status).toBe('needs_revision');
    expect(once.get('t1').status).toBe('passed');
  });
});

describe('runProgress', () => {
  it('按状态归类计数，未知/缺档记 pending', () => {
    const states = itemsToStates(ITEMS);
    const p = runProgress(SPEC, states);
    expect(p).toEqual({ total: 3, passed: 1, failed: 0, active: 1, pending: 1 });
  });

  it('空 spec 归零', () => {
    expect(runProgress(null, itemsToStates(ITEMS))).toEqual({ total: 0, passed: 0, failed: 0, active: 0, pending: 0 });
  });
});

describe('todoRunClass', () => {
  it('执行中一族共用 running 修饰，终态各有其色', () => {
    expect(todoRunClass('running')).toBe('dag-node--running');
    expect(todoRunClass('accepting')).toBe('dag-node--running');
    expect(todoRunClass('recovering')).toBe('dag-node--running');
    expect(todoRunClass('candidate_ready')).toBe('oc-todo-run--candidate');
    expect(todoRunClass('needs_revision')).toBe('oc-todo-run--revise');
    expect(todoRunClass('passed')).toBe('dag-node--done');
    expect(todoRunClass('failed')).toBe('dag-node--error');
    expect(todoRunClass('interrupted')).toBe('dag-node--skipped');
    expect(todoRunClass('invalidated')).toBe('dag-node--skipped');
    expect(todoRunClass('pending')).toBe('dag-node--pending');
    expect(todoRunClass('nonsense')).toBe('dag-node--pending');
  });
});

describe('runGraph', () => {
  it('节点按 spec 顺序携带状态与去重依赖，坐标由 dagre 赋值', () => {
    const g = runGraph(SPEC, itemsToStates(ITEMS));
    expect(g.nodes.map((n) => n.id)).toEqual(['t1', 't2', 't3']);
    expect(g.nodes[0].data.status).toBe('passed');
    expect(g.nodes[1].data.attempt).toBe(2);
    expect(g.nodes[2].data.depNames).toEqual(['t2', 't1']);
    for (const n of g.nodes) {
      expect(Number.isFinite(n.position.x)).toBe(true);
      expect(Number.isFinite(n.position.y)).toBe(true);
    }
  });

  it('边丢弃 self/unknown/dup，与 specToCanvas 同规则', () => {
    const g = runGraph(SPEC, new Map());
    expect(g.edges.map((e) => e.id).sort()).toEqual(['e-t1-t2', 'e-t1-t3', 'e-t2-t3']);
  });

  it('缺档 todo 兜底 pending，spec 形状异常不抛', () => {
    const g = runGraph(null, null);
    expect(g.nodes).toEqual([]);
    expect(g.edges).toEqual([]);
    const g2 = runGraph(SPEC, new Map());
    expect(g2.nodes.every((n) => n.data.status === 'pending')).toBe(true);
  });
});
