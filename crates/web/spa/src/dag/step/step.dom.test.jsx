// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const open = vi.hoisted(() => vi.fn());
const get = vi.hoisted(() => vi.fn());
vi.mock('../../api.js', () => ({ apiGet: get }));
vi.mock('../../sse.js', () => ({ openStream: open }));
import { StepPanel } from './stepPanel.jsx';
import { StepDrawer } from './stepDrawer.jsx';

// Flat node-side wasm capture frame (control SSE frame shape from sse.js).
const out = (seq, stream, text) => ({ seq, event: 'step_output', data: { step: 'build', stream, text, at_ms: seq } });
beforeEach(() => { open.mockReset().mockImplementation(() => ({ abort: vi.fn() })); get.mockReset(); });

it('renders wasm stdout/stderr rows and merges adjacent stdout fragments', async () => {
  render(<StepPanel runId="run1" step="build" kind="wasm" />);
  await waitFor(() => expect(open).toHaveBeenCalledOnce());
  expect(open.mock.calls[0][0]).toMatchObject({
    path: '/api/dag/runs/run1/steps/build/events', after: 0, executionHistory: true, requireEnd: true,
  });
  const stream = open.mock.calls[0][0];
  act(() => {
    stream.onFrame(out(1, 'stdout', 'comp'));
    stream.onFrame(out(2, 'stdout', 'iling crate'));
    stream.onFrame(out(3, 'stderr', 'warning: unused'));
  });
  const log = screen.getByRole('log');
  expect(log.textContent).toContain('[stdout] compiling crate');
  expect(log.textContent).toContain('[stderr] warning: unused');
  expect(log.textContent).not.toContain('[stdout] comp[stdout]');
  // The resync floor is the delivered cursor: reconnects never replay it.
  expect(await stream.onResync()).toBe(3);
});

it('folds agent child-session frames into a TUI say/tool transcript', async () => {
  render(<StepPanel runId="run1" step="review" kind="agent" />);
  await waitFor(() => expect(open).toHaveBeenCalledOnce());
  const stream = open.mock.calls[0][0];
  act(() => {
    stream.onFrame({ seq: 1, event: 'text_delta', data: { text: 'inspecting artifacts' } });
    stream.onFrame({ seq: 2, event: 'tool_start', data: { id: 't1', name: 'bash', input: { command: 'ls' } } });
    stream.onFrame({ seq: 3, event: 'tool_end', data: { id: 't1', name: 'bash', output: 'ok' } });
  });
  // The Say stays visible (body or ladder-header preview), never folded away.
  expect(screen.getAllByText(/inspecting artifacts/).length).toBeGreaterThan(0);
  // Drill the ladder: L0 turn → L1 step → L2 calls aggregate → tool row.
  fireEvent.click(screen.getByText(/1 Step/));
  fireEvent.click(screen.getByText('Step(1)'));
  fireEvent.click(screen.getByText('1 Function call'));
  expect(screen.getByText('🔧 bash')).toBeTruthy();
});

it('shows the receipt kind, status and session in the drawer and opens the run logs', async () => {
  const receipt = {
    run_id: 'run1', execution_status: 'running', name: 'review', kind: 'agent', status: 'running',
    error: null, started_at_ms: 1700000000000, finished_at_ms: null, output: null, head_seq: 5, session_id: 'sess-9',
  };
  // Refetch after the terminal frame loses the receipt status: the drawer
  // must fall back to the stream's step_finished status.
  get.mockResolvedValueOnce(receipt).mockResolvedValueOnce({ ...receipt, status: null });
  const onClose = vi.fn();
  const onOpenRunLogs = vi.fn();
  render(<StepDrawer runId="run1" step="review" specKind="" onClose={onClose} onOpenRunLogs={onOpenRunLogs} />);
  await waitFor(() => expect(get).toHaveBeenCalledWith('/api/dag/runs/run1/steps/review',
    expect.objectContaining({ signal: expect.any(AbortSignal) })));
  await waitFor(() => expect(screen.getByText('Agent 步骤')).toBeTruthy());
  expect(screen.getByText('步骤 review')).toBeTruthy();
  expect(screen.getByText('运行中')).toBeTruthy();
  expect(screen.getByText('sess-9')).toBeTruthy();
  // The terminal stream frame refetches the receipt and backs the status up.
  await waitFor(() => expect(open).toHaveBeenCalledOnce());
  act(() => open.mock.calls[0][0].onFrame({ seq: 9, event: 'step_finished', data: { status: 'done', error: null } }));
  await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(screen.getByText('已完成')).toBeTruthy());
  fireEvent.click(screen.getByRole('button', { name: '运行日志' }));
  expect(onOpenRunLogs).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('button', { name: /^关\s*闭$/ }));
  expect(onClose).toHaveBeenCalledOnce();
});
