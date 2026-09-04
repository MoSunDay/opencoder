// @vitest-environment jsdom
// DAG panel DOM smoke: 定义 tab (defs table → dispatch modal → POST), the
// def editor's validation feedback, 运行 tab rows (status tag / 执行节点
// badge / cancel gating), and RunDetail's SSE fold + run_finished handling.
// api.js and sse.js are mocked at the protocol seam exactly like
// queuePanel.dom.test.jsx; the graph itself (React Flow) stays unmounted —
// the def fetch is left pending so the loading branch renders instead.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';

const apiGetMock = vi.fn();
const apiPostMock = vi.fn();
const apiDelMock = vi.fn();
const openStreamMock = vi.fn();
vi.mock('../api.js', () => ({
  apiGet: (...a) => apiGetMock(...a),
  apiPost: (...a) => apiPostMock(...a),
  apiDel: (...a) => apiDelMock(...a),
}));
vi.mock('../sse.js', () => ({
  openStream: (...a) => openStreamMock(...a),
}));

import '../test/setup-dom.js';
import { DefsTab } from './defsTab.jsx';
import { DefEditor } from './defEditor.jsx';
import { RunsTable } from './runsTable.jsx';
import { RunDetail } from './runDetail.jsx';
import { setNodes } from '../store.js';

const DEFS = [
  { id: 'dag-etl', name: 'etl', spec: { name: 'etl', steps: [{ name: 'fetch', kind: { type: 'python', code: 'x' } }, { name: 'review', depends_on: ['fetch'], kind: { type: 'agent', prompt: 'r' } }] }, updated_at: 1700000000000 },
  { id: 'dag-other', name: 'nightly', spec: { name: 'nightly', steps: [{ name: 'only', kind: { type: 'agent', prompt: 'r' } }] }, updated_at: 1700000100000 },
];

const RUNS = [
  { id: 'run-aaaaaaaa1111', dag_id: 'dag-etl', name: 'etl', node_id: 'node-1', status: 'running', created_at: 1700000000000 },
  { id: 'run-bbbbbbbb2222', dag_id: 'dag-other', name: 'nightly', status: 'pending', created_at: 1700000050000 },
  { id: 'run-cccccccc3333', dag_id: 'dag-etl', name: 'etl', status: 'done', finished_at: 1700000090000, created_at: 1700000010000 },
];

/// The api.js seam is mocked, so fixtures ARE the parsed bodies (plain
/// values — not fetch Response shapes).
const jsonResponse = (body) => Promise.resolve(body);

beforeEach(() => {
  apiGetMock.mockReset().mockImplementation((path) => {
    if (/^\/api\/dag\/defs\/[^/]+$/.test(String(path))) {
      return jsonResponse(DEFS[0]); // single def view (RunDetail spec fetch)
    }
    if (String(path).startsWith('/api/dag/defs')) {
      return jsonResponse(DEFS);
    }
    if (String(path).startsWith('/api/dag/runs')) {
      return jsonResponse(RUNS);
    }
    return jsonResponse({});
  });
  apiPostMock.mockReset().mockResolvedValue({ run_id: 'run-new12345678' });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
  openStreamMock.mockReset();
  setNodes([{ id: 'node-1', name: 'worker-a', addr: 'http://x', status: 'idle' }]);
});

