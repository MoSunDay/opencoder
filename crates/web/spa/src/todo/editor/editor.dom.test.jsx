// @vitest-environment jsdom
// Editor DOM smoke: the TODO canvas editor mounts the real editor canvas
// (palette + React Flow todo cards + inspector) directly — TodoCanvasEditor
// takes spec/onSpecChange as props, so no network shell and NOTHING is
// mocked here; @xyflow/react mounts unmocked exactly like dag's
// editor.dom.test.jsx proves it can. The required_tool_calls editor (this
// increment's core value) is exercised end-to-end through the canvas.

import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

import '../../test/setup-dom.js';
import { TodoCanvasEditor } from './canvasEditor.jsx';
import { TodoInspector } from './todoInspector.jsx';

const SPEC = {
  schema_version: 1,
  id: 'wf-demo',
  name: 'demo',
  objective: 'ship the demo',
  constraints: ['不得修改 crates/core'],
  todos: [
    {
      id: 't1',
      title: '调研方案',
      agent: 'explore',
      depends_on: [],
      max_attempts: 3,
      requirement_background: '背景',
      instructions: '调研',
      acceptance: { criteria: '输出对比文档' },
    },
    {
      id: 't2',
      title: '落地实现',
      agent: 'build',
      depends_on: ['t1'],
      max_attempts: 2,
      requirement_background: '背景2',
      instructions: '实现',
      acceptance: { criteria: '测试通过' },
    },
  ],
};

const mountEditor = (onSpecChange) =>
  render(
    <TodoCanvasEditor
      spec={SPEC}
      problems={[]}
      positions={{}}
      onSpecChange={onSpecChange}
      onPositionsChange={vi.fn()}
    />,
  );

describe('TodoCanvasEditor 画布模式', () => {
  it('渲染 spec 的 TODO 节点与左侧面板', async () => {
    mountEditor(vi.fn());
    await waitFor(() => expect(document.querySelectorAll('.dag-edit-node')).toHaveLength(2));
    expect(screen.getByText('调研方案')).toBeTruthy();
    expect(screen.getByText('落地实现')).toBeTruthy();
    expect(screen.getByText('依赖: t1')).toBeTruthy();
    expect(document.querySelector('.dag-edit-pal')).toBeTruthy();
  });

  it('面板点击添加 TODO 并上抛三节点 spec', async () => {
    const onSpecChange = vi.fn();
    mountEditor(onSpecChange);
    fireEvent.click(await screen.findByText('TODO 节点')); // palette card
    await waitFor(() => expect(document.querySelectorAll('.dag-edit-node')).toHaveLength(3));
    await waitFor(() => expect(onSpecChange).toHaveBeenCalled());
    const spec = onSpecChange.mock.calls.at(-1)[0];
    expect(spec.todos).toHaveLength(3);
    expect(spec.todos.some((t) => t.id === 'todo' && t.agent === 'act')).toBe(true);
    expect(spec.name).toBe('demo'); // spec 级字段随 emit 透传
  });

  it('选中节点后属性面板编辑标题并上抛', async () => {
    const onSpecChange = vi.fn();
    mountEditor(onSpecChange);
    await waitFor(() => expect(document.querySelectorAll('.dag-edit-node')).toHaveLength(2));
    fireEvent.click(document.querySelector('.dag-edit-node')); // t1 (调研方案)
    const input = await screen.findByDisplayValue('调研方案');
    fireEvent.change(input, { target: { value: '调研方案 v2' } });
    await waitFor(() => expect(onSpecChange).toHaveBeenCalled());
    const spec = onSpecChange.mock.calls.at(-1)[0];
    expect(spec.todos.find((t) => t.id === 't1').title).toBe('调研方案 v2');
    expect(spec.todos.find((t) => t.id === 't2').depends_on).toEqual(['t1']); // 边不变
  });

  it('required_tool_calls：添加/填名/合法 JSON 上抛，非法 JSON 行内报错不上抛', async () => {
    const onSpecChange = vi.fn();
    mountEditor(onSpecChange);
    await waitFor(() => expect(document.querySelectorAll('.dag-edit-node')).toHaveLength(2));
    fireEvent.click(document.querySelector('.dag-edit-node')); // t1
    fireEvent.click(await screen.findByText('+ 添加工具调用'));
    const nameInput = await waitFor(() => {
      const el = document.querySelector('input[placeholder="工具名，如 bash"]');
      expect(el).toBeTruthy();
      return el;
    });
    fireEvent.change(nameInput, { target: { value: 'bash' } });
    await waitFor(() => {
      const t = onSpecChange.mock.calls.at(-1)[0].todos.find((x) => x.id === 't1');
      expect(t.acceptance.required_tool_calls[0].name).toBe('bash');
      expect(t.acceptance.required_tool_calls[0].arguments_contains).toEqual({});
    });
    // 非法 JSON：失焦后行内报错，arguments_contains 不被污染、不再上抛
    const argsArea = document.querySelector('textarea[placeholder="{}"]');
    fireEvent.change(argsArea, { target: { value: '{ nope' } });
    fireEvent.blur(argsArea);
    expect(await screen.findByText(/JSON 解析失败/)).toBeTruthy();
    const callsAfterBad = onSpecChange.mock.calls.length;
    // 合法 JSON 对象：失焦提交
    fireEvent.change(argsArea, { target: { value: '{"cmd":"ls"}' } });
    fireEvent.blur(argsArea);
    await waitFor(() => {
      const t = onSpecChange.mock.calls.at(-1)[0].todos.find((x) => x.id === 't1');
      expect(t.acceptance.required_tool_calls[0].arguments_contains).toEqual({ cmd: 'ls' });
      expect(onSpecChange.mock.calls.length).toBeGreaterThan(callsAfterBad);
    });
  });

  it('未选中时展示 spec 基础信息表单', async () => {
    mountEditor(vi.fn());
    expect(await screen.findByDisplayValue('demo')).toBeTruthy();
    expect(await screen.findByDisplayValue('ship the demo')).toBeTruthy();
    expect(await screen.findByDisplayValue('不得修改 crates/core')).toBeTruthy();
    expect(screen.getByText('+ 添加约束')).toBeTruthy();
  });
});

