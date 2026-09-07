// @vitest-environment jsdom
// Editor DOM smoke: DefEditor's 画布 mode mounts the real editor canvas
// (palette + React Flow step cards + inspector), add/edit flows reach
// onSave, and the JSON ↔ 画布 mode switch roundtrips the spec (blocked
// while the text does not parse). DefEditor takes onSave as a prop, so
// NOTHING is mocked here — @xyflow/react mounts unmocked exactly like
// graph.dom.test.jsx proves it can.

import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

import '../../test/setup-dom.js';
import { DefEditor } from '../defEditor.jsx';
import { StepInspector } from './stepInspector.jsx';

const DEF = {
  id: 'dag-etl',
  name: 'etl',
  spec: {
    name: 'etl',
    description: 'demo',
    steps: [
      { name: 'fetch', kind: { type: 'wasm', command: 'tool.wasm' } },
      { name: 'review', depends_on: ['fetch'], kind: { type: 'agent', prompt: 'review the artifacts' } },
    ],
  },
};

const mountEditor = (onSave) =>
  render(<DefEditor open def={DEF} saving={false} onClose={vi.fn()} onSave={onSave} />);

describe('DefEditor 画布模式', () => {
  it('画布模式默认渲染 spec 步骤节点', async () => {
    mountEditor(vi.fn());
    await waitFor(() => expect(document.querySelectorAll('.dag-edit-node')).toHaveLength(2));
    expect(screen.getByText('fetch')).toBeTruthy();
    expect(screen.getByText('review')).toBeTruthy();
    expect(document.querySelectorAll('.dag-edit-node--agent')).toHaveLength(1);
    expect(document.querySelectorAll('.dag-edit-node--wasm')).toHaveLength(1);
  });

  it('节点面板点击添加 Wasm 步骤并保存', async () => {
    const onSave = vi.fn();
    mountEditor(onSave);
    fireEvent.click(await screen.findByText('Wasm 步骤')); // palette card
    await waitFor(() => expect(document.querySelectorAll('.dag-edit-node')).toHaveLength(3));
    // the fresh wasm step ships with empty command — fill it so validation passes
    const label = await screen.findByText('Wasm 命令 (command)');
    const area = label.closest('.ant-form-item').querySelector('input');
    fireEvent.change(area, { target: { value: 'tool2.wasm' } });
    fireEvent.click(screen.getByText('保 存'));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    const spec = onSave.mock.calls[0][0];
    expect(spec.steps).toHaveLength(3);
    const added = spec.steps.find((s) => /^step/.test(s.name) && s.name !== 'fetch');
    expect(added.kind.type).toBe('wasm');
    expect(added.kind.command).toBe('tool2.wasm');
  });

  it('选中节点后属性面板编辑命令并保存', async () => {
    const onSave = vi.fn();
    mountEditor(onSave);
    await waitFor(() => expect(document.querySelectorAll('.dag-edit-node')).toHaveLength(2));
    fireEvent.click(document.querySelector('.dag-edit-node')); // fetch (wasm step)
    const area = await screen.findByDisplayValue('tool.wasm');
    fireEvent.change(area, { target: { value: 'tool.wasm --v2' } });
    fireEvent.click(screen.getByText('保 存'));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave.mock.calls[0][0].steps[0].kind.command).toBe('tool.wasm --v2');
  });

  it('JSON 与画布模式往返无损', async () => {
    const onSave = vi.fn();
    mountEditor(onSave);
    fireEvent.click(await screen.findByText('JSON'));
    const area = screen.getByRole('textbox');
    expect(area.value).toContain('"name": "etl"');
    const edited = JSON.parse(JSON.stringify(DEF.spec));
    edited.name = 'etl2';
    fireEvent.change(area, { target: { value: JSON.stringify(edited, null, 2) } });
    fireEvent.click(screen.getByText('画布'));
    await waitFor(() => expect(document.querySelectorAll('.dag-edit-node')).toHaveLength(2));
    fireEvent.click(screen.getByText('保 存'));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    const spec = onSave.mock.calls[0][0];
    expect(spec.name).toBe('etl2');
    expect(spec.steps).toHaveLength(2);
  });

  it('JSON 解析失败时阻止切回画布并在保存时报错', async () => {
    const onSave = vi.fn();
    mountEditor(onSave);
    fireEvent.click(await screen.findByText('JSON'));
    const area = screen.getByRole('textbox');
    fireEvent.change(area, { target: { value: '{ nope' } });
    fireEvent.click(screen.getByText('画布'));
    expect(screen.getByRole('textbox')).toBeTruthy(); // mode stayed json
    fireEvent.click(screen.getByText('保 存'));
    expect(await screen.findByText(/JSON 解析失败/)).toBeTruthy();
    expect(onSave).not.toHaveBeenCalled();
  });
});

describe('StepInspector how_append', () => {
  const mountInspector = (step, onChange) =>
    render(
      <StepInspector
        step={step}
        allNames={[step.name]}
        problemList={[]}
        onChange={onChange}
        onRename={vi.fn()}
        onRemove={vi.fn()}
      />,
    );

  it('agent 步骤渲染经验追加输入并提交 kind.how_append', async () => {
    const onChange = vi.fn();
    mountInspector({ name: 'review', kind: { type: 'agent', prompt: 'review the artifacts' } }, onChange);
    const label = await screen.findByText('经验追加 (how_append)');
    const area = label.closest('.ant-form-item').querySelector('textarea');
    expect(area).toBeTruthy();
    fireEvent.change(area, { target: { value: 'cache the build dir' } });
    expect(onChange).toHaveBeenCalledTimes(1);
    const next = onChange.mock.calls[0][0];
    expect(next.name).toBe('review');
    expect(next.kind.how_append).toBe('cache the build dir');
  });

  it('wasm 步骤不渲染经验追加输入', () => {
    mountInspector({ name: 'fetch', kind: { type: 'wasm', command: 'tool.wasm' } }, vi.fn());
    expect(screen.queryByText('经验追加 (how_append)')).toBeNull();
  });
});