describe('DefsTab', () => {
  it('renders the defs table and dispatches to any node by default', async () => {
    const onDispatched = vi.fn();
    render(<DefsTab onNotice={vi.fn()} onDispatched={onDispatched} />);
    expect(await screen.findByText('etl')).toBeTruthy();
    expect(await screen.findByText('nightly')).toBeTruthy();
    // step count column
    expect(screen.getByText('2')).toBeTruthy();

    fireEvent.click(screen.getAllByText('派发')[0]); // row action opens the modal
    expect(await screen.findByText(/选择执行节点/)).toBeTruthy();
    fireEvent.click(await screen.findByText('确认派发'));
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledWith('/api/dag/defs/dag-etl/dispatch', {}));
    expect(onDispatched).toHaveBeenCalledWith('run-new12345678');
  });

  it('dispatch pins a node picked from the fleet snapshot', async () => {
    render(<DefsTab onNotice={vi.fn()} onDispatched={vi.fn()} />);
    fireEvent.click((await screen.findAllByText('派发'))[0]);
    expect(await screen.findByText(/选择执行节点/)).toBeTruthy();
    // open the antd Select and pick the node option
    const selector = await waitFor(() => {
      const el = screen.getByRole('combobox');
      expect(el).toBeTruthy();
      return el;
    });
    fireEvent.mouseDown(selector);
    const opt = await screen.findByText(/worker-a/);
    fireEvent.click(opt);
    fireEvent.click(await screen.findByText('确认派发'));
    await waitFor(() =>
      expect(apiPostMock).toHaveBeenCalledWith('/api/dag/defs/dag-etl/dispatch', { node_id: 'node-1' }),
    );
  });

  it('deletes a def through the confirm popover', async () => {
    render(<DefsTab onNotice={vi.fn()} onDispatched={vi.fn()} />);
    fireEvent.click((await screen.findAllByText('删除'))[0]);
    fireEvent.click(await screen.findByText('删 除')); // popconfirm ok button splits CJK
    await waitFor(() => expect(apiDelMock).toHaveBeenCalledWith('/api/dag/defs/dag-etl'));
  });
});

describe('DefEditor', () => {
  it('surfaces local validation problems and never calls onSave', async () => {
    const onSave = vi.fn();
    render(<DefEditor open def={null} saving={false} onClose={vi.fn()} onSave={onSave} />);
    const area = screen.getByRole('textbox');
    fireEvent.change(area, { target: { value: '{ nope' } });
    fireEvent.click(screen.getByText('保 存'));
    expect(await screen.findByText(/JSON 解析失败/)).toBeTruthy();
    expect(onSave).not.toHaveBeenCalled();

    // a spec-level problem list renders the same way
    fireEvent.change(area, {
      target: { value: JSON.stringify({ name: 'x', steps: [{ name: 'Bad', kind: { type: 'python', code: 'x' } }] }) },
    });
    fireEvent.click(screen.getByText('保 存'));
    expect(await screen.findByText(/steps\[0\]\.name 必须匹配/)).toBeTruthy();
    expect(onSave).not.toHaveBeenCalled();
  });

  it('keeps the drawer open with the server 400 problem list when save rejects', async () => {
    const onSave = vi.fn().mockRejectedValue(
      Object.assign(new Error('HTTP 400'), { status: 400, body: { problems: ['spec.steps 不能为空'] } }),
    );
    render(<DefEditor open def={null} saving={false} onClose={vi.fn()} onSave={onSave} />);
    fireEvent.change(screen.getByRole('textbox'), {
      target: { value: JSON.stringify({ name: 'ok', steps: [{ name: 'a', kind: { type: 'python', code: 'x' } }] }) },
    });
    fireEvent.click(screen.getByText('保 存'));
    expect(await screen.findByText('spec.steps 不能为空')).toBeTruthy();
    expect(onSave).toHaveBeenCalledTimes(1);
  });
});

describe('RunsTable', () => {
  it('renders status tags, short ids and the unclaimed-node hint', async () => {
    render(<RunsTable onNotice={vi.fn()} />);
    expect(await screen.findByText('run-aaaa')).toBeTruthy(); // 8-char short id    expect(screen.getByText('运行中')).toBeTruthy();
    expect(screen.getByText('已完成')).toBeTruthy();
    expect(screen.getByText('任意节点排队中')).toBeTruthy(); // pending, unclaimed
    expect(screen.getByText(/worker-a/)).toBeTruthy(); // claimed → fleet name badge
  });

  it('gates cancel to pending|running|cancelling and posts the cancel call', async () => {
    render(<RunsTable onNotice={vi.fn()} />);
    await screen.findByText('运行中');
    const cancels = screen
      .getAllByText('取消')
      .map((el) => el.closest('button'))
      .filter(Boolean);
    // rows order: running (enabled), pending (enabled), done (disabled)
    expect(cancels[0].disabled).toBe(false);
    expect(cancels[1].disabled).toBe(false);
    expect(cancels[2].disabled).toBe(true);
    fireEvent.click(cancels[0]);
    fireEvent.click(await screen.findByText('取消运行'));
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledWith('/api/dag/runs/run-aaaaaaaa1111/cancel', {}));
  });

  it('opens the detail view from 查看', async () => {
    render(<RunsTable onNotice={vi.fn()} />);
    fireEvent.click((await screen.findAllByText('查看'))[0]);
    expect(await screen.findByText('← 返回运行列表')).toBeTruthy();
  });
});

