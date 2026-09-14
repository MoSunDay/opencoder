// @vitest-environment jsdom
import '../test/setup-dom.js';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { AgentHarnessFields } from './agentFields.jsx';
import { apiGet, apiPut } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPut: vi.fn() }));
beforeEach(() => { vi.resetAllMocks(); apiPut.mockResolvedValue({ ok: true }); });

async function pick(label, title) {
  fireEvent.mouseDown(screen.getByLabelText(label));
  const option = await waitFor(() => {
    const hit = [...document.querySelectorAll('.ant-select-item-option')].find((el) => el.title === title);
    expect(hit).toBeTruthy(); return hit;
  });
  fireEvent.click(option);
}

it.each(['act', 'reviewer'])('changes the execution method for %s from its detail', async (name) => {
  const onSaved = vi.fn();
  render(<AgentHarnessFields meta={{ name, harness: 'opencoder' }} onNotice={vi.fn()} onSaved={onSaved} />);
  await pick('agent-default-harness', 'Codex');
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith(`/api/agents/${name}`, { harness: 'codex' }));
  expect(onSaved).toHaveBeenCalledOnce();
  expect(apiGet).not.toHaveBeenCalled();
});

it('binds and clears a named profile without overwriting agent resources', async () => {
  apiGet.mockResolvedValue({ items: [{ name: 'business', revision: 2 }] });
  const props = { onNotice: vi.fn(), onSaved: vi.fn(), meta: { name: 'reviewer', harness: 'codex' } };
  const view = render(<AgentHarnessFields {...props} />);
  await waitFor(() => expect(screen.getByLabelText('agent-harness-profile').disabled).toBe(false));
  await pick('agent-harness-profile', 'business · v2');
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/agents/reviewer', { harness_profile: 'business' }));
  view.rerender(<AgentHarnessFields {...props} meta={{ ...props.meta, harness_profile: 'business' }} />);
  await pick('agent-harness-profile', '默认 Codex 配置');
  await waitFor(() => expect(apiPut).toHaveBeenLastCalledWith('/api/agents/reviewer', { harness_profile: null }));
});

it('reports failed updates and keeps the current selection for retry', async () => {
  apiPut.mockRejectedValue(new Error('save unavailable'));
  const onNotice = vi.fn(); const onSaved = vi.fn();
  render(<AgentHarnessFields meta={{ name: 'act', harness: 'opencoder' }} onNotice={onNotice} onSaved={onSaved} />);
  await pick('agent-default-harness', 'Codex');
  await waitFor(() => expect(onNotice).toHaveBeenCalledWith(expect.objectContaining({ text: expect.stringContaining('save unavailable') })));
  expect(onSaved).not.toHaveBeenCalled();
  expect(screen.getByLabelText('agent-default-harness').closest('.ant-select').textContent).toBe('OpenCoder');
});

it('blocks profile changes after a failed read and retries without changing the stored binding', async () => {
  apiGet.mockRejectedValueOnce(new Error('offline')).mockResolvedValue({ items: [{ name: 'business', revision: 2 }] });
  render(<AgentHarnessFields meta={{ name: 'act', harness: 'codex', harness_profile: 'business' }} onNotice={vi.fn()} onSaved={vi.fn()} />);
  await screen.findByText('读取 Codex 配置档案失败：offline');
  expect(screen.getByLabelText('agent-harness-profile').disabled).toBe(true);
  expect(apiPut).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: /^重\s*试$/ }));
  await screen.findByText('business · v2');
  expect(screen.getByLabelText('agent-harness-profile').disabled).toBe(false);
});
