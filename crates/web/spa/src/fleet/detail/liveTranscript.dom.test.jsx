// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { act, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useExecutionTranscript } from './liveTranscript.js';
import { apiGet } from '../../api.js';
import { openStream } from '../../sse.js';
vi.mock('../../api.js', () => ({ apiGet: vi.fn() }));
vi.mock('../../sse.js', () => ({ openStream: vi.fn() }));

function Harness({ id = 'a', status = 'running', revision = 0, onError = vi.fn() }) {
  const { state, caughtUp } = useExecutionTranscript({ id, status, revision, enabled: true, onError });
  return <pre>{JSON.stringify({ caughtUp, state })}</pre>;
}
const value = () => JSON.parse(screen.getByText((_, node) => node.tagName === 'PRE').textContent);

describe('execution transcript subscription lifecycle', () => {
  beforeEach(() => { vi.resetAllMocks(); apiGet.mockResolvedValue({ head_seq: 3 }); openStream.mockReturnValue({ abort: vi.fn() }); });
  it('opens an empty DAG transcript through the generic execution API', async () => {
    apiGet.mockResolvedValue({ head_seq: 0, events: [], more: false, finished: true });
    render(<Harness id="dag-browser-artifact" status="done" />);
    await waitFor(() => expect(value().caughtUp).toBe(true));
    expect(apiGet).toHaveBeenCalledWith('/api/executions/dag-browser-artifact/events-page?after=9223372036854775807');
    expect(openStream.mock.calls[0][0].path).toBe('/api/executions/dag-browser-artifact/events');
  });
  it('reports an invalid watermark instead of displaying incomplete replay as current', async () => {
    apiGet.mockResolvedValue({ events: [] });
    const onError = vi.fn();
    render(<Harness onError={onError} />);
    await waitFor(() => expect(onError).toHaveBeenCalledWith('节点未返回有效的事件回放位置'));
    expect(openStream).not.toHaveBeenCalled();
    expect(value().caughtUp).toBe(false);
  });
  it('waits for replay head, then continues from its cursor when the same execution resumes', async () => {
    const view = render(<Harness />);
    await waitFor(() => expect(openStream).toHaveBeenCalledTimes(1));
    const first = openStream.mock.calls[0][0];
    expect(first.executionHistory).toBe(true);
    expect(first.after).toBe(0);
    act(() => {
      first.onFrame({ seq: 1, event: 'text_delta', data: { text: 'first' } });
      first.onFrame({ seq: 2, event: 'done', data: {} });
    });
    expect(value().caughtUp).toBe(false);
    act(() => first.onFrame({ seq: 3, event: 'reasoning_delta', data: { text: 'next thought' } }));
    expect(value().caughtUp).toBe(true);
    view.rerender(<Harness status="idle" />);
    await waitFor(() => expect(openStream).toHaveBeenCalledTimes(2));
    expect(openStream.mock.calls[1][0].after).toBe(3);
    act(() => openStream.mock.calls[1][0].onFrame({ seq: 4, event: 'text_delta', data: { text: 'second' } }));
    expect(value().state.turns.filter((turn) => turn.kind === 'text').map((turn) => turn.text)).toEqual(['first', 'second']);
    expect(await openStream.mock.calls[1][0].onResync()).toBe(4);
    view.rerender(<Harness status="running" revision={1} />);
    await waitFor(() => expect(openStream).toHaveBeenCalledTimes(3));
    expect(openStream.mock.calls[2][0].after).toBe(4);
  });
  it('ignores old execution frames after switching IDs and reports offline errors', async () => {
    const onError = vi.fn();
    const view = render(<Harness onError={onError} />);
    await waitFor(() => expect(openStream).toHaveBeenCalledTimes(1));
    const old = openStream.mock.calls[0][0];
    apiGet.mockRejectedValue(new Error('所属节点离线'));
    view.rerender(<Harness id="b" onError={onError} />);
    act(() => old.onFrame({ seq: 9, event: 'text_delta', data: { text: 'wrong execution' } }));
    await waitFor(() => expect(onError).toHaveBeenCalledWith('所属节点离线'));
    expect(value().state.turns).toEqual([]);
  });
});