describe('RunDetail', () => {
  const RUN = { id: 'run-live9999', dag_id: 'dag-etl', name: 'etl', node_id: 'node-1', status: 'running', created_at: 1700000000000 };

  function streamStub() {
    let handle = null;
    openStreamMock.mockImplementation(({ onFrame, onStatus }) => {
      handle = { onFrame, onStatus };
      return { abort: vi.fn() };
    });
    return () => handle;
  }

  const frame = (kind, data) => ({ event: kind, data, seq: data.seq ?? null });

  it('folds the SSE replay into the feed and finalizes on run_finished', async () => {
    const getHandle = streamStub();
    const onFinished = vi.fn();
    render(<RunDetail run={RUN} onNotice={vi.fn()} onClose={vi.fn()} onFinished={onFinished} />);

    await waitFor(() =>
      expect(openStreamMock).toHaveBeenCalledWith(
        expect.objectContaining({ path: '/api/dag/runs/run-live9999/events', after: 0 }),
      ),
    );

    const push = (...frames) =>
      act(async () => {
        for (const f of frames) {
          getHandle().onFrame(f);
        }
      });
    await push(
      frame('run_started', { seq: 1, kind: 'run_started', payload: { node_id: 'node-1' }, at_ms: 1 }),
      frame('step_started', { seq: 2, kind: 'step_started', step: 'fetch', payload: {}, at_ms: 2 }),
      frame('step_done', { seq: 3, kind: 'step_done', step: 'fetch', payload: { ok: true, output: 'rows=42' }, at_ms: 3 }),
      frame('step_done', { seq: 4, kind: 'step_done', step: 'review', payload: { ok: false, error: 'refused', output: '' }, at_ms: 4 }),
    );
    // reverse-chron feed: newest (review failed) above the older fetch output
    const texts = document.body.textContent;
    expect(texts).toContain('步骤完成');
    expect(texts).toContain('rows=42');
    expect(texts).toContain('refused');
    expect(texts.indexOf('refused')).toBeLessThan(texts.indexOf('rows=42'));

    await push(frame('run_finished', { seq: 5, kind: 'run_finished', payload: { status: 'error', error: 'step review failed' }, at_ms: 5 }));
    // final status applied to the header row (feed rows carry 失败 too — at
    // least one tag + the error alert prove the header finalized)
    expect((await screen.findAllByText('失败')).length).toBeGreaterThan(0);
    expect(screen.getByText('step review failed')).toBeTruthy();
    expect(onFinished).toHaveBeenCalledTimes(1);
  });

  it('closes the stream after applying run_finished (no further frames fold)', async () => {
    const abort = vi.fn();
    let onFrame = null;
    openStreamMock.mockImplementation((arg) => {
      onFrame = arg.onFrame;
      return { abort };
    });
    render(<RunDetail run={RUN} onNotice={vi.fn()} onClose={vi.fn()} onFinished={vi.fn()} />);
    await waitFor(() => expect(onFrame).toBeTruthy());
    await act(async () => {
      onFrame(frame('run_finished', { seq: 1, kind: 'run_finished', payload: { status: 'done' }, at_ms: 1 }));
    });
    await waitFor(() => expect(abort).toHaveBeenCalledTimes(1));
  });
});
