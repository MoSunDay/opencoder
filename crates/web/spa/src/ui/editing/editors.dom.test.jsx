// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPut: vi.fn() }));
vi.mock('../../api.js', () => api);
import { HarnessManagement } from '../../harness/management.jsx';
import { DefEditor } from '../../dag/defEditor.jsx';
vi.mock('../../dag/editor/canvasEditor.jsx', () => ({ CanvasEditor: () => <div>canvas</div> }));

beforeEach(() => { api.apiGet.mockReset(); api.apiPut.mockReset(); api.apiPut.mockResolvedValue({ revision: 2 }); });

it('Harness notification rerenders preserve fields and selected profile', async () => {
  api.apiGet.mockResolvedValue({ harnesses: [{ name: 'codex', revision: 1, settings: { model: 'initial', envs: {} } }], profiles: [] });
  const view = render(<HarnessManagement onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getByLabelText('模型（--model）').value).toBe('initial'));
  fireEvent.change(screen.getByLabelText('模型（--model）'), { target: { value: 'edited' } });
  view.rerender(<HarnessManagement onNotice={vi.fn()} />);
  await act(async () => {});
  expect(screen.getByLabelText('模型（--model）').value).toBe('edited');
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
