// @vitest-environment jsdom
// runCanvas.dom.test.jsx — TODO 运行画布 DOM 冒烟：真实挂载 React Flow
// （与 todo/editor/editor.dom.test.jsx 同路径，不 mock @xyflow/react），
// 断言运行语义可观测：节点卡片带 data-todo-id/data-status、状态 Tag 中文、
// 尝试次数、点击节点回调、Inspector 的验收/错误呈现。

import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

import '../test/setup-dom.js';
import { TodoRunCanvas, TodoRunInspector } from './runCanvas.jsx';

const SPEC = {
  schema_version: 1,
  id: 'wf-demo',
  name: 'demo',
  objective: 'ship it',
  todos: [
    { id: 't1', title: '调研方案', agent: 'explore', depends_on: [], acceptance: { criteria: '输出对比' } },
    { id: 't2', title: '落地实现', agent: 'act', depends_on: ['t1'], acceptance: { criteria: '测试通过' } },
  ],
};

const STATES = new Map([
  ['t1', { status: 'passed', attempt: 1, activeSessionId: 's1', lastError: '' }],
  ['t2', { status: 'running', attempt: 2, activeSessionId: 's2', lastError: '' }],
]);

const runNode = (todoId) => document.querySelector(`.oc-todo-run-node[data-todo-id="${todoId}"]`);

describe('TodoRunCanvas', () => {
  it('渲染带运行状态的只读节点，点击节点回调 todo id', async () => {
    const onSelect = vi.fn();
    render(<TodoRunCanvas spec={SPEC} states={STATES} selectedId="" onSelect={onSelect} height={320} />);
    await waitFor(() => expect(runNode('t1')).toBeTruthy());
    await waitFor(() => expect(runNode('t2')).toBeTruthy());

    expect(runNode('t1').getAttribute('data-status')).toBe('passed');
    expect(runNode('t2').getAttribute('data-status')).toBe('running');
    expect(runNode('t1').textContent).toContain('已通过');
    expect(runNode('t2').textContent).toContain('运行中');
    expect(runNode('t2').textContent).toContain('尝试 2');
    expect(runNode('t2').textContent).toContain('依赖: t1');

    fireEvent.click(runNode('t2'));
    expect(onSelect).toHaveBeenCalledWith('t2');
  });

  it('spec 缺失或无 todo 时落空态，不渲染画布', () => {
    render(<TodoRunCanvas spec={null} states={new Map()} height={200} />);
    expect(screen.getByText('spec 未加载或无 TODO')).toBeTruthy();
    expect(document.querySelector('.oc-todo-run-node')).toBeNull();
  });
});

describe('TodoRunInspector', () => {
  it('呈现验收标准 / 尝试 / 会话，最近错误以 Alert 展示', () => {
    render(<TodoRunInspector
      todo={SPEC.todos[1]}
      state={{ status: 'needs_revision', attempt: 2, activeSessionId: 'sess-abcdef1234567890', lastError: '参数缺失' }}
    />);
    expect(screen.getByText('落地实现')).toBeTruthy();
    expect(document.body.textContent).toContain('测试通过');
    expect(document.body.textContent).toContain('待修改');
    expect(document.body.textContent).toContain('参数缺失');
  });

  it('required_tool_calls 非空时列出工具名，todo 缺省返回 null', () => {
    render(<TodoRunInspector
      todo={{ ...SPEC.todos[1], acceptance: { criteria: 'c', required_tool_calls: [{ name: 'bash' }, { name: 'edit' }] } }}
      state={{ status: 'accepting', attempt: 1 }}
    />);
    expect(document.body.textContent).toContain('bash, edit');
    const { container } = render(<TodoRunInspector todo={null} state={null} />);
    expect(container.textContent).toBe('');
  });
});
