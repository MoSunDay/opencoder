// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiPost: vi.fn() }));
vi.mock('../../api.js', () => api);
import { CapabilityLauncher, launchInput } from './launcher.jsx';
beforeEach(() => api.apiPost.mockReset());

it.each(['agent', 'operator', 'dag', 'team', 'todos', 'brain'])('dispatches the concrete %s capability', async (kind) => {
  api.apiPost.mockResolvedValue({ execution_id: `${kind}-accepted` });
  const onCreated = vi.fn();
  render(<CapabilityLauncher capability={{ id: 'registered-card', kind, target: 'employee' }} todoId="todo-1" onCreated={onCreated} prompt="任务" />);
  fireEvent.change(screen.getByLabelText('能力输入参数'), { target: { value: '{"commit":"fixed"}' } });
  fireEvent.click(screen.getByRole('button', { name: /开始执行/ }));
  await waitFor(() => expect(onCreated).toHaveBeenCalledWith(`${kind}-accepted`));
  expect(api.apiPost).toHaveBeenCalledWith('/api/project/todos/todo-1/dispatch', {
    execution_id: expect.stringMatching(new RegExp(`^${kind}-`)), capability_id: 'registered-card', input: { prompt: '任务', commit: 'fixed' },
  });
});

it('retries a lost admission reply with the same execution ID and inputs', async () => {
  api.apiPost.mockRejectedValueOnce(new Error('connection lost')).mockResolvedValue({ execution_id: 'agent-accepted' });
  const onCreated = vi.fn();
  render(<CapabilityLauncher capability={{ id: 'registered-card', kind: 'agent', target: 'employee' }} todoId="todo-1" onCreated={onCreated} prompt="任务" />);
  fireEvent.click(screen.getByRole('button', { name: /开始执行/ }));
  await screen.findByText(/connection lost/);
  expect(screen.getByLabelText('执行任务').disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: /开始执行/ }));
  await waitFor(() => expect(onCreated).toHaveBeenCalled());
  expect(api.apiPost.mock.calls[1]).toEqual(api.apiPost.mock.calls[0]);
});
it.each(['[]', 'null', 'invalid'])('rejects non-object input %s', (raw) => {
  expect(() => launchInput('task', raw)).toThrow();
});
