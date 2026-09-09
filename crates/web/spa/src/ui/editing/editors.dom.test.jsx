// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPut: vi.fn() }));
vi.mock('../../api.js', () => api);
import { PromptEditor } from '../../promptEditor.jsx';
import { HarnessManagement } from '../../harness/management.jsx';
import { DefEditor } from '../../dag/defEditor.jsx';
vi.mock('../../dag/editor/canvasEditor.jsx', () => ({ CanvasEditor: () => <div>canvas</div> }));

beforeEach(() => { api.apiGet.mockReset(); api.apiPut.mockReset(); api.apiPut.mockResolvedValue({ revision: 2 }); });

it('Prompt changes survive notification callback changes', async () => {
  api.apiGet.mockImplementation(async (path) => path.endsWith('/meta') ? { meta: { current: 1 } } : { content_b64: btoa('original') });
  const view = render(<PromptEditor resourceName="prompt-a" onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getByLabelText('prompt-soul').value).toBe('original'));
  fireEvent.change(screen.getByLabelText('prompt-soul'), { target: { value: 'my edit' } });
  const calls = api.apiGet.mock.calls.length;
  view.rerender(<PromptEditor resourceName="prompt-a" onNotice={vi.fn()} />);
  await act(async () => {});
  expect(screen.getByLabelText('prompt-soul').value).toBe('my edit');
  expect(api.apiGet).toHaveBeenCalledTimes(calls);
});

it('Prompt read errors cannot become an empty overwrite', async () => {
  api.apiGet.mockImplementation(async (path) => {
    if (path.endsWith('/meta')) return { meta: { current: 1 } };
    throw Object.assign(new Error('read unavailable'), { status: 503 });
  });
  render(<PromptEditor resourceName="prompt-a" onNotice={vi.fn()} />);
  await screen.findByText('读取 prompt 失败: read unavailable');
  const save = [...document.querySelectorAll('button')].find((b) => b.textContent.replace(/\s/g, '') === '保存');
  expect(save.disabled).toBe(true);
  expect(screen.getByLabelText('prompt-soul').disabled).toBe(true);
  expect(api.apiPut).not.toHaveBeenCalled();
});

it('Harness notification rerenders preserve fields and selected profile', async () => {
  api.apiGet.mockResolvedValue({ harnesses: [{ name: 'codex', revision: 1, settings: { model: 'initial', envs: {} } }], profiles: [] });
  const view = render(<HarnessManagement onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getByLabelText('Codex 模型').value).toBe('initial'));
  fireEvent.change(screen.getByLabelText('Codex 模型'), { target: { value: 'edited' } });
  view.rerender(<HarnessManagement onNotice={vi.fn()} />);
  await act(async () => {});
  expect(screen.getByLabelText('Codex 模型').value).toBe('edited');
  expect(api.apiGet).toHaveBeenCalledTimes(1);
});

it('DAG JSON edits survive a fresh object for the same definition', () => {
  const def = { id: 'd1', spec: { name: 'original', steps: [] } };
  const props = { open: true, def, saving: false, onClose: vi.fn(), onSave: vi.fn() };
  const view = render(<DefEditor {...props} />);
  fireEvent.click(screen.getByText('JSON', { exact: true }));
  const input = document.querySelector('textarea');
  fireEvent.change(input, { target: { value: '{ "name": "unsaved", "steps": [] }' } });
  view.rerender(<DefEditor {...props} def={{ ...def, spec: { ...def.spec } }} />);
  expect(document.querySelector('textarea').value).toContain('unsaved');
});
