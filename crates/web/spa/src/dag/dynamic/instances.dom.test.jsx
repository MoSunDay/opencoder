// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const get = vi.hoisted(() => vi.fn());
const open = vi.hoisted(() => vi.fn());
vi.mock('../../api.js', () => ({ apiGet: get }));
vi.mock('../../sse.js', () => ({ openStream: open }));
import { Instances } from './instances.jsx';
import { StepPanel } from '../step/stepPanel.jsx';
import { StepDrawer } from '../step/stepDrawer.jsx';

beforeEach(() => { get.mockReset(); open.mockReset().mockImplementation(() => ({ abort: vi.fn() })); });
it('cleans up old instance subscriptions and rejects late frames after selection changes', async () => {
  const { rerender } = render(<StepPanel runId="r" step="process" index={0} kind="wasm" />);
  await waitFor(() => expect(open).toHaveBeenCalledTimes(1));
  const first = open.mock.calls[0][0];
  const abort = open.mock.results[0].value.abort;
  expect(first.path).toBe('/api/dag/runs/r/steps/process/instances/0/events');
  act(() => first.onFrame({seq:1,event:'step_output',data:{text:'first output'}}));
  rerender(<StepPanel runId="r" step="process" index={1} kind="wasm" />);
  await waitFor(() => expect(open).toHaveBeenCalledTimes(2));
  expect(abort).toHaveBeenCalledOnce();
  act(() => {
    first.onFrame({seq:2,event:'step_output',data:{text:'late old output'}});
    open.mock.calls[1][0].onFrame({seq:1,event:'step_output',data:{text:'second output'}});
  });
  expect(screen.getByRole('log').textContent).toContain('second output');
  expect(screen.getByRole('log').textContent).not.toContain('first output');
  expect(screen.getByRole('log').textContent).not.toContain('late old output');
  expect(await open.mock.calls[1][0].onResync()).toBe(1);
});
it('paginates a thousand instances and subscribes only to the selected item', async () => {
  get.mockImplementation(async (path) => {
    if (path.includes('?')) {
      const offset = Number(new URL(path, 'http://test').searchParams.get('offset'));
      return { expanded:true,total:1000,progress:{total:1000,done:4,running:4,error:0,cancelled:0,pending:992},
        instances:Array.from({length:100},(_,i)=>({index:offset+i,status:'running'})) };
    }
    return { kind:'wasm', status:'running', input:['--title','hello world'], started_at_ms:1 };
  });
  render(<Instances runId="r" step="process" />);
  await waitFor(() => expect(open).toHaveBeenCalledTimes(1));
  expect(screen.getByText('4/1000 成功')).toBeTruthy();
  expect(screen.getByText(/hello world/)).toBeTruthy();
  fireEvent.click(screen.getByTitle('2'));
  await waitFor(() => expect(get.mock.calls.some(([p]) => p.includes('offset=100&limit=100'))).toBe(true));
  await waitFor(() => expect(open.mock.calls.at(-1)[0].path).toContain('/instances/100/events'));
  expect(open.mock.results[0].value.abort).toHaveBeenCalledOnce();
});
it('keeps a completed instance replay mounted after its terminal frame', async () => {
  get.mockImplementation(async (path) => path.includes('?')
    ? { expanded:true, total:1, progress:{total:1,done:1}, instances:[{index:0,status:'done'}] }
    : { kind:'wasm', status:'done', input:[], started_at_ms:1 });
  render(<Instances runId="r" step="process" />);
  await waitFor(() => expect(open).toHaveBeenCalledTimes(1));
  act(() => open.mock.calls[0][0].onFrame({seq:2,event:'step_finished',data:{status:'done'}}));
  expect(open).toHaveBeenCalledTimes(1);
  expect(open.mock.results[0].value.abort).not.toHaveBeenCalled();
});

it('does not hide a detail error when the instance list refresh succeeds', async () => {
  get.mockImplementation(async (path) => {
    if (path.includes('?')) return { expanded:true, total:1, instances:[{index:0,status:'running'}] };
    throw new Error('实例详情读取失败');
  });
  render(<Instances runId="r" step="process" />);
  expect(await screen.findByText('实例详情读取失败')).toBeTruthy();
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 3100)); });
  expect(get.mock.calls.filter(([p]) => p.includes('?')).length).toBeGreaterThan(1);
  expect(screen.getByText('实例详情读取失败')).toBeTruthy();
});

it('updates a terminal template receipt when the same run resumes', async () => {
  let receipt = { kind: 'dynamic', status: 'error', error: '旧执行错误', started_at_ms: 1 };
  get.mockImplementation(async (path) => path.endsWith('/steps/process')
    ? receipt : { expanded: false, total: 0, instances: [] });
  render(<StepDrawer runId="r" step="process" specKind="dynamic" onClose={() => {}} onOpenRunLogs={() => {}} />);
  expect(await screen.findByText('旧执行错误')).toBeTruthy();
  receipt = { kind: 'dynamic', status: 'running', started_at_ms: 2 };
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 2100)); });
  expect(screen.queryByText('旧执行错误')).toBeNull();
  expect(screen.getByText('运行中')).toBeTruthy();
});
