// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { useJsonQuery } from './query.js';

const { apiGet } = vi.hoisted(() => ({ apiGet: vi.fn() }));
vi.mock('../../api.js', () => ({ apiGet }));
const decode = (value) => {
  if (typeof value?.label !== 'string') throw new Error('invalid response');
  return value.label;
};
function Query({ path }) {
  const state = useJsonQuery(path, decode);
  return <div>{state.loading ? 'loading' : state.error || state.data}</div>;
}
afterEach(() => { cleanup(); apiGet.mockReset(); });

it('aborts replaced requests and ignores late responses even if transport ignores abort', async () => {
  const pending = {};
  apiGet.mockImplementation((path) => new Promise((resolve) => { pending[path] = resolve; }));
  const view = render(<Query path="/first" />);
  const firstSignal = apiGet.mock.calls[0][1].signal;
  view.rerender(<Query path="/second" />);
  expect(firstSignal.aborted).toBe(true);
  pending['/second']({ label: 'second result' });
  await screen.findByText('second result');
  pending['/first']({ label: 'stale result' });
  await waitFor(() => expect(screen.queryByText('stale result')).toBeNull());
  expect(screen.getByText('second result')).toBeTruthy();
});

it('shows malformed successful responses as errors, not empty data', async () => {
  apiGet.mockResolvedValue({});
  render(<Query path="/broken" />);
  await screen.findByText('invalid response');
});

it('aborts pending requests on unmount', () => {
  apiGet.mockImplementation(() => new Promise(() => {}));
  const view = render(<Query path="/pending" />);
  const signal = apiGet.mock.calls[0][1].signal;
  view.unmount();
  expect(signal.aborted).toBe(true);
});
