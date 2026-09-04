// @vitest-environment jsdom
// Live-graph smoke: RunDetail with a RESOLVED spec mounts the real React
// Flow canvas (dagre-laid nodes, depends_on edges, status classes) and the
// node click opens the output-snapshot side card. Fold input comes through
// the mocked sse.js seam, exactly like dag.dom.test.jsx.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';

const apiGetMock = vi.fn();
const openStreamMock = vi.fn();
vi.mock('../api.js', () => ({ apiGet: (...a) => apiGetMock(...a), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../sse.js', () => ({ openStream: (...a) => openStreamMock(...a) }));

import '../test/setup-dom.js';
import { RunDetail } from './runDetail.jsx';

const DEF = {
  id: 'dag-etl',
  name: 'etl',
  spec: {
    name: 'etl',
    steps: [
      { name: 'fetch', kind: { type: 'python', code: 'print(1)' } },
      { name: 'review', depends_on: ['fetch'], kind: { type: 'agent', prompt: 'r' } },
    ],
  },
};

const RUN = { id: 'run-graph01', dag_id: 'dag-etl', name: 'etl', node_id: null, status: 'running', created_at: 1 };

beforeEach(() => {
  apiGetMock.mockReset().mockImplementation((path) =>
    String(path).startsWith('/api/dag/defs/') ? Promise.resolve(DEF) : Promise.resolve([]),
  );
  openStreamMock.mockReset().mockImplementation(() => ({ abort: vi.fn() }));
});

describe('RunDetail live graph', () => {
  it('renders dagre-laid step nodes with status classes (edges are pure-tested)', async () => {
    render(<RunDetail run={RUN} onNotice={vi.fn()} onClose={vi.fn()} />);
    // jsdom never fires ResizeObserver, so React Flow skips EDGE rendering
    // until nodes measure — nodes themselves do render; the edge list is
    // covered by dagProjection.test.js graphFromSpec.
    await waitFor(() => expect(document.querySelectorAll('.dag-node')).toHaveLength(2));
    expect(document.querySelector('.dag-node--pending')).toBeTruthy();

    // a running fold recolors the projected node
    await act(async () => {
      openStreamMock.mock.calls[0][0].onFrame({
        event: 'step_started',
        data: { seq: 1, kind: 'step_started', step: 'fetch', payload: {}, at_ms: 1 },
      });
    });
    await waitFor(() => expect(document.querySelector('.dag-node--running')).toBeTruthy());
  });

  it('opens the output-snapshot side card on node click', async () => {
    render(<RunDetail run={RUN} onNotice={vi.fn()} onClose={vi.fn()} />);
    await waitFor(() => expect(document.querySelectorAll('.dag-node')).toHaveLength(2));
    await act(async () => {
      openStreamMock.mock.calls[0][0].onFrame({
        event: 'step_done',
        data: { seq: 1, kind: 'step_done', step: 'fetch', payload: { ok: true, output: 'rows=7' }, at_ms: 2 },
      });
    });
    fireEvent.click(document.querySelector('.dag-node')); // fetch node
    expect(await screen.findByText('步骤 · fetch')).toBeTruthy();
    expect(document.querySelector('.dag-node--done')).toBeTruthy();
    // the side card's snapshot (the feed preview shows the same text — scope
    // the assertion to the card)
    const card = screen.getByText('步骤 · fetch').closest('.ant-card');
    expect(card.textContent).toContain('rows=7');
  });
});
