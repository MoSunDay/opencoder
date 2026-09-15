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
import { err } from '../notice.js';
import { DefsTab } from './defsTab.jsx';
import { DefEditor } from './defEditor.jsx';
import { RunsTable } from './runsTable.jsx';
import { RunDetail } from './runDetail.jsx';
import { setNodes } from '../store.js';

const DEFS = [
  { id: 'dag-etl', name: 'etl', spec: { name: 'etl', steps: [{ name: 'fetch', kind: { type: 'wasm', command: 'tool.wasm' } }, { name: 'review', depends_on: ['fetch'], kind: { type: 'agent', prompt: 'r' } }] }, updated_at: 1700000000000 },
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
  openStreamMock.mockReset().mockReturnValue({ abort: vi.fn() });
  setNodes([{
    id: 'node-1', name: 'worker-a', online: true, kinds: ['dag'],
    snapshot: { ready: true, active_agent_loops: 0, cpu_capacity: 4 },
  }]);
});

describe('DefsTab', () => {
  it('renders the defs table and dispatches to any node by default', async () => {
    const onDispatched = vi.fn();
    const onNotice = vi.fn();
    apiPostMock.mockRejectedValueOnce(new Error('connection lost')).mockResolvedValueOnce({ run_id: 'run-new12345678' });
    render(<DefsTab onNotice={onNotice} onDispatched={onDispatched} />);
    expect(await screen.findByText('etl')).toBeTruthy();
    expect(await screen.findByText('nightly')).toBeTruthy();
    // step count column
    expect(screen.queryByText('步骤数')).toBeNull();
    expect(screen.queryByText('类型')).toBeNull();

    fireEvent.click(screen.getAllByText('派发')[0]); // row action opens the modal
    expect(await screen.findByText(/整个工作流会在同一个节点完成/)).toBeTruthy();
    fireEvent.click(await screen.findByText('确认派发'));
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledTimes(1));
    expect(onNotice).toHaveBeenLastCalledWith(err(expect.stringContaining('connection lost')));
    await waitFor(() => expect(screen.getByText('确认派发').closest('button').disabled).toBe(false));
    fireEvent.click(screen.getByText('确认派发'));
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledTimes(2));
    expect(apiPostMock.mock.calls[0][1].id).toBe(apiPostMock.mock.calls[1][1].id);
    expect(apiPostMock).toHaveBeenLastCalledWith('/api/dag/defs/dag-etl/dispatch', { id: expect.stringMatching(/^dag-/) });
    expect(onNotice).toHaveBeenLastCalledWith(err(''));
    expect(onDispatched).toHaveBeenCalledWith('run-new12345678');
  });

  it('dispatch pins a node picked from the fleet snapshot', async () => {
    render(<DefsTab onNotice={vi.fn()} onDispatched={vi.fn()} />);
    fireEvent.click((await screen.findAllByText('派发'))[0]);
    expect(await screen.findByText(/整个工作流会在同一个节点完成/)).toBeTruthy();
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
      expect(apiPostMock).toHaveBeenCalledWith('/api/dag/defs/dag-etl/dispatch', { id: expect.stringMatching(/^dag-/), node_id: 'node-1' }),
    );
  });

  it('deletes a def through the confirm popover', async () => {
    render(<DefsTab onNotice={vi.fn()} onDispatched={vi.fn()} />);
    fireEvent.click((await screen.findAllByText('删除'))[0]);
    fireEvent.click(await screen.findByText('删 除')); // popconfirm ok button splits CJK
    await waitFor(() => expect(apiDelMock).toHaveBeenCalledWith('/api/dag/defs/dag-etl'));
  });

  it('marks degraded rows (no spec, error) and disables dispatch/edit but not delete', async () => {
    // legacy python def that the server can no longer decode → DEGRADED row
    apiGetMock.mockImplementation((path) =>
      jsonResponse(String(path).startsWith('/api/dag/defs')
        ? [DEFS[0], { id: 'dag-legacy', name: 'legacy', created_at: 1700000000000, updated_at: 1700000000000, error: 'parse dag spec: 该定义使用已下线的 python 步骤，无法解析' }]
        : {}));
    render(<DefsTab onNotice={vi.fn()} onDispatched={vi.fn()} />);
    expect(await screen.findByText(/定义无法解析：parse dag spec/)).toBeTruthy();
    expect(screen.getByText('legacy')).toBeTruthy();
    // row order mirrors the payload: row 0 healthy, row 1 degraded; antd
    // wraps button labels in <span>, so reach the native button element.
    const rowButtons = (text) => screen.getAllByText(text).map((el) => el.closest('button'));
    expect(rowButtons('派发')[0].disabled).toBe(false);
    expect(rowButtons('派发')[1].disabled).toBe(true);
    expect(rowButtons('编辑')[0].disabled).toBe(false);
    expect(rowButtons('编辑')[1].disabled).toBe(true);
    expect(rowButtons('删除')[1].disabled).toBe(false); // cleanup stays possible
  });
});

describe('DefEditor', () => {
  it('surfaces local validation problems and never calls onSave', async () => {
    const onSave = vi.fn();
    render(<DefEditor open def={null} saving={false} onClose={vi.fn()} onSave={onSave} />);
    fireEvent.click(screen.getByText('JSON')); // default is now the canvas mode
    const area = screen.getByRole('textbox');
    fireEvent.change(area, { target: { value: '{ nope' } });
    fireEvent.click(screen.getByText('保 存'));
    expect(await screen.findByText(/JSON 解析失败/)).toBeTruthy();
    expect(onSave).not.toHaveBeenCalled();

    // a spec-level problem list renders the same way
    fireEvent.change(area, {
      target: { value: JSON.stringify({ name: 'x', steps: [{ name: 'Bad', kind: { type: 'wasm', command: 'tool.wasm' } }] }) },
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
    fireEvent.click(screen.getByText('JSON')); // default is now the canvas mode
    fireEvent.change(screen.getByRole('textbox'), {
      target: { value: JSON.stringify({ name: 'ok', steps: [{ name: 'a', kind: { type: 'wasm', command: 'tool.wasm' } }] }) },
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
  it('reports a missing immutable definition and allows retry', async () => {
    apiGetMock.mockResolvedValue({});
    render(<RunDetail run={RUNS[0]} onClose={vi.fn()} />);
    expect(await screen.findByText('执行缺少工作流定义快照')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /重\s*试/ }));
    await waitFor(() => expect(apiGetMock).toHaveBeenCalledTimes(2));
    expect(openStreamMock).not.toHaveBeenCalled();
  });
});