describe('TodoInspector 单独挂载', () => {
  const mountInspector = (todo, handlers) =>
    render(
      <TodoInspector
        todo={todo}
        allIds={['t2']}
        problemList={[]}
        onChange={vi.fn()}
        onRemove={vi.fn()}
        {...handlers}
      />,
    );

  it('id 失焦时上抛 onRename（重名校验交给父层）', async () => {
    const onRename = vi.fn();
    mountInspector(SPEC.todos[0], { onRename });
    const input = await screen.findByDisplayValue('t1');
    fireEvent.change(input, { target: { value: 'research' } });
    fireEvent.blur(input);
    expect(onRename).toHaveBeenCalledTimes(1);
    expect(onRename).toHaveBeenCalledWith('research');
  });

  it('回车同样触发 onRename，未修改则不触发', async () => {
    const onRename = vi.fn();
    mountInspector(SPEC.todos[0], { onRename });
    const input = await screen.findByDisplayValue('t1');
    fireEvent.change(input, { target: { value: 'plan-b' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onRename).toHaveBeenCalledWith('plan-b');
    const again = screen.getByDisplayValue('t1'); // 草稿回退为已提交 id
    fireEvent.blur(again);
    expect(onRename).toHaveBeenCalledTimes(1);
  });

  it('problemList 非空时渲染校验错误', () => {
    render(
      <TodoInspector
        todo={SPEC.todos[0]}
        allIds={[]}
        problemList={['TODO t1 title 不能为空']}
        onChange={vi.fn()}
        onRename={vi.fn()}
        onRemove={vi.fn()}
      />,
    );
    expect(screen.getByText('TODO t1 title 不能为空')).toBeTruthy();
    expect(screen.getByText('校验未通过')).toBeTruthy();
  });
});
