// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import '../../test/setup-dom.js';
const { get, post } = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn() }));
vi.mock('../../api.js', () => ({ apiGet: get, apiPost: post }));
vi.mock('../../fleet/download.js', () => ({ downloadArtifact: vi.fn() }));
import { useRuns } from './useRuns.js';
import { submitAttempt } from './attempt.js';
import { RunReplay } from './run.jsx';
import { RunEvents } from './events.jsx';

beforeEach(() => { get.mockReset(); post.mockReset(); sessionStorage.clear(); });
describe('project attempt replay', () => {
  it('keeps the same run identity after a lost receipt and suppresses concurrent submissions', async () => {
    post.mockRejectedValueOnce(new Error('connection lost'));
    await expect(submitAttempt('retry', 'execute')).rejects.toThrow('connection lost');
    const first = post.mock.calls[0][1].run_id;
    let complete;
    post.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
    const retry = submitAttempt('retry', 'execute');
    expect(submitAttempt('retry', 'execute')).toBe(retry);
    expect(post.mock.calls[1][1].run_id).toBe(first);
    complete({ run_id: first }); await retry;
    post.mockResolvedValueOnce({});
    await submitAttempt('retry', 'execute');
    expect(post.mock.calls[2][1].run_id).not.toBe(first);
  });
  it('loads older history without losing it during refresh', async () => {
    get.mockResolvedValueOnce({ runs: [{ id: 'new', version: 2 }], next_version: 2 })
      .mockResolvedValueOnce({ runs: [{ id: 'old', version: 1 }], next_version: null })
      .mockResolvedValueOnce({ runs: [{ id: 'new', version: 2, status: 'done' }], next_version: 2 });
    const hook = renderHook(() => useRuns('history', false));
    await waitFor(() => expect(hook.result.current.runs).toHaveLength(1));
    await act(() => hook.result.current.next());
    expect(get.mock.calls[1][0]).toContain('before_version=2');
    await act(() => hook.result.current.refresh());
    expect(hook.result.current.runs.map((run) => run.id)).toEqual(['new', 'old']);
    expect(hook.result.current.more).toBe(false);
  });
  it('ignores a late response from the previously selected TODO and exposes refresh errors', async () => {
    let oldResponse;
    get.mockImplementationOnce(() => new Promise((resolve) => { oldResponse = resolve; }))
      .mockResolvedValueOnce({ runs: [{ id: 'current', version: 1 }] });
    const hook = renderHook(({ id }) => useRuns(id, false), { initialProps: { id: 'old' } });
    hook.rerender({ id: 'current' });
    await waitFor(() => expect(hook.result.current.runs[0]?.id).toBe('current'));
    await act(async () => oldResponse({ runs: [{ id: 'stale', version: 10 }] }));
    expect(hook.result.current.runs[0].id).toBe('current');
    get.mockRejectedValueOnce(new Error('node offline'));
    await act(() => hook.result.current.refresh());
    expect(hook.result.current.error).toContain('node offline');
    expect(hook.result.current.updated).toBeTruthy();
  });
  it('opens the exact session and reads oversized input through the run index', async () => {
    const open = vi.fn();
    const marker = { omitted: true, read_via: 'detail_field', field: 'project.run.prun-test.input_snapshot' };
    get.mockResolvedValue({ encoding: 'utf8-base64', offset: 0, next_offset: 4, total_bytes: 4, eof: true, bytes_b64: btoa('test') });
    render(<RunReplay id="prun-test" onOpen={open} detail={{ run: { version: 1, kind: 'execute', agent: 'act', session_id: 'exact-session', input_snapshot: marker }, retention: 'incomplete_history' }} />);
    fireEvent.click(screen.getByText('打开对应会话'));
    expect(open).toHaveBeenCalledWith('exact-session');
    fireEvent.click(screen.getByText('本次输入与 Agent 版本'));
    fireEvent.click(await screen.findByText('分段查看'));
    await waitFor(() => expect(get.mock.calls[0][0]).toContain('/api/executions/prun-test/detail-field?'));
    expect(screen.getByText('历史记录缺少完整输入或过程归档')).toBeTruthy();
  });
  it('pages through all archived events using stable sequence cursors', async () => {
    get.mockResolvedValueOnce({ events: [{ seq: 1, kind: 'status', data: { text: 'first page' } }], more: true })
      .mockResolvedValueOnce({ events: [{ seq: 2, kind: 'tool_end', data: { omitted: true, read_via: 'event_payload' } }], more: false });
    render(<RunEvents id="prun-events" />);
    await screen.findByText(/first page/);
    fireEvent.click(screen.getByText('下一页事件'));
    await screen.findByText('读取完整事件');
    expect(get.mock.calls[1][0]).toBe('/api/executions/prun-events/events-page?after=1');
    expect(screen.getByText('下一页事件').closest('button').disabled).toBe(true);
  });

});
