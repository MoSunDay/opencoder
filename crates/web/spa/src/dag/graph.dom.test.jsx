// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
const get = vi.hoisted(() => vi.fn());
const open = vi.hoisted(() => vi.fn());
vi.mock('../api.js', () => ({ apiGet: get, apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../sse.js', () => ({ openStream: open }));
import '../test/setup-dom.js';
import { RunDetail } from './runDetail.jsx';
import { ExecutionView } from '../fleet/detail.jsx';

const spec = { name: 'etl', steps: [
  { name: 'fetch', kind: { type: 'wasm', command: 'tool.wasm' } },
  { name: 'review', depends_on: ['fetch'], kind: { type: 'agent', prompt: 'r' } },
] };
const run = { id: 'dag-result', kind: 'dag', node_id: 'node-1', status: 'done', created_at: 1 };
let snapshot;
const log = (seq, step, text) => ({ seq, kind: 'step_log', data: { step, payload: { event: 'stdout', data: { text } } } });
beforeEach(() => {
  snapshot = { head_seq: 20, execution_status: 'done', steps: [{ name: 'fetch', status: 'done' }, { name: 'review', status: 'done' }] };
  get.mockReset().mockImplementation(async (path) => {
    if (path.endsWith('/progress')) return snapshot;
    if (path.includes('events-page')) return { events: [log(19, 'fetch', 'fetch output'), log(20, 'review', 'review output')], head_seq: 20, more: false, finished: true };
    if (path === '/api/executions/dag-result') return { execution: run, definition: { spec }, request: { kind: 'dag' }, dag_steps: snapshot };
    return {};
  });
  open.mockReset().mockImplementation(() => ({ abort: vi.fn() }));
});

for (const entry of ['run', 'execution']) describe(`${entry} result entry`, () => {
  it('shows final states immediately, opens the step drawer and loads run logs behind it', async () => {
    render(entry === 'run' ? <RunDetail run={run} onClose={vi.fn()} /> : <ExecutionView executionRef={run} />);
    await waitFor(() => expect(document.querySelectorAll('.dag-node--done')).toHaveLength(2));
    expect(open).not.toHaveBeenCalled();
    expect(document.querySelector('.dag-detail-side')).toBeNull();
    expect(screen.queryByRole('log')).toBeNull();
    expect(get.mock.calls.some(([path]) => path.includes('events-page'))).toBe(false);
    // Clicking a step opens its node-side record drawer (StepDrawer), not the
    // run-wide logs; the step event stream subscribes at the step path.
    fireEvent.click(document.querySelector('[data-id="fetch"] .dag-node'));
    const drawer = await screen.findByRole('dialog');
    expect(within(drawer).getByText('步骤 fetch')).toBeTruthy();
    await waitFor(() => expect(open.mock.calls.some(([opts]) => opts.path === '/api/dag/runs/dag-result/steps/fetch/events')).toBe(true));
    // The run-wide logs stay one click away behind 运行日志 (same 75vw right drawer).
    fireEvent.click(within(drawer).getByRole('button', { name: '运行日志' }));
    const logs = await waitFor(() => {
      const dialogs = screen.getAllByRole('dialog');
      const target = dialogs.find((dialog) => within(dialog).queryByText('实时日志'));
      expect(target).toBeTruthy();
      return target;
    });
    for (const root of document.querySelectorAll('.dag-logs-drawer')) {
      expect(root.classList.contains('ant-drawer-right')).toBe(true);
      expect(root.querySelector('.ant-drawer-content-wrapper').style.width).toBe('75vw');
    }
    expect(get.mock.calls.some(([path]) => path.includes('events-page'))).toBe(true);
    await waitFor(() => expect(within(logs).getByRole('log').textContent).toContain('fetch output'));
    expect(within(logs).getByRole('log').textContent).not.toContain('review output');
    fireEvent.mouseDown(within(logs).getByRole('combobox', { name: '切换步骤日志' }));
    fireEvent.click(await screen.findByText('review', { selector: '.ant-select-item-option-content' }));
    expect(within(logs).getByRole('log').textContent).toContain('review output');
    expect(within(logs).getByRole('log').textContent).not.toContain('fetch output');
    fireEvent.click(within(logs).getByRole('button', { name: 'Close' }));
    // Only the run-logs drawer closes; the step drawer stays open.
    await waitFor(() => expect(screen.getAllByRole('dialog')).toHaveLength(1));
  });
});

it('shows steps that never started as unexecuted after cancellation', async () => {
  snapshot = { head_seq: 20, execution_status: 'cancelled', steps: [{ name: 'fetch', status: 'cancelled' }, { name: 'review', status: 'pending' }] };
  render(<RunDetail run={run} />);
  await waitFor(() => expect(document.querySelector('.dag-node--skipped')).toBeTruthy());
  expect(document.querySelector('.dag-node--skipped').textContent).toContain('未执行');
  expect(document.querySelector('.dag-node--pending')).toBeNull();
});

it('subscribes strictly above the current state, folds new steps and finalizes once', async () => {
  snapshot = { head_seq: 20, execution_status: 'running', steps: [{ name: 'fetch', status: 'done', at_ms: 10 }, { name: 'review', status: 'running', at_ms: 11 }] };
  const finished = vi.fn();
  render(<RunDetail run={{ ...run, status: 'running' }} onFinished={finished} />);
  await waitFor(() => expect(open).toHaveBeenCalledTimes(1));
  expect(open.mock.calls[0][0].after).toBe(20);
  expect(document.querySelectorAll('.dag-node--running')).toHaveLength(1);
  await act(async () => open.mock.calls[0][0].onFrame({ seq: 21, event: 'step_done', data: { step: 'review', at_ms: 12, payload: { ok: false, error: 'refused' } } }));
  expect(document.querySelector('.dag-node--error').textContent).toContain('refused');
  snapshot = { ...snapshot, head_seq: 22, execution_status: 'error', steps: [{ name: 'fetch', status: 'done' }, { name: 'review', status: 'error', error: 'refused' }] };
  await act(async () => open.mock.calls[0][0].onFrame({ seq: 22, event: 'run_finished', data: { at_ms: 13, payload: { status: 'error', error: 'run failed' } } }));
  expect(finished).toHaveBeenCalledOnce();
  expect(screen.getByText('run failed')).toBeTruthy();
  expect(open.mock.results[0].value.abort).toHaveBeenCalled();
});
