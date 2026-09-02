// @vitest-environment jsdom
// QueuePanel DOM smoke: pending steer + queue rows render from the two
// inputs endpoints, 删除 hits DELETE /inputs/:seq, the queue-only ↑/↓ hit
// POST /inputs/reorder with the {a,b} seq pair. api.js is module-mocked
// (same pattern as sse.test.js) — no signing, no network.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiDelMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('./api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiDel: apiDelMock,
}));

import './test/setup-dom.js';
import { QueuePanel, rowsFromInputs } from './queuePanel.jsx';

const steerFixture = {
  inputs: [
    { seq: 1, delivery: 'steer', prompt: 'steer one', admitted_seq: 1, promoted_seq: null },
    { seq: 2, delivery: 'steer', prompt: 'steer two', admitted_seq: 2, promoted_seq: null },
  ],
};
const queueFixture = {
  inputs: [
    { seq: 5, delivery: 'queue', prompt: 'queue A', admitted_seq: 3, promoted_seq: null },
    { seq: 6, delivery: 'queue', prompt: 'queue B', admitted_seq: 4, promoted_seq: null },
  ],
};

const installApi = () => {
  apiGetMock.mockReset();
  apiPostMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
  apiGetMock.mockImplementation((path) => {
    if (String(path).includes('delivery=queue')) {
      return Promise.resolve(queueFixture);
    }
    return Promise.resolve(steerFixture);
  });
};

beforeEach(installApi);

afterEach(() => {
  cleanup();
});

describe('rowsFromInputs', () => {
  it('normalizes inputs and tolerates garbage', () => {
    expect(rowsFromInputs(undefined)).toEqual([]);
    const rows = rowsFromInputs([
      { seq: 3, delivery: 'queue', prompt: 'b' },
      { seq: 1, delivery: 'steer', prompt: 'a' },
      { seq: 'x', delivery: 'steer', prompt: 'bad' },
      null,
    ]);
    expect(rows).toEqual([
      { seq: 3, delivery: 'queue', prompt: 'b' },
      { seq: 1, delivery: 'steer', prompt: 'a' },
    ]);
  });
});

describe('QueuePanel', () => {
  it('renders nothing without a session', async () => {
    const { container } = render(<QueuePanel sessionId={null} refreshSignal={0} />);
    await act(async () => {});
    expect(container.textContent).toBe('');
    expect(apiGetMock).not.toHaveBeenCalled();
  });

  it('fetches both deliveries and renders one row per pending input', async () => {
    render(<QueuePanel sessionId="s1" refreshSignal={0} />);
    await waitFor(() => {
      expect(screen.getByText('steer one')).toBeTruthy();
    });
    expect(screen.getByText('queue A')).toBeTruthy();
    expect(screen.getByText('steer two')).toBeTruthy();
    expect(screen.getByText('queue B')).toBeTruthy();
    const paths = apiGetMock.mock.calls.map((c) => c[0]);
    expect(paths).toContain('/api/sessions/s1/inputs?delivery=steer');
    expect(paths).toContain('/api/sessions/s1/inputs?delivery=queue');
    // steer rows get no arrows; queue rows get both.
    expect(screen.getAllByLabelText('上移')).toHaveLength(2);
    expect(screen.getAllByLabelText('下移')).toHaveLength(2);
  });

  it('deletes through DELETE /api/sessions/:id/inputs/:seq and refreshes', async () => {
    render(<QueuePanel sessionId="s1" refreshSignal={0} />);
    await waitFor(() => {
      expect(screen.getByText('steer one')).toBeTruthy();
    });
    const callsAfterLoad = apiGetMock.mock.calls.length;
    fireEvent.click(screen.getAllByLabelText('移除')[0]);
    await waitFor(() => {
      expect(apiDelMock).toHaveBeenCalledWith('/api/sessions/s1/inputs/1');
    });
    await waitFor(() => {
      expect(apiGetMock.mock.calls.length).toBeGreaterThan(callsAfterLoad);
    });
  });

  it('reorders queue rows with the adjacent seq as {a,b}', async () => {
    const { rerender } = render(<QueuePanel sessionId="s1" refreshSignal={0} />);
    await waitFor(() => {
      expect(screen.getByText('queue A')).toBeTruthy();
    });
    // First queue row ↓ swaps with the next: a=5, b=6.
    fireEvent.click(screen.getAllByLabelText('下移')[0]);
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/sessions/s1/inputs/reorder', { a: 5, b: 6 });
    });
    // Second queue row ↑ swaps with the previous: a=6, b=5.
    fireEvent.click(screen.getAllByLabelText('上移')[1]);
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/sessions/s1/inputs/reorder', { a: 6, b: 5 });
    });
    // refreshSignal bump pulls both endpoints again.
    const calls = apiGetMock.mock.calls.length;
    rerender(<QueuePanel sessionId="s1" refreshSignal={1} />);
    await waitFor(() => {
      expect(apiGetMock.mock.calls.length).toBeGreaterThan(calls);
    });
  });

  it('degrades to an empty hint when the endpoints return no inputs', async () => {
    apiGetMock.mockResolvedValue({});
    render(<QueuePanel sessionId="s1" refreshSignal={0} />);
    expect(await screen.findByText('暂无待处理输入')).toBeTruthy();
  });
});
