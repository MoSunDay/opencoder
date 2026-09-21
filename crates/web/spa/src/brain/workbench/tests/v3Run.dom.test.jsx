// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { BrainRunBody } from '../run.jsx';
import { apiGet } from '../../../api.js';
import { openStream } from '../../../sse.js';

vi.mock('../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
vi.mock('../../../sse.js', () => ({ openStream: vi.fn(() => ({ abort() {} })) }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

const view = {
  schema_version: 3,
  objective: '修复并复测',
  input_names: ['repo', 'commit'],
  run: { run_id: 'brain-v3', phase: 'waiting', round: 1, generation: 4, last_event_seq: 8, updated_at: 1, error: null },
  capabilities: [{ capability_id: 'cap-agent', kind: 'agent', target: 'act', version: 'current' }],
  rounds: [{ round: 1, status: 'running', operations: [{ operation_id: 'agent-1', run_id: 'brain-v3', round: 1, capability_id: 'cap-agent', execution_kind: 'agent', execution_id: 'agent-1', status: 'running', source_sequence: null, cancel_requested: false }] }],
};

describe('v3 大脑运行总览', () => {
  it('从未调度进入首轮及下一轮时展开当前轮次，并允许查看历史', async () => {
    let current = { ...view, run: { ...view.run, round: 0 }, rounds: [] };
    apiGet.mockImplementation(async (path) => path.endsWith('/view') ? current : { schema_version: 3, run: current.run, operations: [] });
    render(<BrainRunBody id="brain-v3" />);
    await screen.findByText('尚未产生调度轮次');
    await waitFor(() => expect(openStream).toHaveBeenCalledOnce());
    const frame = openStream.mock.calls[0][0].onFrame;
    current = view;
    await act(async () => frame({ seq: 9, event: 'round_started', data: {} }));
    await screen.findByRole('button', { name: /查看执行 agent-1/ });
    expect(screen.getByRole('button', { name: /第 1 轮/ }).textContent).toContain('执行中');
    await act(async () => openStream.mock.calls[0][0].onStatus('open'));
    expect(screen.getByText('实时连接')).toBeTruthy();
    current = { ...view, run: { ...view.run, round: 2 }, rounds: [view.rounds[0], {
      round: 2, status: 'running', operations: [{ ...view.rounds[0].operations[0], round: 2, operation_id: 'agent-2', execution_id: 'agent-2' }],
    }] };
    await act(async () => frame({ seq: 10, event: 'round_started', data: {} }));
    await screen.findByRole('button', { name: /查看执行 agent-2/ });
    expect(screen.getByRole('button', { name: /第 1 轮/ }).getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByRole('button', { name: /第 2 轮/ }).getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: /第 1 轮/ }));
    fireEvent.click(screen.getByRole('button', { name: /第 1 轮/ }));
    expect(screen.getByRole('button', { name: /查看执行 agent-1/ })).toBeTruthy();
  });

  it('历史 v2 运行不再提供提交输入入口', async () => {
    apiGet.mockResolvedValue({ schema_version: 2, phase: 'completed', objective: '旧运行', input_requests: {
      repo: { name: 'repo', description: 'repository', schema: { type: 'string' }, answered: false },
    } });
    render(<BrainRunBody id="brain-history" />);
    await screen.findByText('历史运行只读');
    expect(screen.queryByRole('button', { name: '提交输入' })).toBeNull();
    expect(screen.queryByRole('textbox')).toBeNull();
  });

  it('恢复运行后不把历史阻塞显示为当前失败，诊断仍可查看', async () => {
    const error = 'model provider returned HTTP 429';
    let current = { ...view, run: { ...view.run, phase: 'blocked', error } };
    apiGet.mockImplementation(async (path) => {
      if (path.endsWith('/view')) return current;
      if (path.includes('/events-page')) return { events: [{ seq: 8, event_type: 'decision_blocked', reason_summary: error }], more: false };
      return { schema_version: 3, run: current.run, operations: current.rounds[0].operations };
    });
    render(<BrainRunBody id="brain-v3" />);
    await screen.findByText('运行阻塞或失败');
    await waitFor(() => expect(openStream).toHaveBeenCalledOnce());
    const frame = openStream.mock.calls[0][0].onFrame;
    // Older persisted runs can still carry the error after successful recovery.
    for (const [phase, event, seq] of [['waiting', 'run_resumed', 9], ['completed', 'run_completed', 10]]) {
      current = { ...current, run: { ...current.run, phase } };
      await act(async () => frame({ seq, event, data: {} }));
      await waitFor(() => expect(screen.queryByText('运行阻塞或失败')).toBeNull());
      expect(screen.getByRole('button', { name: /查看执行 agent-1/ })).toBeTruthy();
    }
    fireEvent.click(screen.getByText('调度诊断与事件'));
    fireEvent.click(await screen.findByRole('button', { name: '从头查看' }));
    await screen.findByText(error);
    expect(screen.getByText('decision_blocked')).toBeTruthy();
  });

  it('只显示摘要画布和轮次索引，点击 execution ID 打开托管执行面板', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/brain/runs/brain-v3') return { schema_version: 3, run: view.run, operations: view.rounds[0].operations };
      if (path === '/api/brain/runs/brain-v3/view') return view;
      if (path.startsWith('/api/brain/runs/brain-v3/events-page')) return { events: [], more: false };
      if (path === '/api/executions/agent-1') return { execution: { id: 'agent-1', kind: 'agent', status: 'running', node_id: 'node-1', created_at: 1 }, request: { kind: 'agent', target: 'act', input: {} } };
      if (path.startsWith('/api/executions/agent-1/messages')) return { messages: [], more: false, next_cursor: null };
      return {};
    });
    render(<BrainRunBody id="brain-v3" onNotice={vi.fn()} />);
    expect(await screen.findByRole('heading', { name: '修复并复测' })).toBeTruthy();
    expect(screen.getByLabelText('大脑调度总览画布')).toBeTruthy();
    expect(screen.getAllByText('第 1 轮').length).toBeGreaterThan(0);
    expect(screen.queryByText('步骤执行画布')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /查看执行 agent-1/ }));
    expect(await screen.findByRole('dialog', { name: '能力执行明细' })).toBeTruthy();
    expect(screen.getByText('所属节点')).toBeTruthy();
  });
});
